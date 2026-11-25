//! The air pass (SPEC §5.8).
//!
//! Runs at [`crate::brain::Step::Air`] when `ctx.due.tactics`, last of the
//! managers (SPEC §3.1 step 11). Its move, attack and patrol orders pass the
//! micro limiter (§2.3).
//!
//! Each pass:
//!
//! - **Wait point:** a repair pad, else an air plant, else home. Idle
//!   aircraft further than `air.leash` from it fly back.
//! - **Standing patrol:** the first `air.patrol_fighters` fighters and
//!   `air.patrol_bombers` bombers by id fly 4 points at −60°, −20°, +20°
//!   and +60° from the enemy-facing direction, clamp(base edge, 200, 900) +
//!   `air.patrol_margin` from home (first point Immediate, the rest
//!   Queued). Patrol bombers never join sorties.
//! - **Fighters** attack a seen enemy aircraft, armed first, then the
//!   nearest; no chase leash.
//! - **Bombers** (at least [`BOMBER_PACK`] off patrol): home defence, else
//!   a building sortie scored by metal, factory weight, anti-air cover,
//!   distance and the wave's reach, else the largest cluster.
//! - **Gunships** (at least `air.gunship_pack`): the seen armed ground
//!   contact nearest the wave centre, else the bomber target.
//! - **Joint timing** (`air.sync`): bomber and gunship strikes are held
//!   while the wave is out, unspent and has had no contact, the target is
//!   not near home, and the wave centre is near neither its objective nor
//!   the target. Home defence and fighters are never held.
//! - **Torpedo seaplanes** (at least [`TORPEDO_PACK`]): seen contacts over
//!   water, mobile first, then the nearest to home.
//!
//! Anti-air cover is the influence grid's AA layer (§3.3); every target
//! under more than `air.bomb_max_aa` is skipped, except by home defence.
//!
//! Reads `world.own.aircraft`, `world.contacts`, `world.grid`,
//! `world.army`. Writes nothing shared. Owns armed aircraft
//! (`UnitClass::Aircraft`); unarmed aircraft are scouts, and air transports
//! belong to the transport pass. No randomness: ties go to the lower id.

use super::{dist, order};
use crate::brain::roles::Role;
use crate::brain::{Contact, Ctx, Manager, Phase, Staging, Standing};
use crate::knobs::Fixed;
use crate::view::{Command, Orders, Pos, UnitDef, UnitId};

/// §5.8: bombers fly sorties once this many are off patrol.
pub const BOMBER_PACK: usize = 2;
/// §5.8: torpedo seaplanes fly once this many exist.
pub const TORPEDO_PACK: usize = 2;
/// §5.8: the patrol radius's base-edge clamp.
pub const PATROL_EDGE_MIN: i64 = 200;
pub const PATROL_EDGE_MAX: i64 = 900;
/// §5.8: the patrol points, as (cos, sin) × 1000 of −60°, −20°, +20° and
/// +60° from the enemy-facing direction.
pub const PATROL_ARC: [(i64, i64); 4] = [(500, -866), (940, -342), (940, 342), (500, 866)];
/// §5.8: a strike is never held when its target is this close to home.
pub const SYNC_HOME_RADIUS: i64 = 900;

/// What an aircraft is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fighter,
    Bomber,
    Gunship,
    Torpedo,
}

/// The kind of an armed aircraft: its role, else its weapons (anti-air:
/// fighter; dropped: bomber; otherwise a gunship).
fn kind(ctx: &Ctx<'_>, def: crate::view::DefId, d: &UnitDef) -> Kind {
    let r = &ctx.world.roles;
    if r.is(def, Role::Fighter) || r.is(def, Role::SeaplaneFighter) {
        Kind::Fighter
    } else if r.is(def, Role::Bomber) {
        Kind::Bomber
    } else if r.is(def, Role::Gunship) {
        Kind::Gunship
    } else if r.is(def, Role::TorpedoSeaplane) {
        Kind::Torpedo
    } else if d.weapons.iter().all(|w| w.anti_air) {
        Kind::Fighter
    } else if d.weapons.iter().any(|w| w.dropped) {
        Kind::Bomber
    } else {
        Kind::Gunship
    }
}

/// A strike target and whether it is home defence (never held, ignores AA).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Strike {
    target: UnitId,
    home_defence: bool,
}

/// The air manager.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirManager {}

