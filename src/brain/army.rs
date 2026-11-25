//! The army manager (SPEC §5.1-5.7, §5.11-5.13) and the other §5 roles, each
//! its own slot: [`navy::NavyManager`], [`scouts::ScoutManager`],
//! [`transport::TransportManager`] and [`air::AirManager`] (those four are
//! still stubs).
//!
//! # Where the army runs
//!
//! The core calls [`ArmyManager::update`] at three steps (see
//! [`super::Ctx::step`]):
//!
//! - [`super::Step::Phase`] every tick (SPEC §3.1 step 5), in [`picture`]:
//!   fill `world.threat` (intruders, target intruder, sector memory with its
//!   decay, §5.4), take the radar warning sample when `ctx.due.radar` (§5.4,
//!   1 s), keep the odds gate and run the phase machine (§5.1) into
//!   `world.army.phase`. The planner runs later in the same tick and reads
//!   these.
//! - [`super::Step::Sweep`] when `ctx.due.sweep` (every 15 ticks, before
//!   the planner), in [`safety`]: the exposed-commander verdict (§5.5), the
//!   commander's danger, fight or flight and d-gun (§5.7), and builder
//!   retreat (§5.7). Writes `world.coord.commander_fleeing` and
//!   `world.coord.sheltering`. Fleeing, sheltering and d-gun orders are
//!   staged at [`super::Priority::Urgent`]; the dispatcher lets the army's
//!   urgent orders reach the economy's builders (its survival rule).
//! - [`super::Step::Army`] when `ctx.due.tactics` (§3.1 step 10), in
//!   [`tactics`] and [`micro`]: the rally point, waves, reserve and
//!   objective (§5.2), the counter lean (§5.3, [`mix`], into
//!   `world.army.lean`), defence and proportional response, the radar line
//!   and outpost raids (§5.4), hunting exposed commanders (§5.5), raids,
//!   guards and anti-air escorts (§5.6), damaged units (§5.11), per-unit
//!   micro (§5.12) and the base-loss rules (§5.13).
//!
//! # Orders and the limiter
//!
//! Move, attack, attack-move, d-gun and guard orders pass the micro limiter
//! (SPEC §2.3): a refused unit loses its orders for the tick and the next
//! pass regenerates them, so every pass decides from fresh state. A unit is
//! re-ordered only when its wanted order differs from the one it holds (in
//! its queue or in flight), which keeps the limiter's budget for changes.
//!
//! # Shared state
//!
//! Reads: `world.own`, `world.contacts` and `world.enemy`, `world.grid`,
//! `world.reach`, `world.coord` (`site_guard`, `ferry_rally`,
//! `commander_tasked`), `world.economy`.
//!
//! Writes: `world.threat` (every field), `world.army` (every field),
//! `world.coord.commander_fleeing`, `world.coord.sheltering`.
//!
//! # Navy hooks (§5.12)
//!
//! The navy reads `world.army.commander_danger` and
//! `world.army.commander_threats` for its commander guard,
//! `world.army.hunt_target` for submarines hunting the commander, and may
//! reuse [`micro::StallClock`], [`micro::stand_off`] and the bearings of
//! [`geom`] for its stand-off and stall repositioning.
//!
//! # Numbers
//!
//! Every tunable comes from the knobs. The few fixed distances and times the
//! spec states without a knob are the named constants below, each with its
//! section.
//!
//! Owns combat units and mobile anti-air; the commander is shared. Units lent
//! to the scouts or the transports are not in `world.own.combat` until
//! returned.

pub mod air;
pub mod geom;
pub mod micro;
pub mod mix;
pub mod navy;
pub mod picture;
pub mod safety;
pub mod scouts;
pub mod tactics;
pub mod transport;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use super::{Ctx, Manager, Staging, Step};
use crate::knobs::Fixed;
use crate::view::{Amount, Command, Pos, Tick, UnitDef, UnitId, TICKS_PER_SECOND};

