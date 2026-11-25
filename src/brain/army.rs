//! The army manager (SPEC §5) and the other §5 roles, each its own slot:
//! [`navy::NavyManager`], [`scouts::ScoutManager`],
//! [`transport::TransportManager`] and [`air::AirManager`]. **Stubs**: they
//! do nothing yet.
//!
//! # Where the army runs
//!
//! The core calls [`ArmyManager::update`] at three steps (see
//! [`super::Ctx::step`]):
//!
//! - [`super::Step::Phase`] every tick (SPEC §3.1 step 5): fill
//!   `world.threat` (intruders, target intruder, sector memory with its
//!   decay, §5.4), take the radar warning sample when `ctx.due.radar`
//!   (§5.4, 1 s), and run the phase machine (§5.1) into `world.army.phase`.
//!   The planner runs later in the same tick and reads these.
//! - [`super::Step::Sweep`] when `ctx.due.sweep` (every 15 ticks, before
//!   the planner): builder safety and retreat, the commander's safety and
//!   repair dispatch, and the defence attack watch (§5.7). Write
//!   `world.coord.commander_fleeing` and `world.coord.sheltering`. Stage
//!   fleeing and sheltering orders at [`super::Priority::Urgent`].
//! - [`super::Step::Army`] when `ctx.due.tactics` (`cadence.tactics_ticks`,
//!   first on tick 15; §3.1 step 10): waves, reserve and objective (§5.2),
//!   the counter lean (§5.3, into `world.army.lean`; the role histogram is
//!   already kept in `world.enemy.roles`), defence and proportional response
//!   (§5.4), hunting exposed commanders (§5.5), raids, guards and the
//!   build-site guard (`world.coord.site_guard`, §5.6), damaged units
//!   (§5.11), per-unit micro (§5.12) and base-loss recovery (§5.13).
//!
//! # Orders and the limiter
//!
//! Move, attack, attack-move, patrol, d-gun and guard orders from the army,
//! navy and air slots pass the micro limiter (SPEC §2.3): a refused unit
//! loses its orders for the tick and the next pass regenerates them, so
//! passes must be stateless about whether an order arrived. A unit with
//! orders in flight (`ctx.in_flight(unit)`) has not seen them yet. Orders
//! aimed at a unit need the target seen now (`ctx.target(id)`). The
//! commander is shared with the economy: do not overwrite a planner job
//! (`world.coord.commander_tasked`) unless the order outranks it.
//!
//! # Shared state
//!
//! Reads: `world.own` (`combat`, `anti_air`, `army_size`, `commander`,
//! losses, home), `world.contacts` and `world.enemy` (enemy base, focus,
//! army values, air threat, besieged factories), `world.grid` (best attack
//! cell, threat in radius), `world.reach`, `world.map`, `world.coord`
//! (`site_guard`, `ferry_rally`, `commander_tasked`), `world.eco`
//! (`saving_for`).
//!
//! Writes: `world.threat` (every field), `world.army` (every field),
//! `world.coord.commander_fleeing`, `world.coord.sheltering`.
//!
//! Owns combat units and mobile anti-air; the commander is shared. Units lent
//! to the scouts or the transports are not in `world.own.combat` until
//! returned.

pub mod air;
pub mod navy;
pub mod scouts;
pub mod transport;

use super::{Ctx, Manager, Staging};

/// The army manager (SPEC §5). Does nothing yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArmyManager {}

impl Manager for ArmyManager {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}
