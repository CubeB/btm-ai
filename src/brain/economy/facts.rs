//! The shared economy predicates of SPEC §4.1, unit counts by role, the
//! affordability test and the type groups the planner treats specially.

use super::super::{Ctx, EconomySummary, MapCharacter, OwnState, Role, RoleTable, World};
use crate::knobs::Knobs;
use crate::view::{Amount, DefId, GameView, Pos, Tick, UnitDef, TICKS_PER_SECOND};

/// Counts of our units by role (SPEC §3.1 step 1 "completed" and "total").
#[derive(Clone, Copy)]
pub struct Census<'a> {
    pub own: &'a OwnState,
    pub roles: &'a RoleTable,
}

impl<'a> Census<'a> {
    pub fn new(w: &'a World) -> Census<'a> {
        Census {
            own: &w.own,
            roles: &w.roles,
        }
    }

    /// Finished + frames + unplaced orders of a role (0 when the role is empty).
    pub fn tot(&self, r: Role) -> i32 {
        self.roles.get(r).map_or(0, |d| {
            i32::try_from(self.own.total_of(d)).unwrap_or(i32::MAX)
        })
    }

    /// Finished units of a role.
    pub fn done(&self, r: Role) -> i32 {
        self.roles.get(r).map_or(0, |d| {
            i32::try_from(self.own.completed_of(d)).unwrap_or(i32::MAX)
        })
    }

    /// A finished unit of the role stands.
    pub fn has(&self, r: Role) -> bool {
        self.done(r) > 0
    }

    pub fn def(&self, r: Role) -> Option<DefId> {
        self.roles.get(r)
    }

    pub fn is(&self, d: DefId, r: Role) -> bool {
        self.roles.is(d, r)
    }
}

/// Whole metal (or energy) per second.
pub fn whole(a: Amount) -> i64 {
    a.whole()
}

/// Seconds as ticks; negative values count as 0.
pub fn secs(s: i32) -> Tick {
    u32::try_from(s)
        .unwrap_or(0)
        .saturating_mul(TICKS_PER_SECOND)
}

/// A distance knob in whole world units.
pub fn dist(k: crate::knobs::Fixed) -> i64 {
    k.floor().max(0)
}

/// The facts one planning pass decides on (SPEC §4.1 and the flags the list
/// reads). Recomputed at the start of every pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub tick: Tick,
    pub home: Option<Pos>,
    pub enemy_base: Option<Pos>,
    /// Metal income, whole per second.
    pub income: i64,
    pub metal_short: bool,
    pub metal_full: bool,
    pub metal_pegged: bool,
    pub energy_stalled: bool,
    pub energy_short: bool,
    pub energy_wanted: bool,
    pub energy_rich: bool,
    /// Energy stock in permille of storage.
    pub energy_fill: i64,
    pub air_matters: bool,
    pub tech_income: bool,
    pub tier_worth: bool,
    pub fleet_target: i32,
    pub capacity_short: bool,
    /// "Land factories pointless" (`build.mission_isolated_sea_air`).
    pub land_pointless: bool,
    /// The lab is pointless too (knob value 2).
    pub lab_pointless: bool,
    /// An enemy near base (urgency).
    pub near_base: bool,
    pub raided_outpost: Option<Pos>,
    pub air_threat: bool,
    /// Air worth it (SPEC §4.4).
    pub air_worth: bool,
    /// The map has submerged metal patches.
    pub wet_patches: bool,
    /// The base is quiet: no enemy near base.
    pub quiet: bool,
    /// Free deposits on our side (SPEC §4.5).
    pub free_deposits: i32,
    /// An enemy ship has been seen (remembered).
    pub enemy_ship: bool,
    /// Energy income less demand, whole per second.
    pub energy_surplus: i64,
    /// The army needs a ferry (SPEC §5.9, `ferry.want_from_map`).
    pub needs_ferry: bool,
    /// A hover route to the enemy exists (SPEC §3.4).
    pub hover_route: bool,
}

