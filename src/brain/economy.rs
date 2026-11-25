//! The economy and build manager (SPEC §4).
//!
//! # Where it runs
//!
//! The core calls [`EconomyManager::update`] at two steps (see
//! [`super::Ctx::step`]):
//!
//! - [`super::Step::MetalMakers`] every tick (SPEC §3.1 step 6): the
//!   "capacity short" history (§4.1), the maker switch (§4.9), and, every 15
//!   ticks (`ctx.due.sweep`), the defence attack watch (§4.8) and the
//!   commander-repair dispatch (§4.11). Builders belong to the economy, so
//!   the economy is the one that can send them to the commander.
//! - [`super::Step::Plan`] when `ctx.due.plan` (`cadence.plan_ticks`; §2.2,
//!   §3.1 step 7): housekeeping, factories topped up (§4.4), then **exactly
//!   one** idle builder given at most one job (§4.2). Planner jobs are staged
//!   at [`super::Priority::Tasked`], so the army's routine orders for the
//!   shared commander do not overwrite them.
//!
//! # Layout
//!
//! - `facts`: the §4.1 predicates, counts by role, affordability.
//! - `list`: the §4.3 priority list (rules 0-39, §4.13), rule E, loss
//!   promotion, the stranded list.
//! - `siting`: extractor sites (§4.5) and generic siting (§4.6).
//! - `defence`: rebuild records, the attack watch, fortification (§4.8),
//!   outposts (§4.7).
//! - `factory`: factory production (§4.4).
//! - `makers`: metal makers (§4.9).
//! - `reclaim`: field, own-wreck and harvest reclaim (§4.12).
//!
//! # Shared state
//!
//! Reads: `world.own`, `world.economy`, `world.contacts` / `world.enemy`,
//! `world.grid`, `world.map`, `world.reach`, `world.claims`, `world.threat`
//! (`base_under_attack`, `raided_outpost`, sectors), `world.army` (wave,
//! lean), `world.coord` (`commander_fleeing`, `sheltering`).
//!
//! Writes: `world.eco` (every field), `world.coord.site_guard` (§5.6),
//! `world.own.deliberate_removals`, `world.claims` (notes).
//!
//! # Numbers
//!
//! Every tuned number is a knob read from `ctx.knobs`. The few the spec
//! states as fixed (the 48 that takes a deposit, the 900 of an outpost, the
//! 1200 harvest radius, the 60 s heart wait, ...) are named constants next
//! to their use. No float is used anywhere.

mod defence;
mod factory;
mod facts;
mod geom;
mod list;
mod makers;
mod reclaim;
mod siting;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::{Ctx, GuardRequest, Manager, Priority, Role, Staging, Step};
use defence::{Defence, Outpost, Piece, Rebuild};
use factory::{Factories, Wants};
use facts::{
    affordable, affordable_new, dist, is_economy_type, is_extractor, is_tech_type, is_tower,
    is_water_structure, secs, tier_worth, CapacityWatch, Census, Facts,
};
use list::{ListCtx, Plans};
use makers::Makers;
use reclaim::Reclaim;
use siting::{Builder, MexFound, MexQuery, Site, S};

use crate::rng::AiRng;
use crate::view::{Command, DefId, FeatureId, Orders, Pos, Tick, UnitDef, UnitId};

/// Why a list entry was not taken (SPEC §4.2 "walking the list").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No site was found.
    NoSite,
    /// Not affordable, and not within its saving window.
    Unaffordable,
    /// Saving for another type: only economy types may be taken.
    Saving,
    /// The tower does not pay back (`def.payback_s`).
    NotWorth,
    /// The site is contested; its cell was marked failed.
    Contested,
    /// The commander leaves outpost towers to constructors.
    LeftToConstructors,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::NoSite => "no site",
            Refusal::Unaffordable => "not affordable",
            Refusal::Saving => "saving for another type",
            Refusal::NotWorth => "tower does not pay back",
            Refusal::Contested => "site contested",
            Refusal::LeftToConstructors => "left to constructors",
        })
    }
}

impl std::error::Error for Refusal {}

/// What a build is for: decides the tower-worth and contested tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Plain,
    Extractor,
    Rebuild(usize),
    Fort,
    Outpost(Outpost),
}

/// A job for one builder (SPEC §4.2 rule 3).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    Build {
        def: DefId,
        site: Site,
        purpose: Purpose,
    },
    Repair(UnitId),
    Reclaim(Vec<FeatureId>),
    Upgrade(Upgrade),
    Guard(UnitId),
}

/// A tier-two extractor upgrade in progress (SPEC §4.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Upgrade {
    builder: UnitId,
    old: UnitId,
    pos: Pos,
    def: DefId,
    since: Tick,
}

/// The plans of this pass, with their details.
#[derive(Clone, Copy, Debug, Default)]
struct Detail {
    rebuild: Option<Rebuild>,
    fort: Option<Piece>,
    outpost: Option<Outpost>,
    /// The base already has its light towers.
    base_towers_done: bool,
}

/// The economy and build manager (SPEC §4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomyManager {
    /// The round-robin cursor over idle builders (SPEC §4.2 step 2).
    cursor: Option<UnitId>,
    /// "Tier worth it", computed once.
    tier_worth: Option<bool>,
    capacity: CapacityWatch,
    makers: Makers,
    factories: Factories,
    defence: Defence,
    reclaim: Reclaim,
    /// Deposits waiting for their heart to clear (SPEC §4.5).
    heart: BTreeMap<usize, Tick>,
    /// The last build order given to each builder (dropped-order check).
    last_orders: BTreeMap<UnitId, (Pos, Tick)>,
    /// The one mending builder (SPEC §4.11).
    mender: Option<UnitId>,
    upgrade: Option<Upgrade>,
    /// The threat of the held build-site guard request (SPEC §5.6).
    guard_threat: i64,
    /// The last "expansion wanted" check (SPEC §5.9).
    expansion_checked: Option<Tick>,
}

impl Manager for EconomyManager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        match ctx.step {
            Step::MetalMakers => self.every_tick(ctx, out),
            Step::Plan => self.plan(ctx, out),
            _ => {}
        }
    }
}

