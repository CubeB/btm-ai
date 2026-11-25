//! The tactics pass (SPEC §3.1 step 10): the rally point, waves, reserve and
//! objective (§5.2), defence and proportional response, the radar line and
//! outpost raids (§5.4), raids, guards and anti-air escorts (§5.6), damaged
//! units (§5.11), and each unit's duty, first match (§5.12). With no home
//! point there is no rally, no defence line, no mending and no outpost
//! response (§5.13).

use std::collections::{BTreeMap, BTreeSet};

use super::geom::{centroid, inside, on_bearing, sidestep};
use super::{
    at_least, dist, health_pct, metal_of, order, secs, without, ArmyManager, COMMANDER_GUARD_SLACK,
    MAP_MARGIN, MEND_SPOTS, RADAR_LINE_DISTANCE, RADAR_LINE_PITCH, RADAR_LINE_SLACK,
    RADAR_LINE_SLOTS, RAID_BUILDER_RADIUS, RAID_RELEASE_RADIUS, RALLY_SLACK, RESERVE_JOINS_AT,
    SITE_GUARD_DONE, SITE_GUARD_STAND,
};
use crate::brain::roles::Role;
use crate::brain::{Ctx, Phase, Staging, Standing};
use crate::view::{Amount, Command, Pos, Tick, UnitId};

/// A unit sent home to be mended (§5.11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mend {
    since: Tick,
    /// Back on duty (timed out): not sent again until healed.
    done: bool,
}

/// What the tactics pass remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TacticsState {
    /// Each wave member's health at the last pass (§5.2 contact).
    pub(crate) wave_health: BTreeMap<UnitId, i32>,
    /// The raiding party, in id order (§5.6).
    pub(crate) raid: Vec<UnitId>,
    pub(crate) raid_since: Tick,
    pub(crate) raid_released: bool,
    /// The build-site guard group, and the request it last released.
    pub(crate) guard: Vec<UnitId>,
    pub(crate) guard_done: Option<Tick>,
    /// Units mending, by id.
    pub(crate) mending: BTreeMap<UnitId, Mend>,
}

/// One pass's decisions, shared by every unit's duty.
#[derive(Clone, Debug, Default)]
struct Plan {
    phase: Phase,
    rally: Option<Pos>,
    objective: Option<Pos>,
    wave: BTreeSet<UnitId>,
    reserve: BTreeSet<UnitId>,
    raid: BTreeSet<UnitId>,
    raid_go: bool,
    raid_target: Option<UnitId>,
    guard: BTreeSet<UnitId>,
    guard_site: Option<Pos>,
    target: Option<UnitId>,
    responders: BTreeSet<UnitId>,
    holding: bool,
    outpost: Option<(Pos, BTreeSet<UnitId>, Vec<UnitId>)>,
    radar: Option<(Pos, Pos)>,
    commander: Option<(Pos, Vec<UnitId>)>,
}

