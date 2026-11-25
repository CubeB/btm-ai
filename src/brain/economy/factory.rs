//! Factory production (SPEC §4.4): the besieged and stalled-head checks, the
//! tier-two reserve, and the unit tables per factory type.

use std::collections::BTreeMap;

use super::super::{Ctx, Priority, Role, Staging, World};
use super::facts::{affordable, secs, Census, Facts};
use crate::knobs::Knobs;
use crate::view::{Command, DefId, Orders, Tick, UnitId};

/// The watch on one factory's queue head (SPEC §4.4 rule 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HeadWatch {
    def: DefId,
    frame: Option<UnitId>,
    since: Tick,
    done: i64,
}

/// What the factory step needs from the rest of the planner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wants {
    /// Walls are owed under walling every tower (constructor target).
    pub walls_owed: bool,
    /// A seaplane plant or underwater fusion is still wanted (advanced
    /// construction sub).
    pub sub_work: bool,
}

/// Factory production state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Factories {
    heads: BTreeMap<UnitId, HeadWatch>,
    /// When anything was first owed (latched once per game, SPEC §4.4).
    owed_since: Option<Tick>,
}

impl Factories {
    /// Tops up every factory, in id order (SPEC §4.2 step 1, §4.4).
    pub fn top_up(
        &mut self,
        ctx: &Ctx<'_>,
        out: &mut Staging,
        f: &Facts,
        wants: Wants,
        saving: bool,
    ) {
        // While the tier-two reserve holds, or while the planner saves for
        // an item (spec-readings §4.2), only builders are queued.
        let reserve = self.reserve_holds(ctx, f);
        let hold = reserve || saving;
        let factories = ctx.world.own.factories.clone();
        self.heads.retain(|id, _| factories.contains(id));
        for id in factories {
            let Some(s) = ctx.world.own.get(id) else {
                continue;
            };
            if s.unit.scripted || ctx.in_flight(id).is_some() {
                continue;
            }
            let Some(fs) = s.unit.factory.clone() else {
                continue;
            };
            let fdef = s.unit.def;
            // 1. Besieged: cancel everything queued.
            if ctx.knobs.build.notice_harassment && ctx.world.enemy.besieged_factories.contains(&id)
            {
                let mut counts: BTreeMap<DefId, i32> = BTreeMap::new();
                for &q in &fs.queue {
                    *counts.entry(q).or_insert(0) += 1;
                }
                out.set_priority(Priority::Tasked);
                for (def, n) in counts {
                    out.now(id, Command::FactoryQueue { def, count: -n });
                }
                continue;
            }
            // 2. Stalled head.
            if ctx.knobs.build.drop_stalled_head && !reserve {
                if let Some(drop) = self.stalled_head(ctx, id, f.tick) {
                    let n = fs.queue.iter().filter(|&&q| q == drop).count();
                    let n = i32::try_from(n).unwrap_or(i32::MAX);
                    out.set_priority(Priority::Tasked);
                    out.now(
                        id,
                        Command::FactoryQueue {
                            def: drop,
                            count: -n,
                        },
                    );
                    continue;
                }
            } else {
                self.heads.remove(&id);
            }
            // 3. Queue depth.
            let depth = usize::try_from(ctx.knobs.build.queue_depth.max(1)).unwrap_or(1);
            if fs.queue.len() >= depth {
                continue;
            }
            // 4. One unit.
            if let Some(def) = choose(ctx, fdef, f, wants, hold) {
                out.set_priority(Priority::Tasked);
                out.now(id, Command::FactoryQueue { def, count: 1 });
            }
        }
    }

