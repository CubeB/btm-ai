//! Transports and ferries on water maps (SPEC §5.9). **A stub**: it does
//! nothing yet.
//!
//! Runs at [`crate::brain::Step::Transports`] when `ctx.due.tactics`
//! (SPEC §3.1 step 9, before the army). Load, unload and stop orders pass
//! the micro limiter untouched (§2.3).
//!
//! Must implement:
//!
//! - **When a ferry is wanted:** enemy across water (latched),
//!   `ferry.want_from_map` (no land route: `world.reach.land_route_to_enemy`),
//!   expansion wanted (`world.eco.unreachable_expansion`).
//! - **Passengers:** book units with `world.ownership.lend(unit, lender,
//!   Source::Transport, LoanReason::FerryPassenger)` (from the army or the
//!   economy); other passes then leave them alone, and they drop out of the
//!   idle, combat and anti-air lists. Return them on landing or when a trip
//!   is called off.
//! - **The muster point:** publish `world.coord.ferry_rally`, which the
//!   army uses as its rally point while the army needs a sea ferry (§5.2).
//! - Trips, pickup timeouts, failed landings and the "never queue two
//!   identical unloads" rule of the order stream.
//!
//! Reads `world.own.transports`, `world.reach`, `world.map`, `world.army`
//! (objective), `world.eco`. Writes `world.coord.ferry_rally` and loans in
//! `world.ownership`.

use crate::brain::{Ctx, Manager, Staging};

/// The transport manager. Does nothing yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransportManager {}

impl Manager for TransportManager {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}