impl ArmyManager {
    pub(crate) fn tactics(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let alive: BTreeSet<UnitId> = ctx.world.own.combat.iter().copied().collect();
        let living = |u: UnitId| alive.contains(&u);
        ctx.world.army.wave.retain(|&u| living(u));
        ctx.world.army.reserve.retain(|&u| living(u));
        self.tac.raid.retain(|&u| living(u));
        self.tac.guard.retain(|&u| living(u));
        super::prune(&mut self.tac.mending, living);
        super::prune(&mut self.tac.wave_health, living);
        self.micro.prune(ctx, &alive);

        self.counter_lean(ctx);
        let rally = rally_point(ctx);
        ctx.world.army.rally = rally;
        self.site_guard(ctx);
        self.wave_upkeep(ctx);
        let objective = objective(ctx);
        ctx.world.army.objective = objective;
        self.raid_upkeep(ctx);

        let mut plan = Plan {
            phase: ctx.world.army.phase,
            rally,
            objective,
            wave: ctx.world.army.wave.iter().copied().collect(),
            reserve: ctx.world.army.reserve.iter().copied().collect(),
            raid: self.tac.raid.iter().copied().collect(),
            raid_go: self.tac.raid_released,
            raid_target: None,
            guard: self.tac.guard.iter().copied().collect(),
            guard_site: ctx.world.coord.site_guard.map(|r| r.site),
            ..Plan::default()
        };
        if !plan.raid.is_empty() {
            plan.raid_target = raid_target(ctx);
        }
        defence(ctx, &mut plan);
        outpost(ctx, &mut plan);
        if let (true, Some(h), Some(w)) = (
            ctx.world.threat.intruders.is_empty(),
            super::home(ctx),
            ctx.world.threat.radar_warning,
        ) {
            plan.radar = Some((h, w.toward));
        }
        if ctx.world.army.commander_danger {
            if let Some(c) = ctx.world.own.commander.and_then(|c| ctx.world.own.get(c)) {
                plan.commander = Some((c.unit.pos, ctx.world.army.commander_threats.clone()));
            }
        }

        for u in ctx.world.own.combat.clone() {
            if let Some(want) = self.duty(ctx, &plan, u) {
                order(ctx, out, u, want);
            }
        }
        self.anti_air(ctx, out, &plan);
        commander_alone(ctx, out, &plan);
    }

    /// The build-site guard group (§5.6).
    fn site_guard(&mut self, ctx: &Ctx<'_>) {
        let size = usize::try_from(ctx.knobs.build.site_guard_size.max(0)).unwrap_or(0);
        let Some(req) = ctx.world.coord.site_guard.filter(|_| size > 0) else {
            self.tac.guard.clear();
            return;
        };
        if self.tac.guard_done == Some(req.tick) {
            self.tac.guard.clear();
            return;
        }
        let own = &ctx.world.own;
        let built = own
            .units
            .iter()
            .any(|s| s.building && !s.is_frame() && s.unit.pos.within(req.site, SITE_GUARD_DONE));
        let timed_out =
            ctx.tick.saturating_sub(req.tick) >= secs(ctx.knobs.build.site_guard_timeout_s);
        if own.get(req.builder).is_none() || built || timed_out {
            self.tac.guard_done = Some(req.tick);
            self.tac.guard.clear();
            return;
        }
        let taken: BTreeSet<UnitId> = ctx
            .world
            .army
            .wave
            .iter()
            .chain(&ctx.world.army.reserve)
            .chain(&self.tac.raid)
            .chain(&self.tac.guard)
            .copied()
            .collect();
        let free = without(&own.combat, &[&taken]);
        for u in free {
            if self.tac.guard.len() >= size {
                break;
            }
            self.tac.guard.push(u);
        }
        self.tac.guard.sort();
    }