    /// The stalled-head watch; returns the type to drop.
    fn stalled_head(&mut self, ctx: &Ctx<'_>, id: UnitId, tick: Tick) -> Option<DefId> {
        let s = ctx.world.own.get(id)?;
        let fs = s.unit.factory.as_ref()?;
        let Some(&head) = fs.queue.first() else {
            self.heads.remove(&id);
            return None;
        };
        let hd = ctx.def(head)?;
        let worker = ctx.def(s.unit.def).map_or(0, |d| d.worker_rate);
        let frame = fs.building;
        let (done, total) = frame
            .and_then(|f| ctx.world.own.get(f))
            .and_then(|f| f.unit.frame)
            .map_or((0, hd.build_time * 30), |p| (p.done, p.total));
        let restart = |w: &mut BTreeMap<UnitId, HeadWatch>| {
            w.insert(
                id,
                HeadWatch {
                    def: head,
                    frame,
                    since: tick,
                    done,
                },
            );
        };
        let Some(watch) = self.heads.get(&id).copied() else {
            restart(&mut self.heads);
            return None;
        };
        if watch.def != head || watch.frame != frame || done < watch.done {
            restart(&mut self.heads);
            return None;
        }
        let floor = worker.max(total / 100).max(1);
        let owed =
            crate::view::Amount(hd.cost_metal.milli().saturating_mul(total - done) / total.max(1));
        let can = affordable(&ctx.world.economy, owed, total - done, worker, 0, false);
        if done - watch.done >= floor || can {
            restart(&mut self.heads);
            return None;
        }
        (tick.saturating_sub(watch.since) >= secs(ctx.knobs.build.stall_s)).then_some(head)
    }

    /// The tier-two reserve (SPEC §4.4): while it holds, only builders are
    /// queued.
    fn reserve_holds(&mut self, ctx: &Ctx<'_>, f: &Facts) -> bool {
        let k = ctx.knobs;
        if !k.eco.t2_reserve {
            return false;
        }
        let c = Census::new(ctx.world);
        let reactors = c.done(Role::Fusion) + c.done(Role::UnderwaterFusion);
        let adv = c.has(Role::AdvancedConstructor);
        let sub = c.has(Role::AdvancedConstructionSub) && c.def(Role::UnderwaterFusion).is_some();
        let owed = (adv && c.def(Role::Moho).is_some() && !c.has(Role::Moho))
            || ((c.tot(Role::AdvancedConstructor) > 0 || sub)
                && reactors == 0
                && (c.def(Role::Fusion).is_some() || c.def(Role::UnderwaterFusion).is_some()));
        if !owed {
            return false;
        }
        let since = *self.owed_since.get_or_insert(f.tick);
        let army = i64::from(ctx.world.own.army_size)
            + i64::try_from(ctx.world.own.hulls.len()).unwrap_or(0);
        f.tick.saturating_sub(since) < secs(k.eco.t2_reserve_max_s)
            && f.quiet
            && army >= i64::from(k.eco.t2_reserve_min_army)
    }
}

/// A candidate with its share.
struct Share {
    def: DefId,
    share: i64,
}

/// The candidate with the smallest owned / share, by cross-multiplication;
/// the first listed wins a tie; zero shares are skipped (SPEC §4.4).
fn by_share(w: &World, cands: &[Share]) -> Option<DefId> {
    let mut best: Option<(&Share, i64)> = None;
    for c in cands.iter().filter(|c| c.share > 0) {
        let owned = i64::from(w.own.total_of(c.def));
        match best {
            Some((b, bo)) if owned * b.share >= bo * c.share => {}
            _ => best = Some((c, owned)),
        }
    }
    best.map(|(c, _)| c.def)
}

/// The constructor target of a kbot lab (SPEC §4.4).
pub fn constructor_target(k: &Knobs, f: &Facts, walls_owed: bool) -> i32 {
    let mut t = k.build.constructors;
    if f.capacity_short {
        t += k.eco.capacity_constructors;
    }
    let per = k.build.deposits_per_constructor;
    if per > 0 {
        t += k
            .build
            .expansion_constructors
            .min(f.free_deposits / per)
            .max(0);
    }
    if walls_owed {
        t += k.fort.extra_constructors;
    }
    if k.open.timed && f.tick >= secs(k.open.constructor_by_s) {
        t = t.max(1);
    }
    t
}

