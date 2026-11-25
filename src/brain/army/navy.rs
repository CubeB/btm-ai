//! The navy (SPEC §5.12 "Navy", with the submarine hunt of §5.5 and the
//! commander guard of §5.6).
//!
//! Runs at [`crate::brain::Step::Navy`] when `ctx.due.tactics`, right after
//! the army (both are SPEC §3.1 step 10); its orders pass the micro limiter.
//!
//! - **Station:** `navy.rally` out from the first shipyard, on the first of
//!   16 bearings with deep, connected water (at least
//!   [`STATION_DEPTH`] deep at the station, [`ROUTE_DEPTH`] along the
//!   way). With no shipyard the navy freezes (§5.13).
//! - **Sailing:** once `navy.sail_size` hulls are within [`GATHER_RADIUS`]
//!   of the yard; recalled below [`RECALL_BELOW`]. The land attack's
//!   `navy.attack_size` gate is the army's phase machine (§5.1).
//! - **Objective:** the remembered enemy on our water body nearest the
//!   enemy base; else the shipyard site on our water nearest the enemy
//!   base; else the start positions more than [`START_MIN_DISTANCE`] from
//!   home, furthest first, rotating every [`START_ROTATE`].
//! - **Per hull**, first match: commander guard (`world.army
//!   .commander_danger` / `commander_threats`, within `cmd.guard_radius`);
//!   submarines hunting the allowed enemy commander (`world.army
//!   .hunt_target`) on our water while the fleet is out; attacks on seen,
//!   naval-reachable enemies within [`ENGAGE_RADIUS`], standing off when
//!   the hull outranges them (`navy.stand_off`, [`super::micro::stand_off`])
//!   and repositioning after `navy.stall_s` without damage
//!   ([`super::micro::StallClock`]) to [`STALL_BEARINGS`] bearings round
//!   the target at [`STALL_RADII`]; then the objective or the station.
//!
//! Reads `world.own.hulls`, `world.contacts`, `world.reach`, `world.map`,
//! `world.army`. Writes nothing shared. A hull lent as the naval scout is
//! in `world.own.scouts` instead. No randomness: ties go to the lower id.

use std::collections::{BTreeMap, BTreeSet};

use super::geom::{lerp, on_bearing};
use super::micro::{stand_off, StallClock};
use super::{dist, order, secs, COMMANDER_GUARD_SLACK, RALLY_SLACK, SAME_MOVE};
use crate::brain::roles::Role;
use crate::brain::{Ctx, Manager, Staging, Standing};
use crate::view::{Command, Pos, Tick, UnitDef, UnitId, TICKS_PER_SECOND, TILE_SIZE};

/// §5.12: the station's least depth, and the least depth on the way there.
pub const STATION_DEPTH: i32 = 20;
pub const ROUTE_DEPTH: i32 = 12;
/// §5.12: hulls within this of the yard count towards sailing.
pub const GATHER_RADIUS: i64 = 440;
/// §5.12: the fleet is recalled below this many hulls.
pub const RECALL_BELOW: usize = 2;
/// §5.12: start positions closer to home than this are no objective.
pub const START_MIN_DISTANCE: i64 = 900;
/// §5.12: the start objective rotates this often.
pub const START_ROTATE: Tick = 120 * TICKS_PER_SECOND;
/// §5.12: hulls attack seen enemies within this.
pub const ENGAGE_RADIUS: i64 = 450;
/// §5.12: the stand-off margin round both ranges.
pub const STAND_OFF_MARGIN: i64 = 40;
/// §5.12: stall repositioning: bearings round the target, and the radii
/// tried, in order.
pub const STALL_BEARINGS: usize = 8;
pub const STALL_RADII: [i64; 3] = [240, 340, 160];

/// The navy manager.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavyManager {
    /// The yard the station was found for, and the station.
    station: Option<(UnitId, Pos)>,
    /// The fleet is out.
    sailing: bool,
    /// When the start-position rotation began.
    rotation_since: Option<Tick>,
    /// Stall clocks by (hull, target).
    stalls: BTreeMap<(UnitId, UnitId), StallClock>,
    /// Targets a hull gave up on, until a tick.
    ignored: BTreeMap<(UnitId, UnitId), Tick>,
}