/// Mobile units this close to a frame build it (SPEC §4.2 dropped orders).
const SITE_STANDS: i64 = 48;
/// "Expansion wanted" is checked at most this often (SPEC §5.9).
const EXPANSION_CHECK_S: i32 = 10;
/// A build-site guard is released once a building of ours stands this close.
const GUARD_DONE: i64 = 64;
/// Exposure counts contacts seen within this many ticks (SPEC §5.7).
const EXPOSURE_MEMORY: Tick = 150;
/// Buildings reach this far past their weapon range for exposure (§5.7).
const BUILDING_REACH: i64 = 32;
/// Mobile cover may be this far behind the position (§5.7).
const COVER_BEHIND: i64 = 64;

impl EconomyManager {
    /// The every-tick work (SPEC §3.1 step 6).
    fn every_tick(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let short = self
            .capacity
            .sample(ctx.knobs, &ctx.world.economy, ctx.tick);
        ctx.world.eco.capacity_short = short;
        self.makers.switch(ctx, out);
        if ctx.due.sweep {
            self.defence.watch(ctx);
            self.commander_repair(ctx, out);
        }
    }

    /// The commander-repair dispatch (SPEC §4.11), every 15 ticks.
    fn commander_repair(&mut self, ctx: &Ctx<'_>, out: &mut Staging) {
        let k = ctx.knobs;
        let w = &*ctx.world;
        if !k.build.repair_commander {
            return;
        }
        let Some(cmdr) = w.own.commander.and_then(|c| w.own.get(c)) else {
            return;
        };
        let Some(cd) = ctx.def(cmdr.unit.def) else {
            return;
        };
        if i64::from(cmdr.unit.health) * 100
            >= i64::from(cd.health) * i64::from(k.build.repair_commander_below_pct)
        {
            return;
        }
        let at = cmdr.unit.pos;
        if exposed(ctx, at) {
            return;
        }
        let on_it = |u: UnitId| repairing(ctx, u) == Some(cmdr.id());
        let n = w.own.builders.iter().filter(|&&u| on_it(u)).count();
        if n >= usize::try_from(k.build.commander_repairers.max(0)).unwrap_or(0) {
            return;
        }
        let r = dist(k.build.repair_commander_radius);
        let pick = w
            .own
            .builders
            .iter()
            .filter(|&&u| u != cmdr.id() && !on_it(u) && ctx.in_flight(u).is_none())
            .filter(|&&u| !w.coord.is_sheltering(u, ctx.tick))
            .filter(|&&u| Some(u) != self.reclaim.field_worker)
            .filter_map(|&u| w.own.get(u))
            .filter(|s| s.unit.pos.within(at, r))
            .min_by_key(|s| (s.unit.pos.dist2(at), s.id()));
        if let Some(b) = pick {
            out.set_priority(Priority::Tasked);
            out.now(b.id(), Command::Repair(cmdr.id()));
        }
    }

    /// One planning pass (SPEC §4.2).
    fn plan(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let tick = ctx.tick;
        if ctx.world.own.home.is_none() {
            return;
        }
        let tier = *self
            .tier_worth
            .get_or_insert_with(|| tier_worth(ctx.view, &ctx.world.roles, ctx.knobs));
        self.housekeeping(ctx);
        let facts = Facts::new(ctx, tier, ctx.world.eco.capacity_short);
        self.makers.plan_pass(&facts);

        // The field worker's recall.
        if let Some((u, cmd)) = self
            .reclaim
            .recall(ctx.view, ctx.leaks, ctx.knobs, &*ctx.world)
        {
            out.set_priority(Priority::Tasked);
            out.now(u, cmd);
        }

        let reserved = reserved(ctx);
        let vents = ctx.view.vents();
        let adv_lab = Census::new(ctx.world).has(Role::AdvancedKbotLab);
        let detail = {
            let s = S {
                view: ctx.view,
                leaks: ctx.leaks,
                knobs: ctx.knobs,
                w: &*ctx.world,
                facts: &facts,
                reserved: &reserved,
                vents: &vents,
                tick,
            };
            let base_llt = base_llt_count(&s);
            Detail {
                rebuild: self.defence.rebuild(&s),
                fort: self.defence.fortification(&s, adv_lab),
                outpost: self.defence.outpost(&s),
                base_towers_done: base_llt >= ctx.knobs.def.towers,
            }
        };

        // 1. Factories.
        let wants = Wants {
            walls_owed: detail.fort.is_some_and(|p| p.every_tower),
            sub_work: sub_work(ctx, &facts),
        };
        self.factories.top_up(ctx, out, &facts, wants);

        // 2. One idle builder, round robin.
        let idle = ctx.world.own.idle_builders.clone();
        let Some(&b) = idle
            .iter()
            .find(|&&u| self.cursor.is_none_or(|c| u > c))
            .or_else(|| idle.first())
        else {
            return;
        };
        self.cursor = Some(b);
        self.assign(ctx, out, &facts, detail, &reserved, &vents, b);
    }

