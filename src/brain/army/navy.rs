//! The navy (SPEC §5.12 "Navy"). **A stub**: it does nothing yet.
//!
//! Runs at [`crate::brain::Step::Navy`] when `ctx.due.tactics`, right after
//! the army (both are SPEC §3.1 step 10).
//!
//! Must implement:
//!
//! - the station `navy.rally` out from the first shipyard on the first of
//!   16 bearings with deep, connected water (use `world.reach.naval` and
//!   `world.map` water bodies);
//! - sailing at `navy.sail_size` hulls within 440 of the yard, recall below
//!   2, and the phase gate `navy.attack_size` (§5.1, read
//!   `world.army.phase`);
//! - the objective: the remembered enemy on our water body nearest the enemy
//!   base, else the shipyard site nearest it (`world.map.shipyard_sites`),
//!   else start positions more than 900 from home, rotating every 120 s;
//! - per hull: commander guard, submarines hunting the commander, attacks on
//!   seen naval-reachable enemies with stand-off, and stall repositioning
//!   after `navy.stall_s`.
//!
//! Reads `world.own.hulls`, `world.contacts`, `world.enemy`, `world.reach`,
//! `world.map`, `world.army`. Writes nothing shared. Owns hulls; a hull lent
//! as the naval scout (`scout.naval`) is in `world.own.scouts` instead.

use crate::brain::{Ctx, Manager, Staging};

/// The navy manager. Does nothing yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavyManager {}

impl Manager for NavyManager {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}