    /// Wave formation, reinforcement, contact, the reserve and spending
    /// (§5.2).
    fn wave_upkeep(&mut self, ctx: &mut Ctx<'_>) {
        let k = &ctx.knobs.army;
        if ctx.world.army.phase != Phase::Assault {
            ctx.world.army.wave_centre = None;
            return;
        }
        let combat = ctx.world.own.combat.clone();
        let raid: BTreeSet<UnitId> = self.tac.raid.iter().copied().collect();
        let guard: BTreeSet<UnitId> = self.tac.guard.iter().copied().collect();
        let health = |ctx: &Ctx<'_>, u: UnitId| ctx.world.own.get(u).map_or(0, |s| s.unit.health);
        if !k.waves {
            let wave = without(&combat, &[&raid, &guard]);
            ctx.world.army.wave_centre = centroid(
                wave.iter()
                    .filter_map(|&u| ctx.world.own.get(u).map(|s| s.unit.pos)),
            );
            ctx.world.army.wave = wave;
            ctx.world.army.reserve.clear();
            return;
        }
        let mut wave = std::mem::take(&mut ctx.world.army.wave);
        let mut reserve = std::mem::take(&mut ctx.world.army.reserve);
        if wave.is_empty() && !self.pic.spent {
            let pool = without(&combat, &[&raid, &guard]);
            let n = pool.len();
            if n > 0 {
                let commit = if k.reserve {
                    let n64 = i64::try_from(n).unwrap_or(i64::MAX);
                    let r = (i128::from(n64) * i128::from(k.commit_frac.raw()) + 5_000) / 10_000;
                    usize::try_from(r).unwrap_or(n).clamp(1, n)
                } else {
                    n
                };
                wave = pool
                    .get(..commit)
                    .map(<[UnitId]>::to_vec)
                    .unwrap_or_default();
                reserve = pool
                    .get(commit..)
                    .map(<[UnitId]>::to_vec)
                    .unwrap_or_default();
                self.tac.wave_health = wave.iter().map(|&u| (u, health(ctx, u))).collect();
                ctx.world.army.contact_since = None;
            }
        } else if !wave.is_empty() {
            let w: BTreeSet<UnitId> = wave.iter().chain(&reserve).copied().collect();
            let free = without(&combat, &[&raid, &guard, &w]);
            let need = usize::try_from(k.reinforce.max(1)).unwrap_or(usize::MAX);
            if free.len() >= need {
                for u in free {
                    self.tac.wave_health.insert(u, health(ctx, u));
                    wave.push(u);
                }
                wave.sort();
            }
        }
        let centre = centroid(
            wave.iter()
                .filter_map(|&u| ctx.world.own.get(u).map(|s| s.unit.pos)),
        );
        // Contact: a member lost health, or a standing contact is near.
        let hurt = wave.iter().any(|&u| {
            self.tac
                .wave_health
                .get(&u)
                .is_some_and(|&h| health(ctx, u) < h)
        });
        let meet = dist(k.meet_radius);
        let near = centre.is_some_and(|c| {
            ctx.world
                .contacts
                .near(c, meet)
                .any(|e| ctx.standing(e.id).is_some())
        });
        if (hurt || near) && ctx.world.army.contact_since.is_none() && !wave.is_empty() {
            ctx.world.army.contact_since = Some(ctx.tick);
        }
        for &u in &wave {
            self.tac.wave_health.insert(u, health(ctx, u));
        }
        // The reserve joins after enough contact, or when the wave is small.
        let released = ctx
            .world
            .army
            .contact_since
            .is_some_and(|t| ctx.tick.saturating_sub(t) >= secs(k.reserve_release_s));
        if !reserve.is_empty() && !wave.is_empty() && (released || wave.len() <= RESERVE_JOINS_AT) {
            wave.append(&mut reserve);
            wave.sort();
        }
        let spent_size = usize::try_from(k.spent_size.max(0)).unwrap_or(0);
        if wave.len() < spent_size || (wave.is_empty() && !self.pic.spent) {
            // Spent: the reserve returns to duty.
            self.pic.spent = true;
            reserve.clear();
            self.tac.raid.clear();
        }
        ctx.world.army.wave_centre = centre;
        ctx.world.army.wave = wave;
        ctx.world.army.reserve = reserve;
    }

