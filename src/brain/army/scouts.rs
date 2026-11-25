//! Scouting (SPEC §5.10).
//!
//! Runs at [`crate::brain::Step::Scouts`] when `ctx.due.scouts`
//! (`cadence.scout_ticks`; SPEC §3.1 step 8). Its orders are not
//! micro-limited (the limiter covers the army and air passes only, §2.3).
//!
//! - **Borrowing.** With no scout owned, built or being built, the first
//!   combat unit is lent from the army (`LoanReason::BorrowedScout`) and
//!   mirrored in `world.coord.borrowed_scout`; it goes back once a real
//!   scout appears. On water maps with nothing enemy known, one hull is
//!   lent from the navy (`scout.naval`, `LoanReason::NavalScout`) and goes
//!   back once any enemy is known.
//! - **Each pass** (`scout.enabled`): a scout with orders is left alone. A
//!   hurt scout (below 1/3 health) goes home when more than
//!   [`HURT_HOME_SLACK`] away. Otherwise it gets its legs: while the enemy
//!   base is unknown, the first is the stalest declared start more than
//!   [`OPENING_MIN_DISTANCE`] from home whose cell is staler than
//!   [`OPENING_MIN_STALENESS`]; the rest (planes [`PLANE_LEGS`] in all,
//!   ground and sea scouts [`OTHER_LEGS`]) maximise the influence grid's
//!   scout score from the previous leg, with the accept test:
//!   [`CLAIM_SPACING`] from other scouts' claims and earlier legs, zero
//!   ground threat, and the right terrain and reachability for the scout.
//!
//! Reads `world.own`, `world.grid`, `world.map.starts`, `world.reach`,
//! `world.contacts`. Writes `world.coord.borrowed_scout` and loans in
//! `world.ownership`. No randomness: ties go to grid scan order or the
//! lower id.

use std::collections::BTreeMap;

use crate::brain::mapintel::MapCharacter;
use crate::brain::own::{classify, UnitClass};
use crate::brain::{Ctx, LoanReason, Manager, Source, Staging};
use crate::view::{Command, DefId, Orders, Pos, UnitDef, UnitId, VisCell};

/// §5.10: a hurt scout goes home when further than this.
pub const HURT_HOME_SLACK: i64 = 256;
/// §5.10: the opening leg's start is further than this from home ...
pub const OPENING_MIN_DISTANCE: i64 = 512;
/// ... and its cell is staler than this.
pub const OPENING_MIN_STALENESS: u32 = 20;
/// §5.10: legs per plan for planes and for ground or sea scouts.
pub const PLANE_LEGS: usize = 3;
pub const OTHER_LEGS: usize = 1;
/// §5.10: a leg keeps this far from other scouts' claims and earlier legs.
pub const CLAIM_SPACING: i64 = 320;

/// The scout manager.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoutManager {
    /// Each scout's planned legs, by id: the claims other scouts avoid.
    claims: BTreeMap<UnitId, Vec<Pos>>,
}

/// How a scout gets about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Terrain {
    Air,
    Ground,
    Sea,
}

/// A scout type: the scout roles, or an unarmed non-builder aircraft.
fn is_scout_def(ctx: &Ctx<'_>, def: DefId) -> bool {
    ctx.def(def)
        .is_some_and(|d| classify(def, d, &ctx.world.roles) == UnitClass::Scout)
}

impl ScoutManager {
    /// Lends and returns the borrowed scouts (§5.10).
    fn borrowing(&mut self, ctx: &mut Ctx<'_>) {
        let own = &ctx.world.own;
        let real = own
            .units
            .iter()
            .any(|s| !s.is_frame() && s.class == UnitClass::Scout && !s.unit.scripted);
        let building = own.units.iter().any(|s| {
            (s.is_frame() && is_scout_def(ctx, s.unit.def))
                || s.unit
                    .factory
                    .as_ref()
                    .is_some_and(|f| f.queue.iter().any(|&d| is_scout_def(ctx, d)))
        });
        let enabled = ctx.knobs.scout.enabled != 0;
        let borrowed: Vec<UnitId> = ctx
            .world
            .ownership
            .lent_for(LoanReason::BorrowedScout)
            .collect();
        if real || !enabled {
            for u in borrowed {
                // Only the borrower returns it; an error means it is gone.
                let _ = ctx.world.ownership.give_back(u, Source::Scouts);
            }
        } else if borrowed.is_empty() && !building {
            if let Some(&u) = ctx.world.own.combat.first() {
                let _ = ctx.world.ownership.lend(
                    u,
                    Source::Army,
                    Source::Scouts,
                    LoanReason::BorrowedScout,
                );
            }
        }

        let naval: Vec<UnitId> = ctx
            .world
            .ownership
            .lent_for(LoanReason::NavalScout)
            .collect();
        let wet = ctx.world.map.character != MapCharacter::Land;
        let unknown = ctx.world.contacts.is_empty();
        if !(enabled && ctx.knobs.scout.naval && wet && unknown) {
            for u in naval {
                let _ = ctx.world.ownership.give_back(u, Source::Scouts);
            }
        } else if naval.is_empty() {
            if let Some(&u) = ctx.world.own.hulls.first() {
                let _ = ctx.world.ownership.lend(
                    u,
                    Source::Navy,
                    Source::Scouts,
                    LoanReason::NavalScout,
                );
            }
        }

        let o = &ctx.world.ownership;
        ctx.world.coord.borrowed_scout = o
            .lent_for(LoanReason::BorrowedScout)
            .next()
            .or_else(|| o.lent_for(LoanReason::NavalScout).next());
    }