    /// Lapsed marks, losses, the upgrade timeout, the guard release and
    /// "expansion wanted".
    fn housekeeping(&mut self, ctx: &mut Ctx<'_>) {
        let tick = ctx.tick;
        let k = ctx.knobs;
        ctx.world
            .eco
            .failed_cells
            .retain(|_, &mut until| tick < until);
        self.defence.update(ctx.view, k, &*ctx.world, tick);
        // The upgrade.
        if let Some(u) = self.upgrade {
            let builder_gone = !ctx.world.own.builders.contains(&u.builder);
            let timed_out = tick.saturating_sub(u.since) >= secs(k.eco.upgrade_timeout_s);
            let built = ctx
                .world
                .own
                .units
                .iter()
                .any(|s| s.unit.def == u.def && s.unit.pos.within(u.pos, SITE_STANDS));
            if builder_gone || timed_out || built {
                self.upgrade = None;
                ctx.world.eco.pending_upgrades.retain(|p| *p != u.pos);
            }
        }
        if self.upgrade.is_none() {
            ctx.world.eco.pending_upgrades.clear();
        }
        // The build-site guard.
        if let Some(g) = ctx.world.coord.site_guard {
            let dead = ctx.world.own.get(g.builder).is_none();
            let stands = ctx
                .world
                .own
                .units
                .iter()
                .any(|s| s.building && s.unit.pos.within(g.site, GUARD_DONE));
            let old = tick.saturating_sub(g.tick) >= secs(k.build.site_guard_timeout_s);
            if dead || stands || old {
                ctx.world.coord.site_guard = None;
                self.guard_threat = 0;
            }
        }
        // The dropped-order memory of builders that died.
        let builders: BTreeSet<UnitId> = ctx.world.own.builders.iter().copied().collect();
        self.last_orders.retain(|u, _| builders.contains(u));
        if self.mender.is_some_and(|m| !builders.contains(&m)) {
            self.mender = None;
        }
        // Expansion wanted (SPEC §5.9).
        let due = self
            .expansion_checked
            .is_none_or(|t| tick.saturating_sub(t) >= secs(EXPANSION_CHECK_S));
        if due {
            self.expansion_checked = Some(tick);
            ctx.world.eco.unreachable_expansion = unreachable_expansion(ctx);
        }
    }

    /// Gives builder `id` at most one job (SPEC §4.2 step 3).
    #[allow(clippy::too_many_arguments)]
    fn assign(
        &mut self,
        ctx: &mut Ctx<'_>,
        out: &mut Staging,
        f: &Facts,
        detail: Detail,
        reserved: &[(DefId, Pos)],
        vents: &[Pos],
        id: UnitId,
    ) {
        let tick = ctx.tick;
        let k = ctx.knobs;
        let Some(me) = ctx.world.own.get(id).cloned() else {
            return;
        };
        let Some(bdef) = ctx.def(me.unit.def).cloned() else {
            return;
        };
        // 1. A fleeing commander, or a builder in its shelter window.
        if (me.commander && ctx.world.coord.commander_fleeing)
            || ctx.world.coord.is_sheltering(id, tick)
        {
            return;
        }
        // 2. The dropped-order check.
        if let Some((site, at)) = self.last_orders.remove(&id) {
            let recent = tick.saturating_sub(at) <= secs(k.build.failed_site_s);
            let stands = ctx
                .world
                .own
                .units
                .iter()
                .any(|s| s.building && s.unit.pos.within(site, SITE_STANDS));
            if recent && !stands {
                ctx.world.eco.failed_cells.insert(
                    site.vis_cell(),
                    tick.saturating_add(secs(k.build.failed_site_s)),
                );
            }
        }
        // 3. Context.
        let flies = bdef.flies;
        let naval = !flies && !bdef.move_class.hovers && bdef.move_class.min_depth > 0;
        let cmd_layer = me.commander && k.cmd.own_reachability;
        let mut b = Builder {
            id,
            def: me.unit.def,
            pos: me.unit.pos,
            commander: me.commander,
            flies,
            naval,
            at_base: true,
            anchor: me.unit.pos,
            worker: bdef.worker_rate,
            cmd_layer,
        };
        b.at_base = flies || b.reachable(ctx.world, me.unit.pos);
        let home = f.home.unwrap_or(me.unit.pos);
        b.anchor = if b.at_base { home } else { me.unit.pos };

        let job = self.choose_job(ctx, f, detail, reserved, vents, &b, &bdef);
        let Some(job) = job else { return };
        self.issue(ctx, out, f, &b, job);
    }

    /// Rules 4-11 of SPEC §4.2.
    #[allow(clippy::too_many_arguments)]
    fn choose_job(
        &mut self,
        ctx: &mut Ctx<'_>,
        f: &Facts,
        detail: Detail,
        reserved: &[(DefId, Pos)],
        vents: &[Pos],
        b: &Builder,
        bdef: &UnitDef,
    ) -> Option<Job> {
        let k = ctx.knobs;
        // 4. Repair a damaged structure.
        if bdef.builder {
            if let Some(t) = repair_target(ctx, b) {
                return Some(Job::Repair(t));
            }
        }
        // 5. Resume an orphaned frame.
        if let Some(t) = orphan_frame(ctx, f, b) {
            return Some(Job::Repair(t));
        }
        // 6. Assist an advanced-lab frame.
        if let Some(t) = adv_lab_frame(ctx, b) {
            return Some(Job::Repair(t));
        }
        // 7. Mend a damaged mobile unit.
        if let Some(t) = self.mend_target(ctx, b) {
            self.mender = Some(b.id);
            return Some(Job::Repair(t));
        }
        // 8. Field reclaim, then own-wreck reclaim.
        let field = self.reclaim.field(ctx.view, ctx.leaks, k, &*ctx.world, b);
        if !field.is_empty() {
            return Some(Job::Reclaim(field));
        }
        let flying: Vec<crate::view::Order> = ctx.in_flight.all(ctx.tick).copied().collect();
        let claimed = reclaim::claimed_features(ctx.world, &flying);
        let own =
            self.reclaim
                .own_wrecks(ctx.view, ctx.leaks, k, &*ctx.world, f, b, &claimed, |p| {
                    reclaim_fit(ctx, b, p)
                });
        if !own.is_empty() {
            return Some(Job::Reclaim(own));
        }
        // 9. An extractor upgrade this builder owns.
        if let Some(u) = self.upgrade.filter(|u| u.builder == b.id) {
            return Some(Job::Upgrade(u));
        }
        // 10. Walk the priority list.
        let (job, saving, fails) = self.walk(ctx, f, detail, reserved, vents, b, bdef);
        for p in fails {
            ctx.world.eco.failed_cells.insert(
                p.vis_cell(),
                ctx.tick.saturating_add(secs(k.build.failed_site_s)),
            );
        }
        ctx.world.eco.saving_for = saving;
        if job.is_some() {
            return job;
        }
        // 11. Nothing taken.
        let saving = ctx.world.eco.saving_for.is_some();
        if b.commander && !saving {
            if let Some(t) = commander_assist(ctx, b) {
                return Some(Job::Repair(t));
            }
        }
        if let Some(x) = reclaim::harvest(ctx.view, ctx.leaks, k, ctx.world, f, b, |p| {
            reclaim_fit(ctx, b, p)
        }) {
            return Some(Job::Reclaim(vec![x]));
        }
        if !saving {
            if let Some(t) = factory_assist(ctx, b) {
                return Some(Job::Guard(t));
            }
        }
        None
    }