impl Manager for AirManager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        if ctx.step != crate::brain::Step::Air {
            return;
        }
        let Some(home) = ctx.home() else {
            return;
        };
        let wait = wait_point(ctx).unwrap_or(home);
        let mut fighters = Vec::new();
        let mut bombers = Vec::new();
        let mut gunships = Vec::new();
        let mut torpedo = Vec::new();
        for &u in &ctx.world.own.aircraft {
            let Some(s) = ctx.world.own.get(u) else {
                continue;
            };
            let Some(d) = ctx.def(s.unit.def) else {
                continue;
            };
            match kind(ctx, s.unit.def, d) {
                Kind::Fighter => fighters.push(u),
                Kind::Bomber => bombers.push(u),
                Kind::Gunship => gunships.push(u),
                Kind::Torpedo => torpedo.push(u),
            }
        }
        let k = &ctx.knobs.air;
        let patrol_fighters = usize::try_from(k.patrol_fighters.max(0)).unwrap_or(0);
        let patrol_bombers = usize::try_from(k.patrol_bombers.max(0)).unwrap_or(0);
        let route = patrol_route(ctx, home);

        // Fighters.
        let air_targets = enemy_aircraft(ctx);
        for (i, &u) in fighters.iter().enumerate() {
            let Some(here) = pos_of(ctx, u) else {
                continue;
            };
            let t = air_targets
                .iter()
                .filter(|c| ctx.view.can_hit(u, c.id))
                .min_by_key(|c| (!c.armed, c.pos.dist2(here), c.id));
            if let Some(c) = t {
                order(ctx, out, u, Command::Attack(c.id));
            } else if i < patrol_fighters {
                patrol(ctx, out, u, &route);
            } else {
                leash(ctx, out, u, here, wait);
            }
        }

        // Bombers and gunships.
        let bomber_target = bomber_target(ctx, home);
        let sortie: Vec<UnitId> = bombers.iter().skip(patrol_bombers).copied().collect();
        for &u in bombers.iter().take(patrol_bombers) {
            patrol(ctx, out, u, &route);
        }
        let bombers_go = sortie.len() >= BOMBER_PACK;
        for &u in &sortie {
            let strike = bomber_target.filter(|s| bombers_go && !held(ctx, home, *s));
            fly(ctx, out, u, strike.map(|s| s.target), wait);
        }
        let pack = usize::try_from(k.gunship_pack.max(1)).unwrap_or(usize::MAX);
        let gunship_target = (gunships.len() >= pack)
            .then(|| gunship_target(ctx).or(bomber_target))
            .flatten()
            .filter(|s| !held(ctx, home, *s));
        for &u in &gunships {
            fly(ctx, out, u, gunship_target.map(|s| s.target), wait);
        }

        // Torpedo seaplanes.
        let torpedo_target = (torpedo.len() >= TORPEDO_PACK)
            .then(|| torpedo_target(ctx, home))
            .flatten();
        for &u in &torpedo {
            fly(ctx, out, u, torpedo_target, wait);
        }
    }
}

fn pos_of(ctx: &Ctx<'_>, u: UnitId) -> Option<Pos> {
    ctx.world.own.get(u).map(|s| s.unit.pos)
}

/// A repair pad, else an air plant, else nothing (the caller uses home).
fn wait_point(ctx: &Ctx<'_>) -> Option<Pos> {
    let w = &ctx.world;
    let first = |roles: &[Role]| {
        w.own
            .units
            .iter()
            .find(|s| !s.is_frame() && roles.iter().any(|&r| w.roles.is(s.unit.def, r)))
            .map(|s| s.unit.pos)
    };
    first(&[Role::AirRepairPad]).or_else(|| first(&[Role::AirPlant, Role::AdvancedAirPlant]))
}

/// The four patrol points (§5.8).
fn patrol_route(ctx: &Ctx<'_>, home: Pos) -> Vec<Pos> {
    let info = ctx.view.map();
    let facing = ctx
        .world
        .enemy_base()
        .filter(|&b| b != home)
        .unwrap_or(Pos::new(info.width() / 2, info.height() / 2));
    let edge = ctx
        .world
        .own
        .units
        .iter()
        .filter(|s| s.building)
        .map(|s| s.unit.pos.dist(home))
        .max()
        .unwrap_or(0)
        .clamp(PATROL_EDGE_MIN, PATROL_EDGE_MAX);
    let r = edge + dist(ctx.knobs.air.patrol_margin);
    let len = home.dist(facing);
    if len == 0 {
        return vec![super::geom::inside(
            ctx.view,
            super::geom::pos(i64::from(home.x) + r, i64::from(home.z)),
            super::MAP_MARGIN,
        )];
    }
    let dx = i64::from(facing.x) - i64::from(home.x);
    let dz = i64::from(facing.z) - i64::from(home.z);
    PATROL_ARC
        .iter()
        .map(|&(c, s)| {
            let x = i64::from(home.x) + r * (dx * c - dz * s) / (len * 1000);
            let z = i64::from(home.z) + r * (dx * s + dz * c) / (len * 1000);
            super::geom::inside(ctx.view, super::geom::pos(x, z), super::MAP_MARGIN)
        })
        .collect()
}

