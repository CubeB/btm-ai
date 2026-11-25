//! The economy and build manager (SPEC §4). **A stub**: it does nothing yet.
//!
//! # Where it runs
//!
//! The core calls [`EconomyManager::update`] at two steps (see
//! [`super::Ctx::step`]):
//!
//! - [`super::Step::MetalMakers`] every tick (SPEC §3.1 step 6, §4.9). The
//!   dispatcher already lets through at most one on/off command per maker
//!   per second and drops one that matches the maker's state, so this step
//!   may simply stage the wanted state.
//! - [`super::Step::Plan`] when `ctx.due.plan` (`cadence.plan_ticks`; SPEC §2.2,
//!   §3.1 step 7): top up factories, then give **exactly one** idle builder
//!   one job (§4.2). Stage planner jobs for the commander at
//!   [`super::Priority::Tasked`] so that the army's routine orders do not
//!   overwrite them; the core then sets `coord.commander_tasked`.
//!
//! # What it must implement
//!
//! - §4.1 shared economy predicates: the fixed ones are already methods of
//!   [`super::EconomySummary`] (`world.economy`); add "air matters", "tech
//!   income", "tier worth it" (computed once), the naval fleet target,
//!   "capacity short" (history; publish in `world.eco.capacity_short`) and
//!   affordability (stock + (income − `world.own.committed_metal`) × time).
//! - §4.2 the builder assignment loop: round-robin cursor over
//!   `world.own.idle_builders`, dropped-order check and failed cells
//!   (`world.eco.failed_cells`), the context rules (commander on
//!   `world.reach.commander`, others on `world.reach.ground`; water
//!   structures exempt), repair, orphaned frames (`world.own.orphan_frames`),
//!   advanced-lab assist, mending, reclaim, the extractor upgrade, the
//!   priority-list walk with saving (`world.eco.saving_for`), commander
//!   assist, harvest reclaim and factory assist.
//! - §4.3 the build priority list (the strategy), rebuilt per builder, with
//!   recent building losses (`world.own.building_losses`) moved to the front.
//! - §4.4 factory production, using the counter lean `world.army.lean`.
//! - §4.5 expansion: deposits are in `world.map.deposits` (8-connected patch
//!   tiles, centre of mass computed); site claims are in `world.claims`
//!   (rebuilt every tick from queues and in-flight orders; call
//!   `world.claims.note` when staging an extractor build so a second builder
//!   this tick sees it); the heart wait; the upgrade fallback, marking the
//!   reclaimed extractor in `world.own.deliberate_removals` and the site in
//!   `world.eco.pending_upgrades`.
//! - §4.6 generic siting (placement through `ctx.leaks.leak_can_place`;
//!   ties broken by `ctx.rng`), facing towers at
//!   `world.threat.sectors.strongest()` (`def.face_threat_sector`).
//! - §4.7 outposts, §4.8 defences and fortification, §4.9 energy and metal
//!   makers, §4.10 constructor counts, §4.11 repair and mending, §4.12
//!   reclaim (own wrecks in `world.own.wreck_sites`), §4.13 the timed
//!   opening.
//! - §5.9 "expansion wanted": publish an unreachable extractor site in
//!   `world.eco.unreachable_expansion` (at most every 10 s).
//!
//! # Shared state
//!
//! Reads: `world.own` (units, counts, losses, idle builders, home),
//! `world.economy`, `world.contacts` / `world.enemy` (enemy base, under-guns
//! tests, besieged factories), `world.grid`, `world.map`, `world.reach`,
//! `world.claims`, `world.threat` (urgency: `base_under_attack`,
//! `raided_outpost`), `world.army` (phase, lean), `world.coord`
//! (`commander_fleeing`, `sheltering`).
//!
//! Writes: `world.eco` (every field), `world.coord.site_guard` (the
//! build-site guard request, §5.6), `world.own.deliberate_removals`,
//! `world.claims` (notes).
//!
//! Owns (see [`super::ownership`]): factories, builders and structures; the
//! commander is shared with the army. Builders lent to the transport pass as
//! ferry passengers are not in `world.own.builders` until returned.

use super::{Ctx, Manager, Staging};

/// The economy and build manager (SPEC §4). Does nothing yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomyManager {}

impl Manager for EconomyManager {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}