impl Facts {
    pub fn new(ctx: &Ctx<'_>, tier_worth: bool, capacity_short: bool) -> Facts {
        let w = &*ctx.world;
        let k = ctx.knobs;
        let e = &w.economy;
        let income = whole(e.raw.metal.income);
        let fleet_target = fleet_target(k, w);
        let near_base = w.threat.base_under_attack();
        let shipyard_role = w.roles.get(Role::Shipyard).is_some();
        let knob = k.build.mission_isolated_sea_air;
        let land_pointless = knob > 0
            && !w.reach.land_route_to_enemy
            && fleet_target > 0
            && shipyard_role
            && w.own.transports.is_empty()
            && !near_base;
        let air_threat = w.enemy.air_threat;
        let armed_buildings = w.contacts.iter().filter(|c| c.building && c.armed).count();
        let air_matters =
            w.map.character != MapCharacter::Land || w.reach.unreachable_ground_exists;
        let air_worth = air_matters
            || air_threat
            || i64::try_from(armed_buildings).unwrap_or(i64::MAX)
                >= i64::from(k.air.worth_it_defences);
        let wet_patches = w
            .map
            .deposits
            .iter()
            .any(|d| ctx.view.water_depth(d.center.tile()) > 0);
        let enemy_ship = w.contacts.iter().any(|c| {
            ctx.def(c.def).is_some_and(|d| {
                d.mobile && !d.flies && !d.move_class.hovers && d.move_class.min_depth > 0
            })
        });
        let mut f = Facts {
            tick: ctx.tick,
            home: w.own.home,
            enemy_base: w.enemy_base(),
            income,
            metal_short: e.metal_short(),
            metal_full: e.metal_full(),
            metal_pegged: e.metal_pegged(),
            energy_stalled: e.raw.energy.stalled,
            energy_short: e.energy_short(),
            energy_wanted: e.energy_wanted(),
            energy_rich: e.energy_rich(),
            energy_fill: e.raw.energy.fill_permille(),
            air_matters,
            tech_income: income >= i64::from(k.tech.min_income),
            tier_worth,
            fleet_target,
            capacity_short,
            land_pointless,
            lab_pointless: land_pointless && knob >= 2,
            near_base,
            raided_outpost: w.threat.raided_outpost,
            air_threat,
            air_worth,
            wet_patches,
            quiet: !near_base,
            free_deposits: 0,
            enemy_ship,
            energy_surplus: whole(e.raw.energy.income - e.raw.energy.demand),
            needs_ferry: k.ferry.want_from_map && !w.reach.land_route_to_enemy,
            hover_route: w.reach.hover.is_some() && w.reach.hover_route_to_enemy,
        };
        f.free_deposits = free_deposits(ctx, &f);
        f
    }

    /// Urgent: an enemy near base or a raided outpost (SPEC §4.2 tower worth).
    pub fn urgent(&self) -> bool {
        self.near_base || self.raided_outpost.is_some()
    }
}

/// The naval fleet target (SPEC §4.1).
pub fn fleet_target(k: &Knobs, w: &World) -> i32 {
    let fleet = k.navy.fleet;
    if fleet <= 0 {
        return 0;
    }
    match w.map.character {
        MapCharacter::Water => fleet,
        MapCharacter::Mixed if w.reach.unreachable_ground_exists => (fleet / 2).max(1),
        _ => 0,
    }
}

/// "Tier worth it" (SPEC §4.1): the tier-two assault kbot's health × best
/// weapon dps / metal against the better of the raider's and the rocket
/// kbot's, compared with `tech.min_value_ratio`. `None` until the data can
/// answer (a role is missing: never worth it).
pub fn tier_worth(view: &dyn GameView, roles: &RoleTable, k: &Knobs) -> bool {
    let value = |r: Role| -> Option<i128> {
        let d = view.unit_def(roles.get(r)?)?;
        let dps = d.weapons.iter().map(|w| w.dps_milli()).max().unwrap_or(0);
        let metal = d.cost_metal.milli().max(1);
        Some(i128::from(d.health) * i128::from(dps) * 1_000_000 / i128::from(metal))
    };
    let Some(assault) = value(Role::AssaultKbot) else {
        return false;
    };
    let best = value(Role::RaiderKbot)
        .into_iter()
        .chain(value(Role::RocketKbot))
        .max();
    let Some(best) = best.filter(|&b| b > 0) else {
        return assault > 0;
    };
    // assault / best >= ratio, with ratio in ten-thousandths.
    assault * 10_000 >= best * i128::from(k.tech.min_value_ratio.raw())
}