/// Puts a unit on the patrol route unless it already patrols.
fn patrol(ctx: &Ctx<'_>, out: &mut Staging, u: UnitId, route: &[Pos]) {
    if matches!(super::current_order(ctx, u), Some(Command::Patrol(_))) {
        return;
    }
    for (i, &p) in route.iter().enumerate() {
        if i == 0 {
            out.now(u, Command::Patrol(p));
        } else {
            out.queued(u, Command::Patrol(p));
        }
    }
}

/// An idle aircraft beyond the leash flies back to the wait point.
fn leash(ctx: &Ctx<'_>, out: &mut Staging, u: UnitId, here: Pos, wait: Pos) {
    if !ctx.busy(u) && !here.within(wait, dist(ctx.knobs.air.leash)) {
        order(ctx, out, u, Command::Move(wait));
    }
}

/// Strikes `target` (attack when seen now, else fly to its last position),
/// or idles on the leash.
fn fly(ctx: &Ctx<'_>, out: &mut Staging, u: UnitId, target: Option<UnitId>, wait: Pos) {
    let Some(here) = pos_of(ctx, u) else {
        return;
    };
    match target.and_then(|t| ctx.world.contacts.get(t)) {
        Some(c) if ctx.target(c.id).is_some() => order(ctx, out, u, Command::Attack(c.id)),
        Some(c) => order(ctx, out, u, Command::Move(c.pos)),
        None => leash(ctx, out, u, here, wait),
    }
}

/// Seen, standing enemy aircraft.
fn enemy_aircraft<'a>(ctx: &'a Ctx<'_>) -> Vec<&'a Contact> {
    ctx.world
        .contacts
        .iter()
        .filter(|c| c.air && ctx.world.contacts.seen_now(c.id))
        .filter(|c| matches!(ctx.standing(c.id), Some(Standing::Live(_))))
        .collect()
}

/// Anti-air cover over a contact.
fn cover(ctx: &Ctx<'_>, c: &Contact) -> i64 {
    i64::from(ctx.world.grid.aa_at(c.pos.vis_cell()))
}

fn max_aa(ctx: &Ctx<'_>) -> i64 {
    i64::from(ctx.knobs.air.bomb_max_aa)
}

/// The bomber target (§5.8), first available.
fn bomber_target(ctx: &Ctx<'_>, home: Pos) -> Option<Strike> {
    let k = &ctx.knobs.air;
    let w = &ctx.world;
    // 1. Home defence: ignores AA.
    let defence = w
        .contacts
        .near(home, dist(k.bomb_home_radius))
        .filter(|c| c.armed && !c.air && ctx.standing(c.id).is_some())
        .min_by_key(|c| (c.pos.dist2(home), c.id));
    if let Some(c) = defence {
        return Some(Strike {
            target: c.id,
            home_defence: true,
        });
    }
    // 2. Building sortie.
    let wave = w.army.wave_centre;
    let reach = dist(k.bomb_army_reach);
    let scale = dist(k.bomb_distance_scale).max(1);
    let mut best: Option<(i128, UnitId)> = None;
    for c in w.contacts.iter().filter(|c| c.building) {
        let aa = cover(ctx, c);
        if aa > max_aa(ctx) || ctx.standing(c.id).is_none() {
            continue;
        }
        let s = sortie_score(
            c.metal.milli(),
            c.builder,
            aa,
            c.pos.dist(home),
            wave.is_some_and(|p| c.pos.within(p, reach)),
            k.bomb_factory_weight,
            k.bomb_aa_penalty,
            scale,
            k.bomb_army_discount,
        );
        if best.is_none_or(|(b, _)| s > b) {
            best = Some((s, c.id));
        }
    }
    if let Some((_, id)) = best {
        return Some(Strike {
            target: id,
            home_defence: false,
        });
    }
    // 3. Cluster.
    let r = dist(k.bomb_cluster_radius);
    let min = usize::try_from(k.bomb_cluster_min.max(1)).unwrap_or(usize::MAX);
    let ground: Vec<&Contact> = w
        .contacts
        .iter()
        .filter(|c| !c.air && ctx.standing(c.id).is_some())
        .collect();
    ground
        .iter()
        .filter(|c| !c.building && w.contacts.seen_now(c.id) && cover(ctx, c) <= max_aa(ctx))
        .map(|c| {
            let n = ground.iter().filter(|o| o.pos.within(c.pos, r)).count();
            (n, c)
        })
        .filter(|&(n, _)| n >= min)
        .min_by_key(|&(n, c)| (std::cmp::Reverse(n), c.pos.dist2(home), c.id))
        .map(|(_, c)| Strike {
            target: c.id,
            home_defence: false,
        })
}