/// §5.4: an intruder may also be within this distance of a besieged factory.
pub const BESIEGED_FACTORY_RADIUS: i64 = 400;
/// §5.4: a radar sample counts a unit as closing when it is this much nearer.
pub const RADAR_CLOSING: i64 = 8;
/// §5.4: the radar warning is held this long.
pub const RADAR_HOLD: Tick = 10 * TICKS_PER_SECOND;
/// §5.4: the radar line's distance from home, slot count, pitch and slack.
pub const RADAR_LINE_DISTANCE: i64 = 540;
pub const RADAR_LINE_SLOTS: u32 = 7;
pub const RADAR_LINE_PITCH: i64 = 56;
pub const RADAR_LINE_SLACK: i64 = 72;
/// §5.2: idle units further than this from the rally point walk to it.
pub const RALLY_SLACK: i64 = 96;
/// §5.2: the reserve joins at once when the wave is down to this many.
pub const RESERVE_JOINS_AT: usize = 3;
/// §5.6: the raid's builder search radius round a fresh frame, and the
/// raid release distance from the wave's objective.
pub const RAID_BUILDER_RADIUS: i64 = 480;
pub const RAID_RELEASE_RADIUS: i64 = 900;
/// §5.6: guards stand within this of the site; a building of ours within
/// the second radius ends the guard.
pub const SITE_GUARD_STAND: i64 = 128;
pub const SITE_GUARD_DONE: i64 = 64;
/// §5.6: commander guards move to it when further than this.
pub const COMMANDER_GUARD_SLACK: i64 = 160;
/// §5.7: builder retreat: the least step, and the map margin.
pub const RETREAT_MIN_STEP: i64 = 96;
pub const MAP_MARGIN: i64 = 64;
/// §5.7: reach added to a building's weapon range, and how far behind a
/// position a mobile unit may stand and still cover it.
pub const BUILDING_REACH: i64 = 32;
pub const COVER_BEHIND: i64 = 64;
/// §5.7: the commander stays in danger this long after it loses health.
pub const COMMANDER_HURT: Tick = 10 * TICKS_PER_SECOND;
/// §5.7: the d-gun's friendly-fire corridor and its reach past range.
pub const DGUN_CORRIDOR: i64 = 48;
pub const DGUN_OVERREACH: i64 = 32;
/// §5.7: the chase leash, as a multiple of the danger radius (1.5 = 3 / 2).
pub const LEASH_NUM: i64 = 3;
pub const LEASH_DEN: i64 = 2;
/// §5.7: fleeing from land threats: home is the goal beyond this distance,
/// else the step straight away from the nearest threat.
pub const FLEE_HOME_BEYOND: i64 = 300;
pub const FLEE_STEP: i64 = 350;
/// §5.7: fleeing from ships: their reach margin, the search rings, and the
/// gain the "anywhere" fallback must beat.
pub const SHIP_MARGIN: i64 = 100;
pub const SHIP_RINGS: [i64; 6] = [200, 350, 500, 700, 900, 1200];
pub const SHIP_FALLBACK_GAIN: i64 = 100;
/// §5.12: the stall sidestep's forward part.
pub const STALL_FORWARD: i64 = 120;
/// §5.12: blocked shots: muzzle height, ground clearance, sample bounds.
pub const MUZZLE_HEIGHT: i64 = 24;
pub const LINE_CLEARANCE: i64 = 12;
pub const LINE_SAMPLES_MIN: i64 = 2;
pub const LINE_SAMPLES_MAX: i64 = 24;
/// §5.11: the number of mend spots round home.
pub const MEND_SPOTS: usize = 8;
/// A unit holding an attack-move within this of the wanted point is left
/// alone (an implementation choice: it keeps re-orders for changes).
pub const SAME_POINT: i64 = 32;
/// The same for a move, which is also how close a move must be to have
/// arrived: kiting steps are small.
pub const SAME_MOVE: i64 = 8;