/// Free deposits on our side (SPEC §4.5): centre within the expansion radius
/// of home, on our side when the enemy base is known, no extractor within 48
/// (ours, our frame or a remembered enemy one), not under enemy guns.
fn free_deposits(ctx: &Ctx<'_>, f: &Facts) -> i32 {
    let w = &*ctx.world;
    let Some(home) = f.home else { return 0 };
    let radius = dist(ctx.knobs.build.expand_radius);
    let n = w
        .map
        .deposits
        .iter()
        .filter(|d| d.center.within(home, radius))
        .filter(|d| our_side(d.center, home, f.enemy_base))
        .filter(|d| !extractor_near_world(ctx.view, w, d.center, EXTRACTOR_TAKEN))
        .filter(|d| !super::siting::under_guns(ctx.view, ctx.knobs, w, d.center))
        .count();
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// Radius within which an extractor takes a deposit (SPEC §4.5).
pub const EXTRACTOR_TAKEN: i64 = 48;

/// No closer to the enemy base than to ours; always true before the enemy
/// base is known (SPEC §4.5 "our side").
pub fn our_side(p: Pos, home: Pos, enemy: Option<Pos>) -> bool {
    enemy.is_none_or(|e| p.dist2(e) >= p.dist2(home))
}

/// An extractor (ours, our frame, or a remembered enemy one) within `r`.
pub fn extractor_near_world(view: &dyn GameView, w: &World, p: Pos, r: i64) -> bool {
    let is_mex = |d: DefId| view.unit_def(d).is_some_and(|d| d.extracts_metal > 0);
    w.own
        .units
        .iter()
        .any(|s| s.building && is_mex(s.unit.def) && s.unit.pos.within(p, r))
        || w.contacts.near(p, r).any(|c| c.building && is_mex(c.def))
}

/// The affordability test of SPEC §4.1: metal stock ≥ 90% of storage (when
/// `full_override`), or stock + (income − committed) × (build seconds +
/// `extra_s`) ≥ the metal still owed. `work_left` is in work-ticks (see
/// [`crate::view::Progress`]) and `worker` in work-ticks per tick, so the
/// build takes `work_left / worker` ticks.
pub fn affordable(
    e: &EconomySummary,
    owed: Amount,
    work_left: i64,
    worker: i64,
    extra_s: i64,
    full_override: bool,
) -> bool {
    if full_override && e.metal_pegged() {
        return true;
    }
    if owed.milli() <= 0 {
        return true;
    }
    let stock = i128::from(e.raw.metal.stock.milli());
    if stock >= i128::from(owed.milli()) {
        return true;
    }
    let worker = i128::from(worker.max(0));
    if worker == 0 {
        return false;
    }
    let net = i128::from((e.raw.metal.income - e.committed_metal).milli());
    let tps = i128::from(TICKS_PER_SECOND);
    // stock + net × (work / worker / 30 + extra) ≥ owed, times 30 × worker.
    let lhs = stock * tps * worker
        + net * (i128::from(work_left.max(0)) + i128::from(extra_s.max(0)) * tps * worker);
    lhs >= i128::from(owed.milli()) * tps * worker
}

/// Affordability of a new unit of `d` built by a builder of `worker` rate.
pub fn affordable_new(e: &EconomySummary, d: &UnitDef, worker: i64, extra_s: i64) -> bool {
    affordable(
        e,
        d.cost_metal,
        d.build_time.saturating_mul(i64::from(TICKS_PER_SECOND)),
        worker,
        extra_s,
        true,
    )
}

/// Economy types skip the affordability test (SPEC §4.2): the dry extractor
/// and the land metal maker always, solar or tidal when energy is wanted.
pub fn is_economy_type(c: &Census<'_>, d: DefId, f: &Facts) -> bool {
    c.is(d, Role::Mex)
        || c.is(d, Role::MetalMaker)
        || (f.energy_wanted && (c.is(d, Role::Solar) || c.is(d, Role::Tidal)))
}

/// Types saved for with the long window `eco.save_tech_s` (SPEC §4.2).
pub fn is_tech_type(c: &Census<'_>, d: DefId, f: &Facts) -> bool {
    const TECH: [Role; 12] = [
        Role::AdvancedKbotLab,
        Role::SeaplanePlant,
        Role::HoverPlant,
        Role::AdvancedShipyard,
        Role::Fusion,
        Role::UnderwaterFusion,
        Role::Moho,
        Role::HeavyPlasmaTower,
        Role::HeavyLaserTower,
        Role::AdvancedAirPlant,
        Role::AirRepairPad,
        Role::TorpedoLauncher,
    ];
    TECH.iter().any(|&r| c.is(d, r))
        || (c.is(d, Role::AirPlant) && c.tot(Role::AirPlant) == 0 && f.air_worth)
}

/// The towers the payback test applies to (SPEC §4.2 "tower worth").
pub fn is_tower(c: &Census<'_>, d: DefId) -> bool {
    [
        Role::LightLaserTower,
        Role::AntiAirTower,
        Role::HeavyLaserTower,
        Role::HeavyPlasmaTower,
    ]
    .iter()
    .any(|&r| c.is(d, r))
}

/// Water structures, exempt from the reachability test (SPEC §4.2 rule 3).
pub fn is_water_structure(c: &Census<'_>, d: DefId) -> bool {
    [
        Role::Tidal,
        Role::Sonar,
        Role::TorpedoLauncher,
        Role::FloatingMaker,
        Role::UnderwaterMetalStorage,
        Role::UnderwaterEnergyStorage,
        Role::UnderwaterFusion,
    ]
    .iter()
    .any(|&r| c.is(d, r))
}

/// Extractor types sited by SPEC §4.5.
pub fn is_extractor(c: &Census<'_>, d: DefId) -> bool {
    c.is(d, Role::Mex) || c.is(d, Role::Moho) || c.is(d, Role::UnderwaterMex)
}

/// Tracks "capacity short" (SPEC §4.1): metal demand × `eco.capacity_ratio`
/// below metal income, continuously for `eco.capacity_s`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapacityWatch {
    since: Option<Tick>,
}