    /// Walks the priority list (SPEC §4.2 rule 10). Returns the job, the new
    /// saving state and the cells to mark failed.
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn walk(
        &mut self,
        ctx: &mut Ctx<'_>,
        f: &Facts,
        detail: Detail,
        reserved: &[(DefId, Pos)],
        vents: &[Pos],
        b: &Builder,
        bdef: &UnitDef,
    ) -> (Option<Job>, Option<DefId>, Vec<Pos>) {
        let k = ctx.knobs;
        let s = S {
            view: ctx.view,
            leaks: ctx.leaks,
            knobs: k,
            w: &*ctx.world,
            facts: f,
            reserved,
            vents,
            tick: ctx.tick,
        };
        let rng: &mut AiRng = &mut *ctx.rng;
        let c = Census::new(s.w);
        let lower = i32::try_from(
            s.w.own
                .builders
                .iter()
                .filter(|&&u| u < b.id)
                .filter_map(|&u| s.w.own.get(u))
                .filter(|u| c.is(u.unit.def, Role::Constructor))
                .count(),
        )
        .unwrap_or(0);
        let lost: Vec<DefId> = s.w.own.building_losses.iter().map(|l| l.def).collect();
        let plans = Plans {
            rebuild: detail.rebuild.map(|r| r.def),
            fort: detail.fort.map(|p| p.def),
            outpost: detail.outpost.is_some(),
            outpost_raided: detail.outpost.is_some_and(|o| o.raided),
            maker_ceiling: self.makers.ceiling(k, f),
            free_deposit: f.free_deposits > 0,
            outlying: list::outlying(&c, f.home),
        };
        let t2 = t2_without_store(&s);
        let x = ListCtx {
            k,
            c,
            f,
            p: plans,
            b,
            bdef,
            lower_constructors: lower,
            lost: &lost,
            vents: i32::try_from(vents.len()).unwrap_or(i32::MAX),
            t2_factory_without_store: t2,
        };
        let wants = list::build(&x);
        let mut saving = s.w.eco.saving_for.filter(|d| wants.contains(d));
        let mut fails = Vec::new();
        let eco = &s.w.economy;
        for def in wants {
            let Some(d) = s.def(def) else { continue };
            let econ = is_economy_type(&c, def, f);
            if saving.is_some_and(|sv| sv != def) && !econ {
                continue;
            }
            if !econ && !affordable_new(eco, d, b.worker, 0) {
                let window = if is_tech_type(&c, def, f) {
                    k.eco.save_tech_s
                } else {
                    k.eco.save_s
                };
                if saving.is_none() && affordable_new(eco, d, b.worker, i64::from(window)) {
                    saving = Some(def);
                }
                continue;
            }
            let found = site_for(&s, rng, &mut self.heart, b, def, d, &detail);
            let (site, purpose) = match found {
                Ok(x) => x,
                Err(_) => {
                    saving = unsaved_without_site(saving, def);
                    if c.is(def, Role::Mex) {
                        if let Some(u) = self.upgrade_fallback(&s, b) {
                            return (Some(Job::Upgrade(u)), None, fails);
                        }
                    }
                    continue;
                }
            };
            if let Err(why) = take(&s, b, def, d, site, purpose) {
                if why == Refusal::Contested {
                    fails.push(site.at);
                }
                continue;
            }
            // Issuing the job clears saving (SPEC §4.2).
            return (Some(Job::Build { def, site, purpose }), None, fails);
        }
        (None, saving, fails)
    }

    /// The upgrade fallback (SPEC §4.5): with no free patch, take down the
    /// nearest own extractor and build the tier-two one on its spot.
    fn upgrade_fallback(&self, s: &S<'_>, b: &Builder) -> Option<Upgrade> {
        let k = s.knobs;
        let c = s.census();
        let moho = c.def(Role::Moho)?;
        let mex = c.def(Role::Mex)?;
        let md = s.def(moho)?;
        if !k.eco.upgrade_mex
            || !b.at_base
            || self.upgrade.is_some()
            || s.facts.energy_stalled
            || !b.can_build_def(s, moho)
        {
            return None;
        }
        let stock = s.w.economy.raw.metal.stock.milli();
        if stock < k.eco.upgrade_min_metal_frac.apply(md.cost_metal.milli()) {
            return None;
        }
        if s.w
            .own
            .units
            .iter()
            .any(|u| u.unit.def == moho && u.is_frame())
        {
            return None;
        }
        let old =
            s.w.own
                .units
                .iter()
                .filter(|u| u.unit.def == mex && !u.is_frame())
                .filter(|u| !siting::under_guns(s.view, k, s.w, u.unit.pos))
                .filter(|u| b.on_side(s.w, u.unit.pos))
                .filter(|u| {
                    !s.leaks
                        .leak_buildings_near(u.unit.pos, UPGRADE_CROWD)
                        .iter()
                        .any(|o| o.id != u.id())
                })
                .min_by_key(|u| (u.unit.pos.dist2(b.pos), u.id()))?;
        Some(Upgrade {
            builder: b.id,
            old: old.id(),
            pos: old.unit.pos,
            def: moho,
            since: s.tick,
        })
    }

    /// The most damaged own mobile unit to mend (SPEC §4.11).
    fn mend_target(&self, ctx: &Ctx<'_>, b: &Builder) -> Option<UnitId> {
        let k = ctx.knobs;
        let w = &*ctx.world;
        if !k.build.mend_units || (b.commander && !k.build.commander_mends) {
            return None;
        }
        if let Some(m) = self.mender.filter(|&m| m != b.id) {
            if repairing(ctx, m).is_some() {
                return None;
            }
        }
        let home = w.own.home?;
        let r = dist(k.build.mend_radius);
        let pct = i64::from(k.build.mend_below_pct);
        w.own
            .units
            .iter()
            .filter(|s| !s.building && !s.is_frame() && s.id() != b.id && !s.unit.scripted)
            .filter(|s| s.unit.carried_by.is_none() && s.unit.pos.within(home, r))
            .filter_map(|s| {
                let max = i64::from(ctx.def(s.unit.def)?.health).max(1);
                let hp = i64::from(s.unit.health);
                (hp * 100 < max * pct).then_some((hp * 1000 / max, s.id()))
            })
            .min()
            .map(|(_, id)| id)
    }