    /// The raiding party (§5.6).
    fn raid_upkeep(&mut self, ctx: &Ctx<'_>) {
        let k = &ctx.knobs.army;
        let a = &ctx.world.army;
        if a.phase != Phase::Assault || a.wave.is_empty() || self.pic.spent {
            self.tac.raid.clear();
            self.tac.raid_released = false;
            return;
        }
        let size = usize::try_from(k.raid_size.max(0)).unwrap_or(0);
        if self.tac.raid.is_empty() && k.raids && size > 0 && ctx.world.threat.intruders.is_empty()
        {
            let taken: BTreeSet<UnitId> = a
                .wave
                .iter()
                .chain(&a.reserve)
                .chain(&self.tac.guard)
                .copied()
                .collect();
            let free = without(&ctx.world.own.combat, &[&taken]);
            // A raid of one is a gift: fewer than the size, no raid.
            if free.len() >= size {
                self.tac.raid = free.get(..size).map(<[UnitId]>::to_vec).unwrap_or_default();
                self.tac.raid_since = ctx.tick;
                self.tac.raid_released = false;
            }
        }
        if self.tac.raid.is_empty() || self.tac.raid_released {
            return;
        }
        let go = !k.raid_sync || {
            let waited = ctx.tick.saturating_sub(self.tac.raid_since) >= secs(k.raid_sync_s);
            let contact = a.contact_since.is_some();
            let meet = dist(k.meet_radius);
            let near = a
                .wave_centre
                .is_some_and(|c| ctx.world.contacts.near(c, meet).next().is_some());
            let close = match (a.wave_centre, a.objective) {
                (Some(c), Some(o)) => c.within(o, RAID_RELEASE_RADIUS),
                _ => false,
            };
            waited || contact || near || close
        };
        if go {
            self.tac.raid_released = true;
        }
    }

    /// Damaged units (§5.11): `Some(order)` while the unit is mending.
    fn mend(&mut self, ctx: &Ctx<'_>, u: UnitId) -> Option<Option<Command>> {
        let k = &ctx.knobs.army;
        let home = super::home(ctx)?;
        if !k.retreat_damaged {
            self.tac.mending.clear();
            return None;
        }
        let s = ctx.world.own.get(u)?;
        let pct = health_pct(s.unit.health, ctx.def(s.unit.def));
        if let Some(m) = self.tac.mending.get_mut(&u) {
            if pct >= i64::from(k.rejoin_pct) {
                self.tac.mending.remove(&u);
                return None;
            }
            if ctx.tick.saturating_sub(m.since) >= secs(k.mend_wait_s) {
                m.done = true;
            }
            if m.done {
                return None;
            }
        } else {
            let raider = ctx.world.roles.is(s.unit.def, Role::RaiderKbot);
            let below = if raider {
                k.retreat_raider_pct
            } else {
                k.retreat_line_pct
            };
            let mender = !k.mend_needs_mender || !ctx.world.own.builders.is_empty();
            if pct >= i64::from(below)
                || !mender
                || !s.unit.pos.within(home, dist(k.mend_max_walk))
                || !ctx.world.threat.intruders.is_empty()
            {
                return None;
            }
            self.tac.mending.insert(
                u,
                Mend {
                    since: ctx.tick,
                    done: false,
                },
            );
        }
        // One of 8 spots round home, by a hash of the id.
        let spot_k = (u.0.wrapping_mul(2_654_435_761) >> 16) as usize % MEND_SPOTS;
        let spot = on_bearing(home, spot_k * (16 / MEND_SPOTS), dist(k.mend_spot_radius));
        let spot = inside(ctx.view, spot, MAP_MARGIN);
        if s.unit.pos.within(spot, dist(k.mend_spot_tolerance)) {
            Some(None)
        } else {
            Some(Some(Command::Move(spot)))
        }
    }