/// The building sortie score (§5.8) as an exact integer quotient: metal ×
/// factory weight ÷ (1 + cover × penalty) ÷ (1 + distance / scale) ×
/// discount near the wave.
#[allow(clippy::too_many_arguments)]
fn sortie_score(
    metal_milli: i64,
    factory: bool,
    cover: i64,
    distance: i64,
    near_wave: bool,
    factory_weight: Fixed,
    aa_penalty: Fixed,
    scale: i64,
    discount: Fixed,
) -> i128 {
    let one = i128::from(Fixed::ONE.raw());
    let fw = if factory {
        i128::from(factory_weight.raw())
    } else {
        one
    };
    let disc = if near_wave {
        i128::from(discount.raw())
    } else {
        one
    };
    let num = i128::from(metal_milli) * fw * disc * i128::from(scale);
    let den = one
        * (one + i128::from(cover) * i128::from(aa_penalty.raw()))
        * (i128::from(scale) + i128::from(distance));
    if den <= 0 {
        return 0;
    }
    num / den
}

/// The gunship target (§5.8): the seen armed ground contact nearest the
/// wave centre, within `air.gunship_radius` of it, under little AA.
fn gunship_target(ctx: &Ctx<'_>) -> Option<Strike> {
    let c = ctx.world.army.wave_centre?;
    ctx.world
        .contacts
        .near(c, dist(ctx.knobs.air.gunship_radius))
        .filter(|e| e.armed && !e.air && ctx.world.contacts.seen_now(e.id))
        .filter(|e| cover(ctx, e) <= max_aa(ctx))
        .filter(|e| matches!(ctx.standing(e.id), Some(Standing::Live(_))))
        .min_by_key(|e| (e.pos.dist2(c), e.id))
        .map(|e| Strike {
            target: e.id,
            home_defence: false,
        })
}

/// The torpedo seaplane target: seen contacts over water, mobile first,
/// then the nearest to home.
fn torpedo_target(ctx: &Ctx<'_>, home: Pos) -> Option<UnitId> {
    ctx.world
        .contacts
        .iter()
        .filter(|c| !c.air && ctx.world.contacts.seen_now(c.id))
        .filter(|c| ctx.view.water_depth(c.pos.tile()) > 0)
        .filter(|c| matches!(ctx.standing(c.id), Some(Standing::Live(_))))
        .min_by_key(|c| (c.building, c.pos.dist2(home), c.id))
        .map(|c| c.id)
}

/// Joint ground and air timing (§5.8): whether a strike is held for the
/// wave.
fn held(ctx: &Ctx<'_>, home: Pos, s: Strike) -> bool {
    if s.home_defence || !ctx.knobs.air.sync {
        return false;
    }
    let a = &ctx.world.army;
    let spent = usize::try_from(ctx.knobs.army.spent_size.max(0)).unwrap_or(0);
    let wave_out = a.phase == Phase::Assault && !a.wave.is_empty() && a.wave.len() >= spent;
    if !wave_out || a.contact_since.is_some() {
        return false;
    }
    let Some(t) = ctx.world.contacts.get(s.target) else {
        return false;
    };
    if t.pos.within(home, SYNC_HOME_RADIUS) {
        return false;
    }
    let r = dist(ctx.knobs.air.sync_radius);
    let Some(c) = a.wave_centre else {
        return true;
    };
    let near_objective = a.objective.is_some_and(|o| c.within(o, r));
    !(near_objective || c.within(t.pos, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sortie_score_follows_the_formula() {
        let one = Fixed::ONE;
        let two = Fixed::from_int(2);
        let quarter = Fixed::from_raw(2500);
        // 100 metal, no AA, at home: 100 (milli 100000) × scale / scale.
        let base = sortie_score(100_000, false, 0, 0, false, two, one, 2400, quarter);
        assert_eq!(base, 100_000);
        // A factory doubles it; one AA unit halves it; a scale's distance
        // halves it; the wave discount quarters it.
        assert_eq!(
            sortie_score(100_000, true, 0, 0, false, two, one, 2400, quarter),
            200_000
        );
        assert_eq!(
            sortie_score(100_000, false, 1, 0, false, two, one, 2400, quarter),
            50_000
        );
        assert_eq!(
            sortie_score(100_000, false, 0, 2400, false, two, one, 2400, quarter),
            50_000
        );
        assert_eq!(
            sortie_score(100_000, false, 0, 0, true, two, one, 2400, quarter),
            25_000
        );
    }
}