    /// Stages a job (first order Immediate, the rest Queued) and does its
    /// bookkeeping.
    fn issue(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging, f: &Facts, b: &Builder, job: Job) {
        let tick = ctx.tick;
        let cmds: Vec<Command> = match &job {
            Job::Build { def, site, purpose } => {
                let mut v = Vec::new();
                if let Some(w) = site.wreck {
                    v.push(Command::ReclaimFeature(w));
                }
                v.push(Command::Build {
                    def: *def,
                    at: site.at,
                });
                if is_extractor(&Census::new(ctx.world), *def) {
                    ctx.world.claims.note(b.id, site.at, *def, tick);
                }
                if let Purpose::Rebuild(i) = purpose {
                    self.defence.rebuilt(*i);
                }
                self.last_orders.insert(b.id, (site.at, tick));
                self.guard_request(ctx, f, b, site.at);
                v
            }
            Job::Repair(t) => vec![Command::Repair(*t)],
            Job::Reclaim(fs) => fs.iter().map(|&x| Command::ReclaimFeature(x)).collect(),
            Job::Guard(t) => vec![Command::Guard(*t)],
            Job::Upgrade(u) => {
                self.upgrade = Some(*u);
                if !ctx.world.eco.pending_upgrades.contains(&u.pos) {
                    ctx.world.eco.pending_upgrades.push(u.pos);
                }
                ctx.world.claims.note(b.id, u.pos, u.def, tick);
                let mut v = Vec::new();
                if ctx.world.own.get(u.old).is_some() {
                    ctx.world.own.deliberate_removals.insert(u.old);
                    v.push(Command::ReclaimUnit(u.old));
                }
                v.push(Command::Build {
                    def: u.def,
                    at: u.pos,
                });
                self.last_orders.insert(b.id, (u.pos, tick));
                v
            }
        };
        out.set_priority(Priority::Tasked);
        for (i, c) in cmds.into_iter().enumerate() {
            if i == 0 {
                out.now(b.id, c);
            } else {
                out.queued(b.id, c);
            }
        }
    }

    /// The build-site guard request (SPEC §5.6).
    fn guard_request(&mut self, ctx: &mut Ctx<'_>, f: &Facts, b: &Builder, site: Pos) {
        let k = ctx.knobs;
        if k.build.site_guard_size <= 0 {
            return;
        }
        let Some(home) = f.home else { return };
        if site.within(
            home,
            dist(k.build.site_guard_min_distance).saturating_sub(1),
        ) {
            return;
        }
        let r = i32::try_from(dist(k.build.site_guard_threat_radius)).unwrap_or(i32::MAX);
        let threat = ctx.world.grid.threat_in_radius(site, r);
        // The knob is in whole dps; the grid in milli-dps.
        if threat < k.build.site_guard_threat.apply(1000) {
            return;
        }
        if ctx.world.coord.site_guard.is_some() && threat < self.guard_threat {
            return;
        }
        self.guard_threat = threat;
        ctx.world.coord.site_guard = Some(GuardRequest {
            site,
            builder: b.id,
            tick: ctx.tick,
        });
    }
}

/// Sites within this of another building block an upgrade (SPEC §4.5).
const UPGRADE_CROWD: i32 = 72;

impl Builder {
    fn can_build_def(&self, s: &S<'_>, d: DefId) -> bool {
        s.def(self.def).is_some_and(|bd| bd.can_build(d))
    }
}

/// The tests a found site must still pass (SPEC §4.2 "walking the list").
fn take(
    s: &S<'_>,
    b: &Builder,
    def: DefId,
    d: &UnitDef,
    site: Site,
    purpose: Purpose,
) -> Result<(), Refusal> {
    let k = s.knobs;
    let c = s.census();
    let f = s.facts;
    // The commander leaves outpost towers to constructors.
    if let Purpose::Outpost(_) = purpose {
        if b.commander && k.def.outpost_towers_to_constructors {
            let other = s.w.own.builders.iter().any(|&u| {
                u != b.id
                    && s.w
                        .own
                        .get(u)
                        .and_then(|x| s.def(x.unit.def))
                        .is_some_and(|x| x.can_build(def))
            });
            if other {
                return Err(Refusal::LeftToConstructors);
            }
        }
    }
    // Tower worth.
    if is_tower(&c, def) {
        let covered = match purpose {
            Purpose::Outpost(o) => i64::from(o.covered),
            _ => 0,
        };
        let waived = f.urgent()
            || matches!(purpose, Purpose::Fort | Purpose::Rebuild(_))
            || matches!(purpose, Purpose::Outpost(o) if o.raided)
            || k.def.payback_s <= 0
            || f.income <= 0;
        let secs = i64::from(k.def.payback_s) + covered * i64::from(k.def.payback_per_mex_s);
        if !waived && d.cost_metal.milli() > f.income.saturating_mul(1000).saturating_mul(secs) {
            return Err(Refusal::NotWorth);
        }
    }
    // Contested site (any type but extractors).
    if !is_extractor(&c, def) {
        let r = dist(k.build.contested_radius);
        let mut contested = siting::armed_ground_within(s.w, site.at, r);
        if !contested && !matches!(purpose, Purpose::Rebuild(_)) {
            if let Some(home) = f.home {
                let far = !site.at.within(home, dist(k.army.defend_radius));
                let loss =
                    s.w.own
                        .building_losses
                        .iter()
                        .chain(&s.w.own.unit_losses)
                        .any(|l| l.pos.within(site.at, r));
                contested = far && loss;
            }
        }
        if contested {
            return Err(Refusal::Contested);
        }
    }
    Ok(())
}