    /// One combat unit's order, first match (§5.12).
    fn duty(&mut self, ctx: &Ctx<'_>, plan: &Plan, u: UnitId) -> Option<Command> {
        // 1. Damaged retreat.
        if let Some(m) = self.mend(ctx, u) {
            return m;
        }
        let s = ctx.world.own.get(u)?;
        let d = ctx.def(s.unit.def)?;
        let here = s.unit.pos;
        let in_wave = plan.wave.contains(&u) || plan.raid.contains(&u);
        // 2. A target in reach. Holding units only answer what is in range.
        let radius = if plan.holding && !in_wave {
            i64::from(d.max_range())
        } else {
            super::micro::acquire_radius(ctx, d)
        };
        if let Some(t) = self.micro.acquire(ctx, u, here, d, radius) {
            return self.engage(ctx, u, t);
        }
        // 3. Commander guard.
        let k = &ctx.knobs;
        if let Some((cpos, threats)) = &plan.commander {
            if here.within(*cpos, dist(k.cmd.guard_radius)) {
                let t = threats
                    .iter()
                    .filter(|&&t| ctx.target(t).is_some())
                    .filter_map(|&t| ctx.world.contacts.get(t))
                    .min_by_key(|c| (c.pos.dist2(*cpos), c.id));
                return match t {
                    Some(c) => Some(Command::Attack(c.id)),
                    None if !here.within(*cpos, COMMANDER_GUARD_SLACK) => {
                        Some(Command::Move(*cpos))
                    }
                    None => None,
                };
            }
        }
        // 4. Outpost response.
        if let Some((site, who, raiders)) = &plan.outpost {
            if who.contains(&u) {
                let t = raiders
                    .iter()
                    .filter(|&&t| ctx.target(t).is_some())
                    .filter_map(|&t| ctx.world.contacts.get(t))
                    .min_by_key(|c| (c.pos.dist2(here), c.id));
                return Some(match t {
                    Some(c) => Command::Attack(c.id),
                    None => Command::AttackMove(*site),
                });
            }
        }
        // 5. Build-site guard.
        if plan.guard.contains(&u) {
            if let Some(site) = plan.guard_site {
                return (!here.within(site, SITE_GUARD_STAND)).then_some(Command::Move(site));
            }
        }
        // 6. Phase duty.
        let target = plan.target.filter(|&t| ctx.target(t).is_some());
        if plan.phase == Phase::Defend && plan.responders.contains(&u) {
            if let Some(t) = target {
                return Some(Command::Attack(t));
            }
        }
        if plan.raid.contains(&u) {
            return raid_errand(ctx, plan, u, here);
        }
        if plan.wave.contains(&u) {
            if let Some(o) = plan.objective {
                return Some(if k.army.attack_move {
                    Command::AttackMove(o)
                } else {
                    Command::Move(o)
                });
            }
        }
        if plan.phase == Phase::Assault && plan.responders.contains(&u) {
            if let Some(t) = target {
                return Some(Command::Attack(t));
            }
        }
        if !plan.reserve.contains(&u) && !plan.holding {
            if let Some((h, toward)) = plan.radar {
                return radar_slot(ctx, u, here, h, toward);
            }
        }
        to_rally(ctx, u, here, plan.rally)
    }

    /// Mobile anti-air (§5.6 escorts): air targets in reach, escorts guard
    /// the slowest wave member, at least one stays home, the rest rally.
    fn anti_air(&mut self, ctx: &Ctx<'_>, out: &mut Staging, plan: &Plan) {
        let aa = ctx.world.own.anti_air.clone();
        let escorts = if plan.phase == Phase::Assault && !plan.wave.is_empty() {
            let n = usize::try_from(ctx.knobs.def.aa_escorts.max(0)).unwrap_or(0);
            n.min(aa.len().saturating_sub(1))
        } else {
            0
        };
        let slowest = plan
            .wave
            .iter()
            .filter_map(|&u| ctx.world.own.get(u))
            .min_by_key(|s| (ctx.def(s.unit.def).map_or(0, |d| d.speed), s.id()))
            .map(|s| s.id());
        for (i, &u) in aa.iter().enumerate() {
            let Some(s) = ctx.world.own.get(u) else {
                continue;
            };
            let Some(d) = ctx.def(s.unit.def) else {
                continue;
            };
            let radius = super::micro::acquire_radius(ctx, d);
            let want = if let Some(t) = self.micro.acquire(ctx, u, s.unit.pos, d, radius) {
                self.engage(ctx, u, t)
            } else if let (true, Some(g)) = (i < escorts, slowest) {
                Some(Command::Guard(g))
            } else {
                to_rally(ctx, u, s.unit.pos, plan.rally)
            };
            if let Some(w) = want {
                order(ctx, out, u, w);
            }
        }
    }
}

