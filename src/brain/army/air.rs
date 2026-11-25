//! The air pass (SPEC §5.8). **A stub**: it does nothing yet.
//!
//! Runs at [`crate::brain::Step::Air`] when `ctx.due.tactics`, last of the
//! managers (SPEC §3.1 step 11). Its move, attack, patrol and guard orders
//! pass the micro limiter (§2.3).
//!
//! Must implement: the wait point (repair pad, else air plant, else home)
//! and the `air.leash`; standing patrols (the first `air.patrol_fighters`
//! fighters and `air.patrol_bombers` bombers by id, 4 points round the
//! enemy-facing direction; queue the first point Immediate, the rest
//! Queued); fighters on seen enemy aircraft, armed first; bomber and
//! gunship sorties avoiding anti-air cover (`world.grid` AA layer).
//!
//! Reads `world.own.aircraft`, `world.contacts`, `world.enemy` (air threat,
//! peak armed air), `world.grid`, `world.army`. Writes nothing shared. Owns
//! armed aircraft (`UnitClass::Aircraft`); unarmed aircraft are scouts, and
//! air transports belong to the transport pass.

use crate::brain::{Ctx, Manager, Staging};

/// The air manager. Does nothing yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirManager {}

impl Manager for AirManager {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}