/// Site rules by type, first match (SPEC §4.6).
fn site_for(
    s: &S<'_>,
    rng: &mut AiRng,
    heart: &mut BTreeMap<usize, Tick>,
    b: &Builder,
    def: DefId,
    d: &UnitDef,
    detail: &Detail,
) -> Result<(Site, Purpose), Refusal> {
    let k = s.knobs;
    let c = s.census();
    let home = s.facts.home.ok_or(Refusal::NoSite)?;
    let some =
        |p: Option<Pos>, purpose: Purpose| p.map(|p| (Site::at(p), purpose)).ok_or(Refusal::NoSite);
    if is_extractor(&c, def) {
        return mex_site(s, rng, heart, b, def).map(|site| (site, Purpose::Extractor));
    }
    if let Some(r) = detail.rebuild.filter(|r| r.def == def) {
        return match siting::spot(s, def, d, r.pos) {
            siting::Spot::Legal => Ok((Site::at(r.pos), Purpose::Rebuild(r.index))),
            siting::Spot::Wreck(w) => Ok((
                Site {
                    at: r.pos,
                    wreck: Some(w),
                },
                Purpose::Rebuild(r.index),
            )),
            _ => Err(Refusal::NoSite),
        };
    }
    if let Some(p) = detail.fort.filter(|p| p.def == def) {
        if p.exact {
            return Ok((Site::at(p.at), Purpose::Fort));
        }
        return some(
            siting::ring_site(s, rng, b, def, d, p.at, false),
            Purpose::Fort,
        );
    }
    let llt = c.is(def, Role::LightLaserTower);
    if let Some(o) = detail.outpost.filter(|_| llt) {
        if o.raided || detail.base_towers_done {
            return some(
                siting::ring_site(s, rng, b, def, d, o.anchor, false),
                Purpose::Outpost(o),
            );
        }
    }
    let aa = c.is(def, Role::AntiAirTower);
    if (llt || aa) && b.at_base && k.def.spread {
        return some(siting::defence_site(s, rng, def, d, aa), Purpose::Plain);
    }
    if c.is(def, Role::Radar) && b.at_base {
        return some(siting::radar_site(s, rng, b, def, d), Purpose::Plain);
    }
    if llt {
        let face = siting::facing(s);
        let anchor = geom::along(home, face, dist(k.def.distance));
        return some(
            siting::ring_site(s, rng, b, def, d, anchor, false),
            Purpose::Plain,
        );
    }
    if c.is(def, Role::Geothermal) {
        return some(siting::geothermal_site(s, b, def), Purpose::Plain);
    }
    if c.is(def, Role::TorpedoLauncher) {
        return some(siting::launcher_site(s, rng, b, def, d), Purpose::Plain);
    }
    if is_water_structure(&c, def) {
        return some(
            siting::ring_site(s, rng, b, def, d, b.anchor, true),
            Purpose::Plain,
        );
    }
    if c.is(def, Role::Shipyard)
        || c.is(def, Role::AdvancedShipyard)
        || c.is(def, Role::SeaplanePlant)
    {
        return some(siting::shipyard_site(s, rng, def), Purpose::Plain);
    }
    if c.is(def, Role::Solar) && b.at_base && k.eco.energy_rows {
        return some(siting::energy_rows(s, rng, b, def, d), Purpose::Plain);
    }
    some(
        siting::ring_site(s, rng, b, def, d, b.anchor, false),
        Purpose::Plain,
    )
}

/// The extractor search order of SPEC §4.5: near search, expansion search
/// (with the boxed retry), then the wet preference.
fn mex_site(
    s: &S<'_>,
    rng: &mut AiRng,
    heart: &mut BTreeMap<usize, Tick>,
    b: &Builder,
    def: DefId,
) -> Result<Site, Refusal> {
    let k = s.knobs;
    let c = s.census();
    let home = s.facts.home.ok_or(Refusal::NoSite)?;
    let constructors = s.w.own.builders.iter().any(|&u| {
        s.w.own
            .get(u)
            .is_some_and(|x| !x.commander && c.is(x.unit.def, Role::Constructor))
    });
    let leash = b.commander.then(|| {
        let r = if k.build.commander_near_sites && constructors {
            dist(k.build.commander_leash)
        } else {
            dist(k.build.commander_mex_radius)
        };
        (home, r)
    });
    let near = MexQuery {
        anchor: b.pos,
        radius: dist(k.build.near_mex_radius),
        our_side: false,
        explored: false,
        leash,
    };
    let mut found = siting::extractor_site(s, rng, heart, b, def, near);
    if matches!(found, MexFound::None { .. }) && b.at_base {
        let radius = if b.commander {
            dist(k.build.commander_mex_radius)
        } else {
            dist(k.build.expand_radius)
        };
        let q = MexQuery {
            anchor: home,
            radius,
            our_side: k.build.expand_our_side,
            explored: k.build.expand_explored_only,
            leash,
        };
        found = siting::extractor_site(s, rng, heart, b, def, q);
        if let MexFound::None { boxed: true } = found {
            if k.build.expand_middle_if_boxed {
                found = siting::extractor_site(
                    s,
                    rng,
                    heart,
                    b,
                    def,
                    MexQuery {
                        our_side: false,
                        ..q
                    },
                );
            }
        }
    }
    let MexFound::Site(site) = found else {
        return Err(Refusal::NoSite);
    };
    // The wet preference: a nearer submerged patch goes to the underwater
    // extractor later in the list.
    if c.is(def, Role::Mex)
        && c.def(Role::UnderwaterMex)
            .is_some_and(|u| b.can_build_def(s, u))
    {
        let d_dry = site.at.dist2(b.anchor);
        let wet_nearer = s.w.map.deposits.iter().any(|dep| {
            s.view.water_depth(dep.center.tile()) > 0
                && dep.center.dist2(b.anchor) < d_dry
                && !facts::extractor_near_world(s.view, s.w, dep.center, facts::EXTRACTOR_TAKEN)
        });
        if wet_nearer {
            return Err(Refusal::NoSite);
        }
    }
    Ok(site)
}