/// The rally point (§5.2): home + `army.rally` towards the enemy base (+x
/// if unknown), or the ferry muster point.
fn rally_point(ctx: &Ctx<'_>) -> Option<Pos> {
    if let Some(f) = ctx.world.coord.ferry_rally {
        return Some(f);
    }
    let h = super::home(ctx)?;
    let r = dist(ctx.knobs.army.rally);
    let p = match ctx.world.enemy_base() {
        Some(b) if b != h => h.toward(b, r),
        _ => super::geom::pos(i64::from(h.x) + r, i64::from(h.z)),
    };
    Some(inside(ctx.view, p, MAP_MARGIN))
}

/// The wave objective (§5.2), first available, with the meeting
/// engagement.
fn objective(ctx: &Ctx<'_>) -> Option<Pos> {
    let k = &ctx.knobs.army;
    let w = &ctx.world;
    let base = w.enemy_base();
    let first = w
        .army
        .hunt_target
        .and_then(|c| w.contacts.get(c))
        .map(|c| c.pos)
        .or_else(|| {
            w.grid
                .best_attack_cell(base, dist(k.target_base_radius), k.threat_aversion)
                .map(|c| c.center())
        })
        .or(base)
        .or_else(|| {
            let h = w.own.home.unwrap_or_default();
            w.contacts
                .iter()
                .min_by_key(|c| (c.pos.dist2(h), c.id))
                .map(|c| c.pos)
        });
    if let Some(c) = w.army.wave_centre {
        let near: Vec<Pos> = w
            .contacts
            .near(c, dist(k.meet_radius))
            .filter(|e| e.armed_ground_mobile())
            .map(|e| e.pos)
            .collect();
        if near.len() >= usize::try_from(k.meet_count.max(1)).unwrap_or(usize::MAX) {
            return centroid(near).or(first);
        }
    }
    first
}

/// The raid target (§5.6).
fn raid_target(ctx: &Ctx<'_>) -> Option<UnitId> {
    let k = &ctx.knobs.army;
    let w = &ctx.world;
    let base = w.enemy_base();
    let focus = w.enemy.focus;
    let home = w.own.home.unwrap_or_default();
    let avoid = dist(k.raid_avoid_base);
    let eligible: Vec<_> = w
        .contacts
        .iter()
        .filter(|c| c.building && focus.is_none_or(|f| c.owner == f))
        .filter(|c| base.is_none_or(|b| !c.pos.within(b, avoid - 1)))
        .filter(|c| w.grid.threat_at(c.pos.vis_cell()) == 0)
        .filter(|c| ctx.standing(c.id).is_some())
        .collect();
    if k.raid_frames {
        let window = secs(k.raid_frame_window_s);
        let fresh = eligible
            .iter()
            .filter(|c| c.frame && ctx.tick.saturating_sub(c.first_seen) <= window)
            .min_by_key(|c| (c.pos.dist2(home), c.id));
        if let Some(f) = fresh {
            let builder = w
                .contacts
                .near(f.pos, RAID_BUILDER_RADIUS)
                .filter(|b| b.builder && !b.building && focus.is_none_or(|o| b.owner == o))
                .filter(|b| w.contacts.seen_now(b.id))
                .min_by_key(|b| (b.pos.dist2(f.pos), b.id));
            return Some(builder.map_or(f.id, |b| b.id));
        }
    }
    eligible
        .iter()
        .min_by_key(|c| (c.pos.dist2(home), c.id))
        .map(|c| c.id)
}

/// A raider's errand (§5.6): wait at the rally point until released, then
/// attack the target when seen and hittable, else walk to it.
fn raid_errand(ctx: &Ctx<'_>, plan: &Plan, u: UnitId, here: Pos) -> Option<Command> {
    if !plan.raid_go {
        return to_rally(ctx, u, here, plan.rally);
    }
    match plan.raid_target.and_then(|t| ctx.world.contacts.get(t)) {
        Some(c) if ctx.target(c.id).is_some() && ctx.view.can_hit(u, c.id) => {
            Some(Command::Attack(c.id))
        }
        Some(c) => Some(Command::Move(c.pos)),
        None => plan.objective.map(Command::AttackMove),
    }
}