    /// The legs for one scout (§5.10).
    fn legs(&self, ctx: &Ctx<'_>, u: UnitId, here: Pos, d: &UnitDef, home: Pos) -> Vec<Pos> {
        let terrain = if d.flies {
            Terrain::Air
        } else if d.move_class.min_depth > 0 {
            Terrain::Sea
        } else {
            Terrain::Ground
        };
        let want = if terrain == Terrain::Air {
            PLANE_LEGS
        } else {
            OTHER_LEGS
        };
        let others: Vec<Pos> = self
            .claims
            .iter()
            .filter(|(&id, _)| id != u)
            .flat_map(|(_, legs)| legs.iter().copied())
            .collect();
        let grid = &ctx.world.grid;
        let mut legs: Vec<Pos> = Vec::new();
        if ctx.world.enemy_base().is_none() {
            let opening = ctx
                .world
                .map
                .starts
                .iter()
                .copied()
                .filter(|s| !s.within(home, OPENING_MIN_DISTANCE))
                // A start the scout cannot get to is no leg for it.
                .filter(|&s| fits(ctx, terrain, d, s))
                .map(|s| (grid.staleness_at(s.vis_cell()), s))
                .filter(|&(st, _)| st > OPENING_MIN_STALENESS)
                .enumerate()
                .max_by_key(|&(i, (st, _))| (st, std::cmp::Reverse(i)))
                .map(|(_, (_, s))| s);
            legs.extend(opening);
        }
        let spaced = |c: Pos, legs: &[Pos]| {
            others
                .iter()
                .chain(legs)
                .all(|&o| !c.within(o, CLAIM_SPACING - 1))
        };
        while legs.len() < want {
            let from = legs.last().copied().unwrap_or(here);
            let cell = grid.best_scout_cell(from, |c: VisCell| {
                let p = c.center();
                grid.staleness_at(c) > 0
                    && grid.threat_at(c) == 0
                    && spaced(p, &legs)
                    && fits(ctx, terrain, d, p)
            });
            match cell {
                Some(c) => legs.push(c.center()),
                None => break,
            }
        }
        legs
    }
}

/// The right terrain and reachability for the scout.
fn fits(ctx: &Ctx<'_>, terrain: Terrain, d: &UnitDef, p: Pos) -> bool {
    match terrain {
        Terrain::Air => true,
        Terrain::Ground => {
            ctx.view.standable(p.tile(), d.move_class) && ctx.world.reach.ground_reachable(p)
        }
        Terrain::Sea => {
            ctx.view.water_depth(p.tile()) > 0
                && ctx.view.standable(p.tile(), d.move_class)
                && ctx.world.reach.naval_reachable(p)
        }
    }
}

impl Manager for ScoutManager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        self.borrowing(ctx);
        let scouts: Vec<UnitId> = ctx
            .world
            .own
            .units
            .iter()
            .filter(|s| !s.is_frame() && ctx.world.ownership.is_with(s.id(), Source::Scouts))
            .map(|s| s.id())
            .collect();
        self.claims.retain(|u, _| scouts.contains(u));
        if ctx.knobs.scout.enabled == 0 {
            return;
        }
        let Some(home) = ctx.home() else {
            return;
        };
        for u in scouts {
            if ctx.busy(u) {
                continue;
            }
            let Some(s) = ctx.world.own.get(u) else {
                continue;
            };
            let Some(d) = ctx.def(s.unit.def) else {
                continue;
            };
            let here = s.unit.pos;
            if i64::from(s.unit.health) * 3 < i64::from(d.health) {
                self.claims.remove(&u);
                if !here.within(home, HURT_HOME_SLACK) {
                    out.now(u, Command::Move(home));
                }
                continue;
            }
            let legs = self.legs(ctx, u, here, d, home);
            for (i, &p) in legs.iter().enumerate() {
                if i == 0 {
                    out.now(u, Command::Move(p));
                } else {
                    out.queued(u, Command::Move(p));
                }
            }
            self.claims.insert(u, legs);
        }
    }
}