/// Pending builds of our builders: queues and in-flight orders.
fn reserved(ctx: &Ctx<'_>) -> Vec<(DefId, Pos)> {
    let mut v: Vec<(DefId, Pos)> = Vec::new();
    for s in &ctx.world.own.units {
        for c in &s.unit.commands {
            if let Command::Build { def, at } = *c {
                v.push((def, at));
            }
        }
    }
    for o in ctx.in_flight.all(ctx.tick) {
        if let Command::Build { def, at } = o.command {
            v.push((def, at));
        }
    }
    v
}

fn base_llt_count(s: &S<'_>) -> i32 {
    let c = s.census();
    let Some(llt) = c.def(Role::LightLaserTower) else {
        return 0;
    };
    let home = s.facts.home.unwrap_or_default();
    let out =
        s.w.own
            .units
            .iter()
            .filter(|u| u.unit.def == llt && !u.unit.pos.within(home, defence::OUTPOST_DISTANCE))
            .count();
    c.tot(Role::LightLaserTower) - i32::try_from(out).unwrap_or(0)
}

/// A tier-two factory stands and no metal (energy) store exists yet
/// (SPEC §4.3 rule 34).
fn t2_without_store(s: &S<'_>) -> (bool, bool) {
    let c = s.census();
    let t2 = c.has(Role::AdvancedKbotLab)
        || c.has(Role::AdvancedAirPlant)
        || c.has(Role::AdvancedShipyard);
    let ms = c.tot(Role::MetalStorage) + c.tot(Role::UnderwaterMetalStorage);
    let es = c.tot(Role::EnergyStorage) + c.tot(Role::UnderwaterEnergyStorage);
    (t2 && ms == 0, t2 && es == 0)
}

/// A seaplane plant or underwater fusion is still wanted (SPEC §4.4).
fn sub_work(ctx: &Ctx<'_>, f: &Facts) -> bool {
    let c = Census::new(ctx.world);
    let k = ctx.knobs;
    let seaplane = c.def(Role::SeaplanePlant).is_some()
        && c.tot(Role::SeaplanePlant) < k.navy.seaplane_plants
        && c.tot(Role::AirPlant) == 0;
    let fusion = c.def(Role::UnderwaterFusion).is_some()
        && c.tot(Role::UnderwaterFusion) < k.eco.uw_fusion
        && f.income >= i64::from(k.navy.tech_income);
    seaplane || fusion
}

/// The unit a builder is repairing (its first order), if any.
fn repairing(ctx: &Ctx<'_>, u: UnitId) -> Option<UnitId> {
    if let Some(Command::Repair(t)) = ctx.in_flight(u).and_then(|o| o.first()).map(|o| o.command) {
        return Some(t);
    }
    match ctx.world.own.get(u)?.unit.commands.first() {
        Some(Command::Repair(t)) => Some(*t),
        _ => None,
    }
}

/// A damaged structure to repair (SPEC §4.11): the nearest defence, else
/// factory, below `build.repair_below_pct` within `build.repair_radius`.
fn repair_target(ctx: &Ctx<'_>, b: &Builder) -> Option<UnitId> {
    let k = ctx.knobs;
    let w = &*ctx.world;
    if !k.build.repair {
        return None;
    }
    let r = dist(k.build.repair_radius);
    let pct = i64::from(k.build.repair_below_pct);
    let cap = usize::try_from(k.build.repairers.max(0)).unwrap_or(0);
    let mut best: Option<(bool, i64, UnitId)> = None;
    for s in &w.own.units {
        if !s.building || s.is_frame() || !s.unit.pos.within(b.pos, r) {
            continue;
        }
        let Some(d) = ctx.def(s.unit.def) else {
            continue;
        };
        let defence = d.armed();
        if !defence && !d.is_factory() {
            continue;
        }
        if i64::from(s.unit.health) * 100 >= i64::from(d.health) * pct {
            continue;
        }
        if !b.on_side(w, s.unit.pos) {
            continue;
        }
        if !k.build.repair_under_fire && siting::under_guns(ctx.view, k, w, s.unit.pos) {
            continue;
        }
        let held = w
            .own
            .builders
            .iter()
            .filter(|&&u| repairing(ctx, u) == Some(s.id()))
            .count();
        if held >= cap {
            continue;
        }
        let key = (!defence, s.unit.pos.dist2(b.pos), s.id());
        if best.is_none_or(|bb| key < bb) {
            best = Some(key);
        }
    }
    best.map(|(_, _, id)| id)
}

/// An orphaned frame to resume (SPEC §4.2 rule 5).
fn orphan_frame(ctx: &Ctx<'_>, f: &Facts, b: &Builder) -> Option<UnitId> {
    let w = &*ctx.world;
    let c = Census::new(w);
    w.own
        .orphan_frames
        .iter()
        .filter_map(|&id| w.own.get(id))
        .filter(|s| b.on_side(w, s.unit.pos) && !exposed(ctx, s.unit.pos))
        .filter(|s| {
            let def = s.unit.def;
            if is_economy_type(&c, def, f) {
                return true;
            }
            let (Some(d), Some(p)) = (ctx.def(def), s.unit.frame) else {
                return false;
            };
            let left = (p.total - p.done).max(0);
            let owed =
                crate::view::Amount(d.cost_metal.milli().saturating_mul(left) / p.total.max(1));
            affordable(&w.economy, owed, left, b.worker, 0, true)
        })
        .min_by_key(|s| (s.unit.pos.dist2(b.pos), s.id()))
        .map(|s| s.id())
}

/// An advanced-lab frame to assist (SPEC §4.2 rule 6).
fn adv_lab_frame(ctx: &Ctx<'_>, b: &Builder) -> Option<UnitId> {
    let w = &*ctx.world;
    let lab = w.roles.get(Role::AdvancedKbotLab)?;
    w.own
        .units
        .iter()
        .filter(|s| s.unit.def == lab && s.is_frame() && b.on_side(w, s.unit.pos))
        .min_by_key(|s| (s.unit.pos.dist2(b.pos), s.id()))
        .map(|s| s.id())
}