impl Manager for NavyManager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        if ctx.step != crate::brain::Step::Navy {
            return;
        }
        let hulls = ctx.world.own.hulls.clone();
        let alive: BTreeSet<UnitId> = hulls.iter().copied().collect();
        let tick = ctx.tick;
        self.stalls
            .retain(|(u, t), _| alive.contains(u) && ctx.world.contacts.get(*t).is_some());
        self.ignored.retain(|(u, t), until| {
            tick < *until && alive.contains(u) && ctx.world.contacts.get(*t).is_some()
        });
        let Some((yard_id, yard)) = first_yard(ctx) else {
            // No shipyard: the navy freezes (§5.13).
            self.station = None;
            self.sailing = false;
            return;
        };
        if self.station.is_none_or(|(y, _)| y != yard_id) {
            self.station = Some((yard_id, station(ctx, yard)));
        }
        let station = self.station.map_or(yard, |(_, p)| p);

        // Sailing and recall.
        let gathered = hulls
            .iter()
            .filter_map(|&u| ctx.world.own.get(u))
            .filter(|s| s.unit.pos.within(yard, GATHER_RADIUS))
            .count();
        let sail = usize::try_from(ctx.knobs.navy.sail_size.max(1)).unwrap_or(usize::MAX);
        if !self.sailing && gathered >= sail {
            self.sailing = true;
        } else if self.sailing && hulls.len() < RECALL_BELOW {
            self.sailing = false;
        }
        let objective = if self.sailing {
            self.objective(ctx, station)
        } else {
            None
        };

        for u in hulls {
            if let Some(want) = self.duty(ctx, u, station, objective) {
                order(ctx, out, u, want);
            }
        }
    }
}

/// Our first shipyard (id order).
fn first_yard(ctx: &Ctx<'_>) -> Option<(UnitId, Pos)> {
    let w = &ctx.world;
    w.own
        .factories
        .iter()
        .filter_map(|&f| w.own.get(f))
        .find(|s| {
            w.roles.is(s.unit.def, Role::Shipyard) || w.roles.is(s.unit.def, Role::AdvancedShipyard)
        })
        .map(|s| (s.id(), s.unit.pos))
}

fn depth(ctx: &Ctx<'_>, p: Pos) -> i32 {
    ctx.view.water_depth(p.tile())
}

/// The fleet station (§5.12): the first of 16 bearings with deep,
/// connected water; the yard itself when none qualifies.
fn station(ctx: &Ctx<'_>, yard: Pos) -> Pos {
    let r = dist(ctx.knobs.navy.rally);
    let n = (r / i64::from(TILE_SIZE)).max(1);
    (0..16)
        .map(|k| on_bearing(yard, k, r))
        .find(|&p| {
            ctx.view.map().contains(p)
                && depth(ctx, p) >= STATION_DEPTH
                && ctx.world.reach.naval_reachable(p)
                && (0..=n).all(|i| depth(ctx, lerp(yard, p, i, n)) >= ROUTE_DEPTH)
        })
        .unwrap_or(yard)
}

/// Whether a point is on the fleet's water body.
fn our_water(ctx: &Ctx<'_>, body: Option<u32>, p: Pos) -> bool {
    body.is_some() && ctx.world.map.water_body(p.tile()) == body
}