/// Defence (§5.4): holding, and the proportional response.
fn defence(ctx: &Ctx<'_>, plan: &mut Plan) {
    let k = &ctx.knobs.army;
    let t = &ctx.world.threat;
    let combat = &ctx.world.own.combat;
    plan.holding = k.hold_outnumbered && t.intruders.len() > combat.len();
    plan.target = t.target_intruder;
    let Some(target) = t.target_intruder.and_then(|i| ctx.world.contacts.get(i)) else {
        return;
    };
    if plan.holding {
        return;
    }
    let empty = BTreeSet::new();
    let wave = if plan.phase == Phase::Assault {
        &plan.wave
    } else {
        &empty
    };
    let mut pool = without(combat, &[&plan.raid, &plan.guard, wave]);
    if !k.proportional {
        plan.responders = pool.into_iter().collect();
        return;
    }
    let pos = |u: UnitId| ctx.world.own.get(u).map(|s| s.unit.pos).unwrap_or_default();
    pool.sort_by_key(|&u| (pos(u).dist2(target.pos), u));
    let n = i64::try_from(pool.len()).unwrap_or(i64::MAX);
    if n == 0 {
        return;
    }
    let min = i64::from(k.respond_min).clamp(1, n);
    let max = {
        let raw = i128::from(k.respond_max_share.raw()) * i128::from(n);
        i64::try_from((raw + 9_999) / 10_000).unwrap_or(n).min(n)
    };
    let floor = min.max(i64::from(t.intruder_count).min(max));
    let mut metal = Amount::ZERO;
    for u in pool {
        let picked = i64::try_from(plan.responders.len()).unwrap_or(i64::MAX);
        if picked >= max
            || (picked >= floor
                && at_least(metal.milli(), k.respond_ratio, t.intruder_metal.milli()))
        {
            break;
        }
        metal += ctx
            .world
            .own
            .get(u)
            .map_or(Amount::ZERO, |s| metal_of(ctx.def(s.unit.def)));
        plan.responders.insert(u);
    }
}

/// Outpost raids (§5.4), only with no intruder at base; one outpost per
/// pass.
fn outpost(ctx: &mut Ctx<'_>, plan: &mut Plan) {
    ctx.world.threat.raided_outpost = None;
    let k = &ctx.knobs.army;
    let Some(home) = super::home(ctx) else {
        return;
    };
    if !k.outpost_response || !ctx.world.threat.intruders.is_empty() {
        return;
    }
    let defend = dist(k.defend_radius);
    let raid_r = dist(k.outpost_raid_radius);
    let mut found: Option<(Pos, Vec<UnitId>)> = None;
    for s in &ctx.world.own.units {
        if !s.building || s.is_frame() || s.unit.pos.within(home, defend) {
            continue;
        }
        let raiders: Vec<UnitId> = ctx
            .world
            .contacts
            .near(s.unit.pos, raid_r)
            .filter(|c| c.armed_ground_mobile() && ctx.world.contacts.seen_now(c.id))
            .map(|c| c.id)
            .collect();
        if !raiders.is_empty() {
            found = Some((s.unit.pos, raiders));
            break;
        }
    }
    let Some((site, raiders)) = found else {
        return;
    };
    ctx.world.threat.raided_outpost = Some(site);
    let taken: BTreeSet<UnitId> = plan
        .wave
        .iter()
        .chain(&plan.reserve)
        .chain(&plan.raid)
        .chain(&plan.guard)
        .copied()
        .collect();
    let reach = dist(k.outpost_response_radius);
    let who: BTreeSet<UnitId> = without(&ctx.world.own.combat, &[&taken])
        .into_iter()
        .filter(|&u| {
            ctx.world
                .own
                .get(u)
                .is_some_and(|s| s.unit.pos.within(site, reach))
        })
        .collect();
    let ours = who
        .iter()
        .filter_map(|&u| ctx.world.own.get(u))
        .fold(Amount::ZERO, |a, s| a + metal_of(ctx.def(s.unit.def)));
    let theirs = raiders
        .iter()
        .filter_map(|&r| ctx.world.contacts.get(r))
        .fold(Amount::ZERO, |a, c| a + c.metal);
    if !who.is_empty() && at_least(ours.milli(), k.outpost_strength, theirs.milli()) {
        plan.outpost = Some((site, who, raiders));
    }
}