impl CapacityWatch {
    /// Samples this tick; returns whether capacity is short.
    pub fn sample(&mut self, k: &Knobs, e: &EconomySummary, tick: Tick) -> bool {
        if !k.eco.capacity_spend {
            self.since = None;
            return false;
        }
        let demand = e.raw.metal.demand.milli();
        let income = e.raw.metal.income.milli();
        let holds = k.eco.capacity_ratio.apply(demand) < income;
        if !holds {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(tick);
        tick.saturating_sub(since) >= secs(k.eco.capacity_s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Economy, Resource};

    fn eco(stock: i64, storage: i64, income: i64, committed: i64) -> EconomySummary {
        EconomySummary {
            raw: Economy {
                metal: Resource {
                    stock: Amount::units(stock),
                    storage: Amount::units(storage),
                    income: Amount::units(income),
                    demand: Amount::ZERO,
                    stalled: false,
                },
                energy: Resource::default(),
            },
            committed_metal: Amount::units(committed),
        }
    }

    #[test]
    fn affordability_counts_income_over_the_build() {
        // 600 metal, 100 s of work at rate 1 per tick (30 work-ticks a second).
        let work = 100 * 30;
        // 100 in stock + 5/s × 100 s = 600.
        assert!(affordable(
            &eco(100, 1000, 5, 0),
            Amount::units(600),
            work,
            1,
            0,
            true
        ));
        // Committed spend eats the income.
        assert!(!affordable(
            &eco(100, 1000, 5, 1),
            Amount::units(600),
            work,
            1,
            0,
            true
        ));
        // A saving window of 25 s makes it up: 100 + 4 × 125 = 600.
        assert!(affordable(
            &eco(100, 1000, 5, 1),
            Amount::units(600),
            work,
            1,
            25,
            true
        ));
        // A 90% full store overrides, unless the override is off.
        assert!(affordable(
            &eco(900, 1000, 0, 0),
            Amount::units(5000),
            work,
            1,
            0,
            true
        ));
        assert!(!affordable(
            &eco(900, 1000, 0, 0),
            Amount::units(5000),
            work,
            1,
            0,
            false
        ));
    }
}