/// The commander assists the nearest own frame (SPEC §4.2 rule 11).
fn commander_assist(ctx: &Ctx<'_>, b: &Builder) -> Option<UnitId> {
    let k = ctx.knobs;
    let w = &*ctx.world;
    let home = w.own.home?;
    let r = dist(k.build.commander_assist_radius);
    let leash = dist(k.build.commander_leash);
    w.own
        .frames
        .iter()
        .filter_map(|&id| w.own.get(id))
        .filter(|s| s.unit.pos.within(b.pos, r) && s.unit.pos.within(home, leash))
        .filter(|s| b.on_side(w, s.unit.pos))
        .min_by_key(|s| (s.unit.pos.dist2(b.pos), s.id()))
        .map(|s| s.id())
}

/// The nearest factory with a non-empty queue of the builder's nature.
fn factory_assist(ctx: &Ctx<'_>, b: &Builder) -> Option<UnitId> {
    let w = &*ctx.world;
    w.own
        .factories
        .iter()
        .filter_map(|&id| w.own.get(id))
        .filter(|s| s.unit.factory.as_ref().is_some_and(|q| !q.queue.is_empty()))
        .filter(|s| {
            let water = ctx
                .def(s.unit.def)
                .is_some_and(|d| d.move_class.min_depth > 0);
            b.flies || water == b.naval
        })
        .min_by_key(|s| (s.unit.pos.dist2(b.pos), s.id()))
        .map(|s| s.id())
}

/// The saving after `def`, now affordable, found no site (SPEC §4.2).
///
/// Saving waits for money, not for a site, so it starts whether or not a
/// site exists. But once the saved-for item is affordable and still has
/// nowhere to go, holding the saving would block every other type for
/// good: metal full and every builder idle. So it ends there, and starts
/// again if the item is ever unaffordable within its window.
fn unsaved_without_site(saving: Option<DefId>, def: DefId) -> Option<DefId> {
    saving.filter(|&s| s != def)
}

/// SPEC §5.7 exposure: remembered armed non-air contacts seen lately whose
/// reach covers `p`, against our armed finished non-air units covering it.
fn exposed(ctx: &Ctx<'_>, p: Pos) -> bool {
    let k = ctx.knobs;
    let w = &*ctx.world;
    let margin = dist(k.safety.margin);
    let memory = u32::try_from(k.safety.contact_memory_ticks).unwrap_or(EXPOSURE_MEMORY);
    let mut threat = 0i64;
    let mut centre = (0i64, 0i64, 0i64);
    for c in w.contacts.iter() {
        if !c.armed || c.air || c.frame || ctx.tick.saturating_sub(c.last_seen) > memory {
            continue;
        }
        let range = ctx.def(c.def).map_or(0, |d| i64::from(d.max_range()));
        let reach = range + if c.building { BUILDING_REACH } else { margin };
        if c.pos.within(p, reach) {
            threat += c.metal.whole();
            centre.0 += i64::from(c.pos.x);
            centre.1 += i64::from(c.pos.z);
            centre.2 += 1;
        }
    }
    if threat == 0 {
        return false;
    }
    let tc = Pos::new(
        i32::try_from(centre.0 / centre.2.max(1)).unwrap_or(p.x),
        i32::try_from(centre.1 / centre.2.max(1)).unwrap_or(p.z),
    );
    let to_threat = geom::dir(p, tc);
    let cover_r = dist(k.safety.cover_radius);
    let mut cover = 0i64;
    for s in &w.own.units {
        if s.is_frame() {
            continue;
        }
        let Some(d) = ctx.def(s.unit.def) else {
            continue;
        };
        if !d.armed() || d.flies {
            continue;
        }
        let metal = if s.commander {
            i64::from(k.cmd.fight_max_metal)
        } else {
            d.cost_metal.whole()
        };
        let at = s.unit.pos;
        let covers = if s.building {
            let r = i64::from(d.max_range());
            at.within(tc, r + BUILDING_REACH) || at.within(p, r)
        } else {
            at.within(p, cover_r) && geom::forward(p, to_threat, at) >= -COVER_BEHIND
        };
        if covers {
            cover += metal;
        }
    }
    // cover < threat × ratio, ratio in ten-thousandths.
    i128::from(cover) * 10_000 < i128::from(threat) * i128::from(k.safety.cover_ratio.raw())
}

/// SPEC §5.9 "expansion wanted": an extractor site on walkable ground that
/// home cannot reach (no radius limit), nearest home first. The site must
/// also be a free deposit on our side (§4.5): no extractor within 48 (ours
/// or a remembered enemy one), not under enemy guns, and with
/// `build.expand_our_side` no closer to the enemy base than to ours. On a
/// map split by water the far shore is the enemy's; ferrying builders to
/// its extractors only fed them to its defences.
fn unreachable_expansion(ctx: &Ctx<'_>) -> Option<Pos> {
    let w = &*ctx.world;
    let home = w.own.home?;
    let ground = w.reach.ground.as_ref()?;
    let mover = ctx.def(ground.mover)?;
    let enemy = w.enemy_base();
    let mut deps: Vec<_> = w.map.deposits.iter().collect();
    deps.sort_by_key(|d| (d.center.dist2(home), d.id));
    deps.into_iter()
        .find(|d| {
            let t = ground.origin(d.center);
            ctx.view.standable(t, mover.move_class)
                && !w.reach.ground_reachable(d.center)
                && (!ctx.knobs.build.expand_our_side || facts::our_side(d.center, home, enemy))
                && !facts::extractor_near_world(ctx.view, w, d.center, facts::EXTRACTOR_TAKEN)
                && !siting::under_guns(ctx.view, ctx.knobs, w, d.center)
        })
        .map(|d| d.center)
}

/// Whether builder `b` may reclaim a feature at `p` (§4.2 rule 8 and 11):
/// it must be reachable on the builder's layer (§4.2 context). The
/// commander also keeps to the ground our land builders reach and to
/// spots that are not exposed (§5.7): an amphibious commander would
/// otherwise wade across a strait to a wreck under the enemy's guns.
fn reclaim_fit(ctx: &Ctx<'_>, b: &Builder, p: Pos) -> bool {
    b.reachable(ctx.world, p)
        && (!b.commander || (ctx.world.reach.ground_reachable(p) && !exposed(ctx, p)))
}