/// Walks to the rally point when further than the slack (§5.2).
fn to_rally(ctx: &Ctx<'_>, u: UnitId, here: Pos, rally: Option<Pos>) -> Option<Command> {
    let r = rally?;
    if here.within(r, RALLY_SLACK) {
        // Near enough: an idle unit stays; one still marching is stopped.
        return match super::current_order(ctx, u) {
            Some(Command::AttackMove(_)) => Some(Command::Move(here)),
            _ => None,
        };
    }
    Some(Command::Move(r))
}

/// The radar line slot (§5.4): (id mod 7) − 3 across the warning bearing,
/// 540 out from home.
fn radar_slot(ctx: &Ctx<'_>, u: UnitId, here: Pos, home: Pos, toward: Pos) -> Option<Command> {
    let slot = i64::from(u.0 % RADAR_LINE_SLOTS) - i64::from(RADAR_LINE_SLOTS / 2);
    let p = sidestep(home, toward, slot * RADAR_LINE_PITCH, RADAR_LINE_DISTANCE);
    let p = inside(ctx.view, p, MAP_MARGIN);
    (!here.within(p, RADAR_LINE_SLACK)).then_some(Command::Move(p))
}

/// The commander alone (§5.4) and harassment (§5.7): routine orders, which
/// lose to a planner job and to the sweep's survival orders.
fn commander_alone(ctx: &Ctx<'_>, out: &mut Staging, plan: &Plan) {
    let Some(c) = ctx.world.own.commander else {
        return;
    };
    if ctx.world.army.commander_danger || ctx.world.coord.commander_tasked {
        return;
    }
    let Some(s) = ctx.world.own.get(c) else {
        return;
    };
    let Some(d) = ctx.def(s.unit.def) else {
        return;
    };
    // Its ordinary weapons, not the special one.
    let range = i64::from(
        d.weapons
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, w)| w.range)
            .max()
            .unwrap_or(0),
    );
    let k = &ctx.knobs.cmd;
    let t = &ctx.world.threat;
    let max = usize::try_from(k.defends_alone_max.max(0)).unwrap_or(0);
    if plan.phase == Phase::Defend && ctx.world.own.combat.is_empty() && t.intruders.len() <= max {
        if let Some(i) = t.target_intruder.and_then(|i| ctx.world.contacts.get(i)) {
            if i.pos.within(s.unit.pos, range) && ctx.target(i.id).is_some() {
                order(ctx, out, c, Command::Attack(i.id));
                return;
            }
        }
    }
    if k.answers_harassment
        && !ctx.world.enemy.besieged_factories.is_empty()
        && ctx.world.own.army_size == 0
        && ctx.world.own.hulls.is_empty()
        && super::current_order(ctx, c).is_none()
    {
        let r = dist(k.danger_radius);
        let near = ctx
            .world
            .contacts
            .near(s.unit.pos, r)
            .filter(|e| matches!(ctx.standing(e.id), Some(Standing::Live(_))))
            .min_by_key(|e| (e.pos.dist2(s.unit.pos), e.id));
        if let Some(e) = near {
            order(ctx, out, c, Command::Attack(e.id));
        }
    }
}