/// The army manager (SPEC §5).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArmyManager {
    /// Phase-step state (§5.1, §5.4).
    pub(crate) pic: picture::PictureState,
    /// Sweep state (§5.5, §5.7).
    pub(crate) safe: safety::SafetyState,
    /// Tactics state (§5.2-5.6, §5.11).
    pub(crate) tac: tactics::TacticsState,
    /// Per-unit micro state (§5.12).
    pub(crate) micro: micro::MicroState,
    /// The counter lean (§5.3).
    pub(crate) mix: mix::MixState,
}

impl Manager for ArmyManager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        match ctx.step {
            Step::Phase => self.phase_step(ctx),
            Step::Sweep => self.sweep(ctx, out),
            Step::Army => self.tactics(ctx, out),
            _ => {}
        }
    }
}

/// A seconds knob as ticks (negative values count as 0).
pub(crate) fn secs(s: i32) -> Tick {
    u32::try_from(s)
        .unwrap_or(0)
        .saturating_mul(TICKS_PER_SECOND)
}

/// A distance knob in whole world units.
pub(crate) fn dist(k: Fixed) -> i64 {
    k.floor()
}

/// `a ≥ k × b`, exactly.
pub(crate) fn at_least(a: i64, k: Fixed, b: i64) -> bool {
    i128::from(a) * 10_000 >= i128::from(k.raw()) * i128::from(b)
}

/// The metal of a definition (0 for an unknown one).
pub(crate) fn metal_of(d: Option<&UnitDef>) -> Amount {
    d.map(|d| d.cost_metal).unwrap_or_default()
}

/// Health as a percentage of the definition's (100 when unknown).
pub(crate) fn health_pct(health: i32, d: Option<&UnitDef>) -> i64 {
    match d {
        Some(d) if d.health > 0 => i64::from(health) * 100 / i64::from(d.health),
        _ => 100,
    }
}

/// The order a unit holds now: its newest in-flight group's first order,
/// else the head of its visible queue.
pub(crate) fn current_order(ctx: &Ctx<'_>, unit: UnitId) -> Option<Command> {
    if let Some(o) = ctx.in_flight(unit) {
        return o.first().map(|o| o.command);
    }
    ctx.world
        .own
        .get(unit)
        .and_then(|s| s.unit.commands.first().copied())
}

/// Whether `held` already does what `want` asks.
pub(crate) fn same_order(held: Command, want: Command) -> bool {
    match (held, want) {
        (Command::Move(p), Command::Move(q)) => p.within(q, SAME_MOVE),
        (Command::AttackMove(p), Command::AttackMove(q)) => p.within(q, SAME_POINT),
        _ => held == want,
    }
}

/// Stages `want` for `unit` unless it already holds it.
pub(crate) fn order(ctx: &Ctx<'_>, out: &mut Staging, unit: UnitId, want: Command) {
    if current_order(ctx, unit).is_some_and(|h| same_order(h, want)) {
        return;
    }
    if let Command::Move(p) = want {
        // A move to where the unit stands is a stop: needless when idle.
        let here = ctx.world.own.get(unit).map(|s| s.unit.pos);
        if here.is_some_and(|h| h.within(p, SAME_MOVE)) && current_order(ctx, unit).is_none() {
            return;
        }
    }
    use crate::view::Orders;
    out.now(unit, want);
}

/// Units in id order without the members of `sets`.
pub(crate) fn without(units: &[UnitId], sets: &[&BTreeSet<UnitId>]) -> Vec<UnitId> {
    units
        .iter()
        .copied()
        .filter(|u| sets.iter().all(|s| !s.contains(u)))
        .collect()
}

/// A tick-stamped per-unit map pruned to living units.
pub(crate) fn prune<V>(m: &mut BTreeMap<UnitId, V>, alive: impl Fn(UnitId) -> bool) {
    m.retain(|&u, _| alive(u));
}

/// The home point or nothing: with no home there is no rally, no defence
/// line, no mending and no outpost response (§5.13).
pub(crate) fn home(ctx: &Ctx<'_>) -> Option<Pos> {
    ctx.world.own.home
}