/// Chooses the unit a factory queues (SPEC §4.4 tables), or nothing.
#[allow(clippy::too_many_lines)]
fn choose(ctx: &Ctx<'_>, fdef: DefId, f: &Facts, wants: Wants, reserve: bool) -> Option<DefId> {
    let k = ctx.knobs;
    let w = &*ctx.world;
    let c = Census::new(w);
    let fd = ctx.def(fdef)?;
    let can = |r: Role| c.def(r).filter(|&d| fd.can_build(d));
    let lean = |d: DefId| i64::from(w.army.lean.get(&d).copied().unwrap_or(0));
    let share = |r: Role, knob: i32| -> Option<Share> {
        let d = can(r)?;
        Some(Share {
            def: d,
            share: i64::from(knob) + lean(d),
        })
    };
    let pick = |v: Vec<Option<Share>>| by_share(w, &v.into_iter().flatten().collect::<Vec<_>>());
    let below = |r: Role, n: i32| can(r).filter(|_| c.tot(r) < n);
    let land_capped = k.build.isolated_army_cap > 0
        && w.reach.unreachable_ground_exists
        && w.map.water_permille * 10_000 >= k.build.isolated_cap_water_frac.raw() * 1000
        && i64::from(w.own.army_size) >= i64::from(k.build.isolated_army_cap);
    let surplus = k.eco.surplus_expand && f.metal_full;
    let ferry = ferry_state(ctx, f);

    let role = c.roles.role_of(fdef);
    match role {
        Some(Role::VehiclePlant) => {
            if let Some(d) = below(Role::ScoutVehicle, k.scout.vehicles) {
                return (!reserve).then_some(d);
            }
            if reserve || land_capped {
                return None;
            }
            pick(vec![
                share(Role::LightTank, k.mix.vp_light_tank),
                share(Role::MissileTruck, k.mix.vp_missile),
                share(Role::MediumTank, k.mix.vp_medium_tank),
            ])
        }
        Some(Role::AdvancedKbotLab) => {
            if let Some(d) = below(Role::AdvancedConstructor, k.build.adv_constructors) {
                return Some(d);
            }
            if reserve || land_capped {
                return None;
            }
            can(Role::AssaultKbot)
        }
        Some(Role::AirPlant) => {
            let fighters = c.tot(Role::Fighter);
            let transport = can(Role::AirTransport)
                .filter(|_| c.tot(Role::AirTransport) < k.air.transports)
                .filter(|_| (ferry.wanted || ferry.raised) && ferry.cargo && !ferry.sea_lift);
            if ferry.overdue && transport.is_some() {
                return if reserve { None } else { transport };
            }
            if reserve {
                return below(Role::AirConstructor, k.build.air_constructors);
            }
            if let Some(d) = below(Role::ScoutPlane, k.air.scout_planes) {
                return Some(d);
            }
            if transport.is_some() {
                return transport;
            }
            let armed_air_seen = w.enemy.peak_armed_air > 0;
            if f.air_threat && armed_air_seen {
                if let Some(d) = below(Role::Fighter, k.air.fighters) {
                    return Some(d);
                }
            }
            if let Some(d) = below(Role::AirConstructor, k.build.air_constructors) {
                return Some(d);
            }
            if let Some(d) = below(Role::Fighter, k.air.patrol_fighters) {
                return Some(d);
            }
            let target = i32::try_from(w.enemy.peak_armed_air)
                .unwrap_or(i32::MAX)
                .clamp(FIGHTER_FLOOR, k.air.fighters_max.max(FIGHTER_FLOOR));
            if f.air_threat && fighters < target {
                if let Some(d) = can(Role::Fighter) {
                    return Some(d);
                }
            }
            if let Some(d) = below(Role::Bomber, k.air.patrol_bombers) {
                return Some(d);
            }
            let bombers = if surplus {
                k.air.bombers.max(BOMBER_BASE * k.eco.surplus_bomber_mult)
            } else {
                k.air.bombers
            };
            below(Role::Bomber, bombers)
        }
        Some(Role::AdvancedAirPlant) => {
            if reserve {
                return None;
            }
            below(Role::Gunship, k.air.gunships)
        }
        Some(Role::Shipyard) => {
            let mut fleet = f.fleet_target;
            if surplus {
                fleet = fleet.saturating_mul(k.eco.surplus_fleet_mult);
            }
            let subs_t = k.navy.subs.min(fleet).max(0);
            let destroyers_t = (fleet - subs_t).max(0);
            let wet_metal = f.wet_patches;
            let sea_t = can(Role::SeaTransport)
                .filter(|_| c.tot(Role::SeaTransport) < k.navy.sea_transports);
            if ferry.overdue {
                if let Some(d) = sea_t {
                    return Some(d);
                }
            }
            if fleet > 0 {
                if let Some(d) = below(Role::ScoutShip, k.navy.scout_ships) {
                    return (!reserve).then_some(d);
                }
                if ferry.raised {
                    if let Some(d) = sea_t {
                        return Some(d);
                    }
                }
            }
            if wet_metal {
                if let Some(d) = below(Role::ConstructionShip, k.navy.construction_ships) {
                    return Some(d);
                }
            }
            if fleet <= 0 || reserve {
                return None;
            }
            if ferry.wanted && ferry.cargo {
                if let Some(d) = sea_t {
                    return Some(d);
                }
            }
            if let Some(d) = below(Role::Destroyer, destroyers_t) {
                return Some(d);
            }
            if c.tot(Role::Destroyer) >= k.navy.subs_after_destroyers {
                return below(Role::Submarine, subs_t);
            }
            None
        }
        Some(Role::AdvancedShipyard) => {
            let m = if surplus { k.eco.surplus_fleet_mult } else { 1 };
            let cruisers = c.tot(Role::Cruiser);
            let battleships = c.tot(Role::Battleship);
            if f.air_threat {
                if let Some(d) = below(Role::AntiAirShip, k.navy.aa_ships) {
                    return (!reserve).then_some(d);
                }
            }
            if cruisers >= 2 && wants.sub_work {
                if let Some(d) = below(Role::AdvancedConstructionSub, 1) {
                    return Some(d);
                }
            }
            if reserve {
                return None;
            }
            if cruisers < k.navy.cruisers * m && cruisers < 2 * (battleships + 1) {
                if let Some(d) = can(Role::Cruiser) {
                    return Some(d);
                }
            }
            if let Some(d) = below(Role::Battleship, k.navy.battleships * m) {
                return Some(d);
            }
            below(Role::Cruiser, k.navy.cruisers * m)
        }
        Some(Role::SeaplanePlant) => {
            if reserve {
                return None;
            }
            let target = i32::try_from(w.enemy.peak_armed_air)
                .unwrap_or(i32::MAX)
                .clamp(FIGHTER_FLOOR, k.air.fighters_max.max(FIGHTER_FLOOR));
            if f.air_threat {
                if let Some(d) = below(Role::SeaplaneFighter, target) {
                    return Some(d);
                }
            }
            below(Role::TorpedoSeaplane, k.air.torpedo_planes)
        }
        Some(Role::HoverPlant) => {
            if reserve || land_capped {
                return None;
            }
            if f.air_threat {
                if let Some(d) = below(Role::HoverAntiAir, HOVER_AA) {
                    return Some(d);
                }
            }
            can(Role::HoverTank)
        }
        _ => {
            // The kbot lab, and any unrecognised factory type.
            let target = constructor_target(k, f, wants.walls_owed);
            if let Some(d) = below(Role::Constructor, target) {
                return Some(d);
            }
            if reserve {
                return None;
            }
            if f.air_threat {
                if let Some(d) = below(Role::AntiAirKbot, k.def.mobile_aa) {
                    return Some(d);
                }
            }
            if land_capped {
                return None;
            }
            pick(vec![
                share(Role::RaiderKbot, k.mix.lab_raider),
                share(Role::RocketKbot, k.mix.lab_rocket),
                share(Role::ArtilleryKbot, k.mix.lab_artillery),
            ])
        }
    }
}

