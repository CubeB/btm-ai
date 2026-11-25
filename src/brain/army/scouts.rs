//! Scouting (SPEC §5.10). **A stub**: it does nothing yet.
//!
//! Runs at [`crate::brain::Step::Scouts`] when `ctx.due.scouts`
//! (`cadence.scout_ticks`; SPEC §3.1 step 8). Its orders are not
//! micro-limited (the limiter covers the army and air passes only, §2.3).
//!
//! Must implement:
//!
//! - **Borrowing.** With no scouts owned, built or being built, borrow the
//!   first combat unit: `world.ownership.lend(unit, Source::Army,
//!   Source::Scouts, LoanReason::BorrowedScout)`, and mirror it in
//!   `world.coord.borrowed_scout`. Give it back
//!   (`world.ownership.give_back`) once a real scout appears. On water maps
//!   with nothing enemy known, borrow one hull the same way
//!   (`LoanReason::NavalScout`, `scout.naval`) and return it once any enemy
//!   is known. The army size already excludes a borrowed combat unit.
//! - **Each pass** (`scout.enabled`): leave a scout with orders alone
//!   (`ctx.busy`); hurt scouts go home; the opening leg to the stalest
//!   declared start more than 512 from home; other legs (planes 3, ground and
//!   sea 1) maximising `world.grid.best_scout_cell`, with the accept test
//!   (320 from other scouts' claims and earlier legs, zero ground threat,
//!   right terrain and reachability).
//!
//! Reads `world.own.scouts`, `world.own.combat`, `world.grid`,
//! `world.map.starts`, `world.reach`, `world.enemy`. Writes
//! `world.coord.borrowed_scout` and loans in `world.ownership`.

use crate::brain::{Ctx, Manager, Staging};

/// The scout manager. Does nothing yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoutManager {}

impl Manager for ScoutManager {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}