impl NavyManager {
    /// The fleet objective (§5.12), first available.
    fn objective(&mut self, ctx: &Ctx<'_>, station: Pos) -> Option<Pos> {
        let w = &ctx.world;
        let body = w.map.water_body(station.tile());
        let base = w.enemy_base();
        let anchor = base.unwrap_or(station);
        let enemy = w
            .contacts
            .iter()
            .filter(|c| our_water(ctx, body, c.pos) && ctx.standing(c.id).is_some())
            .min_by_key(|c| (c.pos.dist2(anchor), c.id))
            .map(|c| c.pos);
        if enemy.is_some() {
            self.rotation_since = None;
            return enemy;
        }
        if let Some(b) = base {
            let site = w
                .map
                .shipyard_sites
                .iter()
                .map(|s| s.center())
                .filter(|&p| our_water(ctx, body, p))
                .min_by_key(|p| p.dist2(b));
            if site.is_some() {
                self.rotation_since = None;
                return site;
            }
        }
        let home = w.own.home?;
        let mut starts: Vec<(usize, Pos)> = w
            .map
            .starts
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, s)| !s.within(home, START_MIN_DISTANCE))
            .collect();
        if starts.is_empty() {
            return None;
        }
        starts.sort_by_key(|&(i, s)| (std::cmp::Reverse(s.dist2(home)), i));
        let since = *self.rotation_since.get_or_insert(ctx.tick);
        let turn = (ctx.tick.saturating_sub(since) / START_ROTATE) as usize;
        starts.get(turn % starts.len()).map(|&(_, s)| s)
    }

    /// One hull's order, first match (§5.12).
    fn duty(
        &mut self,
        ctx: &Ctx<'_>,
        u: UnitId,
        station: Pos,
        objective: Option<Pos>,
    ) -> Option<Command> {
        let s = ctx.world.own.get(u)?;
        let d = ctx.def(s.unit.def)?;
        let here = s.unit.pos;
        let a = &ctx.world.army;

        // Commander guard.
        if a.commander_danger {
            if let Some(c) = ctx.world.own.commander.and_then(|c| ctx.world.own.get(c)) {
                let cpos = c.unit.pos;
                if here.within(cpos, dist(ctx.knobs.cmd.guard_radius)) {
                    let t = a
                        .commander_threats
                        .iter()
                        .filter(|&&t| ctx.target(t).is_some() && ctx.view.can_hit(u, t))
                        .filter_map(|&t| ctx.world.contacts.get(t))
                        .min_by_key(|c| (c.pos.dist2(cpos), c.id));
                    return match t {
                        Some(t) => self.engage(ctx, u, here, d, t.id),
                        None if !here.within(cpos, COMMANDER_GUARD_SLACK) => {
                            Some(Command::Move(cpos))
                        }
                        None => None,
                    };
                }
            }
        }

        // Submarines hunt the commander while the fleet is out.
        let body = ctx.world.map.water_body(station.tile());
        if self.sailing && ctx.world.roles.is(s.unit.def, Role::Submarine) {
            if let Some(c) = a.hunt_target.and_then(|c| ctx.world.contacts.get(c)) {
                if our_water(ctx, body, c.pos) {
                    return Some(if ctx.target(c.id).is_some() {
                        Command::Attack(c.id)
                    } else {
                        Command::Move(c.pos)
                    });
                }
            }
        }

        // Seen, naval-reachable enemies in reach.
        let t = ctx
            .world
            .contacts
            .near(here, ENGAGE_RADIUS)
            .filter(|c| !c.frame && ctx.world.contacts.seen_now(c.id))
            .filter(|c| ctx.world.reach.naval_reachable(c.pos) || our_water(ctx, body, c.pos))
            .filter(|c| !self.ignored.contains_key(&(u, c.id)))
            .filter(|c| matches!(ctx.standing(c.id), Some(Standing::Live(_))))
            .filter(|c| ctx.view.can_hit(u, c.id))
            .min_by_key(|c| (c.pos.dist2(here), c.id))
            .map(|c| c.id);
        if let Some(t) = t {
            return self.engage(ctx, u, here, d, t);
        }

        match objective {
            Some(o) => Some(Command::AttackMove(o)),
            None if !here.within(station, RALLY_SLACK) => Some(Command::Move(station)),
            None => None,
        }
    }

    /// Attack with stand-off and stall repositioning (§5.12).
    fn engage(
        &mut self,
        ctx: &Ctx<'_>,
        u: UnitId,
        here: Pos,
        d: &UnitDef,
        t: UnitId,
    ) -> Option<Command> {
        let c = *ctx.world.contacts.get(t)?;
        let ours = i64::from(d.max_range());
        let theirs = i64::from(ctx.def(c.def).map_or(0, UnitDef::max_range));
        let gap = here.dist(c.pos);

        // A reposition under way is finished first.
        if let Some(clock) = self.stalls.get_mut(&(u, t)) {
            if let Some(p) = clock.moving_to {
                let moving = matches!(super::current_order(ctx, u), Some(Command::Move(_)));
                if !here.within(p, SAME_MOVE) && moving {
                    return Some(Command::Move(p));
                }
                clock.moving_to = None;
            }
        }

        // The stall clock.
        let k = &ctx.knobs;
        let clock = self
            .stalls
            .entry((u, t))
            .or_insert_with(|| StallClock::new(c.health, ctx.tick));
        if clock.tick(ctx.tick, c.health, gap <= ours, secs(k.navy.stall_s)) {
            if clock.tries < k.army.stall_tries {
                clock.tries += 1;
                clock.elapsed = 0;
                let spots: Vec<Pos> = STALL_RADII
                    .iter()
                    .flat_map(|&r| {
                        (0..STALL_BEARINGS)
                            .map(move |b| on_bearing(c.pos, b * 16 / STALL_BEARINGS, r))
                    })
                    .filter(|&p| {
                        ctx.view.map().contains(p)
                            && depth(ctx, p) > 0
                            && ctx.world.reach.naval_reachable(p)
                            && !p.within(here, SAME_MOVE)
                    })
                    .collect();
                let pick = usize::try_from(clock.tries - 1).unwrap_or(0);
                if let Some(&p) = spots.get(pick % spots.len().max(1)) {
                    clock.moving_to = Some(p);
                    return Some(Command::Move(p));
                }
            } else {
                self.stalls.remove(&(u, t));
                self.ignored
                    .insert((u, t), ctx.tick.saturating_add(secs(k.army.stall_forget_s)));
                return Some(Command::Move(here));
            }
        }

        // Stand-off: an outranging hull holds off, buildings included.
        if k.navy.stand_off && ours > theirs + STAND_OFF_MARGIN {
            let keep = stand_off(theirs, ours, STAND_OFF_MARGIN);
            if gap < keep && here != c.pos {
                let back = c.pos.toward(here, keep);
                if depth(ctx, back) > 0 && ctx.world.reach.naval_reachable(back) {
                    return Some(Command::Move(back));
                }
            }
        }
        Some(Command::Attack(t))
    }
}