/// The fighter target's floor (SPEC §4.4).
const FIGHTER_FLOOR: i32 = 2;
/// Bombers kept below this × `eco.surplus_bomber_mult` in surplus (§4.4).
const BOMBER_BASE: i32 = 4;
/// Hover AA kept below this (SPEC §4.4).
const HOVER_AA: i32 = 2;

/// Ferry flags the factories read (SPEC §5.9).
#[derive(Clone, Copy, Debug, Default)]
struct Ferry {
    wanted: bool,
    raised: bool,
    overdue: bool,
    cargo: bool,
    sea_lift: bool,
}

fn ferry_state(ctx: &Ctx<'_>, f: &Facts) -> Ferry {
    let k = ctx.knobs;
    let w = &*ctx.world;
    let c = Census::new(w);
    let wanted = f.needs_ferry || w.eco.unreachable_expansion.is_some();
    let sea_lift = c.has(Role::Shipyard) && c.def(Role::SeaTransport).is_some();
    let own_ferry = if sea_lift {
        c.has(Role::SeaTransport)
    } else {
        !w.own.transports.is_empty()
    };
    let urgent = k.ferry.urgent
        && w.reach.unreachable_ground_exists
        && f.enemy_base.is_some()
        && !w.reach.land_route_to_enemy
        && !own_ferry;
    let before = f.tick < secs(k.ferry.first_by_s);
    Ferry {
        wanted,
        raised: urgent && before,
        overdue: urgent && !before,
        cargo: !w.own.combat.is_empty() || w.own.builders.len() > 1,
        sea_lift,
    }
}
