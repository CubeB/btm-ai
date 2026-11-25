//! The build priority list of SPEC §4.3: the at-base list (rules 0-39, and
//! the timed-opening rules of §4.13), rule E, recent-loss promotion and the
//! stranded-builder list. **The order of the list is the strategy.**

use super::super::Role;
use super::defence::OUTPOST_DISTANCE;
use super::facts::{secs, Census, Facts};
use super::siting::Builder;
use crate::knobs::Knobs;
use crate::view::{DefId, Pos, UnitDef};

/// What the list needs beyond the facts: the plans computed this pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plans {
    pub rebuild: Option<DefId>,
    pub fort: Option<DefId>,
    pub outpost: bool,
    pub outpost_raided: bool,
    /// The maker ceiling (SPEC §4.9).
    pub maker_ceiling: i32,
    /// A free deposit exists on our side (rule E).
    pub free_deposit: bool,
    /// Outlying economy sites (timed opening, §4.13).
    pub outlying: i32,
}

/// Builds the list for one builder.
pub struct ListCtx<'a> {
    pub k: &'a Knobs,
    pub c: Census<'a>,
    pub f: &'a Facts,
    pub p: Plans,
    pub b: &'a Builder,
    pub bdef: &'a UnitDef,
    /// Living constructors with a lower id than the builder (rule E).
    pub lower_constructors: i32,
    /// Recent building losses, most recent first (types).
    pub lost: &'a [DefId],
    /// Geothermal vents on the map.
    pub vents: i32,
    /// Free-standing stores exist next to a tier-two factory (rule 34).
    pub t2_factory_without_store: (bool, bool),
}

struct L<'a> {
    out: Vec<DefId>,
    bdef: &'a UnitDef,
}

impl L<'_> {
    fn push(&mut self, d: Option<DefId>) {
        if let Some(d) = d {
            if self.bdef.can_build(d) && !self.out.contains(&d) {
                self.out.push(d);
            }
        }
    }
}

/// The at-base list (SPEC §4.3), then rule E and loss promotion; or the
/// stranded list for a builder away from base or a construction ship.
pub fn build(x: &ListCtx<'_>) -> Vec<DefId> {
    let stranded = !x.b.at_base
        || (x.c.is(x.b.def, Role::ConstructionShip) && !x.k.navy.ship_builders_plan_base);
    if stranded {
        return stranded_list(x);
    }
    let mut l = at_base(x);
    // Recent losses, most recent first, move to the front.
    for &d in x.lost.iter().rev() {
        if let Some(i) = l.iter().position(|&e| e == d) {
            let e = l.remove(i);
            l.insert(0, e);
        }
    }
    // Rule E: expansion constructors.
    let rule_e = x.c.is(x.b.def, Role::Constructor)
        && !x.b.commander
        && x.b.at_base
        && x.lower_constructors >= x.k.build.constructors
        && x.p.free_deposit;
    if rule_e {
        if let Some(mex) = x.c.def(Role::Mex) {
            if x.bdef.can_build(mex) {
                l.retain(|&d| d != mex);
                l.insert(0, mex);
            }
        }
    }
    l
}

#[allow(clippy::too_many_lines)]
fn at_base(x: &ListCtx<'_>) -> Vec<DefId> {
    let (k, c, f, p) = (x.k, &x.c, x.f, &x.p);
    let mut l = L {
        out: Vec::new(),
        bdef: x.bdef,
    };
    let d = |r: Role| c.def(r);
    let fleet = f.fleet_target > 0;
    let extractors = c.tot(Role::Mex) + c.tot(Role::UnderwaterMex) + c.tot(Role::Moho);
    let dry = c.tot(Role::Mex) + c.tot(Role::Moho);
    let solars = c.tot(Role::Solar);
    let tidal = c.tot(Role::Tidal);
    let lab = c.has(Role::KbotLab);
    let labs = c.tot(Role::KbotLab);
    let shipyards = c.tot(Role::Shipyard);
    let factory = !c.own.factories.is_empty();
    let adv_lab = c.has(Role::AdvancedKbotLab);
    let mex = |l: &mut L<'_>| {
        l.push(d(Role::Mex));
        if f.wet_patches {
            l.push(d(Role::UnderwaterMex));
        }
    };
    let time = f.tick;
    let timed = k.open.timed;

    // 0. Energy stalled.
    if f.energy_stalled {
        l.push(d(Role::Solar));
        if fleet {
            l.push(d(Role::Tidal));
        }
    }
    // 1-2. The opening.
    if extractors < k.open.mex {
        mex(&mut l);
    }
    if solars < k.open.solar {
        l.push(d(Role::Solar));
    }
    // 3. Timed opening: the first lab.
    if timed && time >= secs(k.open.factory_by_s) && f.quiet && labs == 0 && shipyards == 0 {
        l.push(d(Role::KbotLab));
    }
    // 4-5. Timed opening: expansion extractors.
    if timed && time >= secs(k.open.expand_by_s) && p.outlying < k.open.expand_sites {
        l.push(d(Role::Mex));
        if f.wet_patches {
            l.push(d(Role::UnderwaterMex));
        }
    }
    // 6. Metal short.
    if f.metal_short && extractors < k.eco.mex {
        mex(&mut l);
    }
    // 7. The first kbot lab.
    if labs == 0 && !f.land_pointless {
        l.push(d(Role::KbotLab));
    }
    // 8. Early shipyard.
    if k.navy.early_shipyard
        && fleet
        && shipyards < k.navy.shipyards
        && (lab || f.land_pointless)
        && solars >= k.open.solar
        && dry >= k.open.mex
    {
        l.push(d(Role::Shipyard));
    }
    // 9. Late-position lab.
    if f.land_pointless && !f.lab_pointless && labs == 0 {
        l.push(d(Role::KbotLab));
    }
    // 10. Reactive AA.
    if f.air_threat && c.tot(Role::AntiAirTower) < k.def.reactive_aa_towers {
        l.push(d(Role::AntiAirTower));
    }
    // 11. Makers.
    let makers = c.tot(Role::MetalMaker) + c.tot(Role::FloatingMaker);
    let surplus_maker = k.eco.makers_surplus_max > 0 && f.energy_surplus > SURPLUS_MAKER_ENERGY;
    if lab && f.energy_rich && (f.metal_short || surplus_maker) && makers < p.maker_ceiling {
        l.push(d(Role::MetalMaker));
        l.push(d(Role::FloatingMaker));
    }
    // 12. Radar.
    if c.tot(Role::Radar) < k.build.radar && solars >= k.open.solar {
        l.push(d(Role::Radar));
    }
    // 13. Rebuild a lost tower.
    if p.rebuild.is_some() && (!f.metal_short || f.near_base) {
        l.push(p.rebuild);
    }
    // 14. Light laser towers at base.
    let base_llt = base_towers(x);
    if !f.metal_short && base_llt < k.def.towers && lab {
        l.push(d(Role::LightLaserTower));
    }
    // 15. Fortification.
    if p.fort.is_some() && (!f.metal_short || f.near_base) {
        l.push(p.fort);
    }
    // 16. Outpost tower.
    if p.outpost && lab && (!f.metal_short || p.outpost_raided) {
        l.push(d(Role::LightLaserTower));
    }
    // 17. Standing AA.
    let aa_want = if f.air_threat {
        k.def.reactive_aa_towers
    } else {
        k.def.base_aa_towers
    };
    if !f.metal_short && lab && c.tot(Role::AntiAirTower) < aa_want {
        l.push(d(Role::AntiAirTower));
    }
    // 18. Geothermal.
    if x.vents > 0 && (lab || c.has(Role::Shipyard)) && c.tot(Role::Geothermal) < x.vents {
        l.push(d(Role::Geothermal));
    }
    // 19. Advanced kbot lab.
    let tier_two_wanted = tier_two_wanted(x);
    if tier_two_wanted {
        l.push(d(Role::AdvancedKbotLab));
    }
    // 20. Hover plant.
    if hover_worth(x) && factory && c.tot(Role::HoverPlant) < k.build.hover_plants {
        l.push(d(Role::HoverPlant));
    }
    // 21. Early vehicle plant.
    let opening_energy = solars + tidal >= k.open.solar;
    let vp_first = k.build.vehicle_plant_first && !f.air_matters && !f.land_pointless;
    let vps = c.tot(Role::VehiclePlant);
    if vp_first && !f.metal_short && factory && opening_energy && vps < k.build.vehicle_plants {
        l.push(d(Role::VehiclePlant));
    }
    // 22. Air plant.
    let vp_done = !vp_first || c.done(Role::VehiclePlant) >= k.build.vehicle_plants;
    let radar_done = c.done(Role::Radar) >= k.build.radar.min(1);
    let air_plants = c.tot(Role::AirPlant);
    if factory
        && air_plants < k.build.air_plants
        && opening_energy
        && (f.air_matters || dry >= k.eco.mex)
        && vp_done
        && (f.air_matters || radar_done)
    {
        l.push(d(Role::AirPlant));
    }
    // 23. Advanced air plant.
    if k.tech.enabled
        && f.tech_income
        && f.air_worth
        && c.has(Role::AirPlant)
        && c.has(Role::AirConstructor)
        && c.tot(Role::AdvancedAirPlant) < k.build.adv_air_plants
    {
        l.push(d(Role::AdvancedAirPlant));
    }
    // 24. Air repair pad.
    if c.has(Role::AdvancedAirPlant) && c.tot(Role::AirRepairPad) < k.build.repair_pads {
        l.push(d(Role::AirRepairPad));
    }
    // 25. Solars to the target.
    if solars < k.eco.solar && !(k.eco.solar_on_demand && f.energy_rich) {
        l.push(d(Role::Solar));
    }
    // 26. Dry extractors.
    if dry < k.eco.mex {
        l.push(d(Role::Mex));
    }
    // 27. Late vehicle plant.
    if !f.metal_short && !f.land_pointless && c.has(Role::AirPlant) && vps < k.build.vehicle_plants
    {
        l.push(d(Role::VehiclePlant));
    }
    // 28. Extra factory while capacity is short.
    if f.capacity_short && !f.metal_short && factory {
        let extra = k.eco.capacity_factories;
        if f.land_pointless {
            if shipyards < k.navy.shipyards + extra {
                l.push(d(Role::Shipyard));
            }
        } else if d(Role::VehiclePlant).is_some_and(|v| x.bdef.can_build(v))
            && vps < k.build.vehicle_plants + extra
        {
            l.push(d(Role::VehiclePlant));
        } else if !f.lab_pointless && labs < 1 + extra {
            l.push(d(Role::KbotLab));
        }
    }
    // 29-33. Naval.
    if fleet && shipyards < k.navy.shipyards {
        l.push(d(Role::Shipyard));
    }
    if fleet && tidal < k.eco.tidal {
        l.push(d(Role::Tidal));
    }
    if fleet && c.tot(Role::Sonar) < k.navy.sonar {
        l.push(d(Role::Sonar));
    }
    if fleet && f.enemy_ship && c.tot(Role::TorpedoLauncher) < k.navy.torpedo_launchers {
        l.push(d(Role::TorpedoLauncher));
    }
    if fleet
        && c.has(Role::Shipyard)
        && c.tot(Role::AdvancedShipyard) < k.navy.adv_yards
        && f.income >= i64::from(k.navy.tech_income)
    {
        l.push(d(Role::AdvancedShipyard));
    }
    // 34. Storage.
    if factory {
        let (t2_m, t2_e) = x.t2_factory_without_store;
        let ms = c.tot(Role::MetalStorage) + c.tot(Role::UnderwaterMetalStorage);
        if ms < k.eco.metal_storage && (t2_m || f.metal_pegged) {
            l.push(d(Role::MetalStorage));
            l.push(d(Role::UnderwaterMetalStorage));
        }
        let es = c.tot(Role::EnergyStorage) + c.tot(Role::UnderwaterEnergyStorage);
        if es < k.eco.energy_storage && (t2_e || f.energy_fill >= ENERGY_STORE_FILL) {
            l.push(d(Role::EnergyStorage));
            l.push(d(Role::UnderwaterEnergyStorage));
        }
    }
    // 35. Spending tier two.
    if adv_lab {
        let under_attack = f.near_base || !c.own.building_losses.is_empty();
        let hlt = c.tot(Role::HeavyLaserTower);
        if under_attack && hlt < k.def.heavy_towers {
            l.push(d(Role::HeavyLaserTower));
        }
        if f.tech_income {
            l.push(d(Role::Moho));
        }
        if c.tot(Role::AdvancedRadar) < k.build.adv_radar {
            l.push(d(Role::AdvancedRadar));
        }
        let fusion = c.tot(Role::Fusion);
        if fusion < k.eco.fusion
            && (f.energy_short || f.metal_full || (f.tech_income && fusion == 0))
        {
            l.push(d(Role::Fusion));
        }
        if !f.metal_short && hlt < k.def.heavy_towers {
            l.push(d(Role::HeavyLaserTower));
        }
        if f.tech_income && c.tot(Role::HeavyPlasmaTower) < k.def.heavy_towers {
            l.push(d(Role::HeavyPlasmaTower));
        }
    }
    // 36. Extra kbot lab.
    if f.metal_full && !tier_two_wanted && labs < 1 + k.eco.extra_labs {
        l.push(d(Role::KbotLab));
    }
    // 37. Surplus expansion.
    if k.eco.surplus_expand && f.metal_full {
        let step = i64::from(k.eco.surplus_factory_step);
        let grow = if step > 0 { f.income / step } else { 0 };
        let allowed = i64::from(k.eco.surplus_factory_cap).min(1 + grow);
        let allowed = i32::try_from(allowed).unwrap_or(i32::MAX);
        if f.income >= SURPLUS_TECH_INCOME && c.tot(Role::AdvancedKbotLab) == 0 {
            l.push(d(Role::AdvancedKbotLab));
        }
        if fleet {
            if shipyards < allowed {
                l.push(d(Role::Shipyard));
            }
            if air_plants == 0 {
                l.push(d(Role::AirPlant));
            }
        } else {
            if labs < allowed {
                l.push(d(Role::KbotLab));
            }
            if vps < allowed {
                l.push(d(Role::VehiclePlant));
            }
        }
    }
    // 38-39. Extractors, always.
    l.push(d(Role::Mex));
    if f.wet_patches {
        l.push(d(Role::UnderwaterMex));
    }
    l.out
}

/// Energy storage is wanted at this fill (SPEC §4.3 rule 34), permille.
const ENERGY_STORE_FILL: i64 = 900;
/// A surplus maker needs this much spare energy per second (SPEC §4.9).
pub const SURPLUS_MAKER_ENERGY: i64 = 75;
/// A full store at this income forces the advanced lab (SPEC §4.3 rule 37a).
const SURPLUS_TECH_INCOME: i64 = 20;

/// Light laser towers counted at base: all of them less the outposts.
fn base_towers(x: &ListCtx<'_>) -> i32 {
    let Some(llt) = x.c.def(Role::LightLaserTower) else {
        return 0;
    };
    let home = x.f.home.unwrap_or_default();
    let outposts =
        x.c.own
            .units
            .iter()
            .filter(|u| u.unit.def == llt && !u.unit.pos.within(home, OUTPOST_DISTANCE))
            .count();
    x.c.tot(Role::LightLaserTower) - i32::try_from(outposts).unwrap_or(0)
}

/// The tier-two step is still wanted (SPEC §4.3 rule 19).
pub fn tier_two_wanted(x: &ListCtx<'_>) -> bool {
    let (k, c, f) = (x.k, &x.c, x.f);
    k.tech.enabled
        && f.tier_worth
        && f.tech_income
        && c.has(Role::KbotLab)
        && c.def(Role::AdvancedKbotLab).is_some()
        && c.tot(Role::AdvancedKbotLab) < k.build.adv_labs
}

/// Hover worth it (SPEC §5.9): hover plants allowed, the army needs a
/// ferry, a hover route exists, and income at the tech mark.
fn hover_worth(x: &ListCtx<'_>) -> bool {
    x.k.build.hover_plants > 0 && x.f.needs_ferry && x.f.hover_route && x.f.tech_income
}

/// The stranded-builder list (SPEC §4.3).
fn stranded_list(x: &ListCtx<'_>) -> Vec<DefId> {
    let (k, c, f) = (x.k, &x.c, x.f);
    let mut l = L {
        out: Vec::new(),
        bdef: x.bdef,
    };
    let d = |r: Role| c.def(r);
    let fleet = f.fleet_target > 0;
    let adv_yard = c.has(Role::AdvancedShipyard);
    if fleet
        && c.has(Role::Shipyard)
        && c.tot(Role::AdvancedShipyard) < k.navy.adv_yards
        && f.income >= i64::from(k.navy.tech_income)
    {
        l.push(d(Role::AdvancedShipyard));
    }
    if k.eco.surplus_expand
        && f.metal_full
        && c.tot(Role::AirPlant) == 0
        && adv_yard
        && c.tot(Role::SeaplanePlant) < k.navy.seaplane_plants
    {
        l.push(d(Role::SeaplanePlant));
    }
    let reactors = c.done(Role::Fusion) + c.done(Role::UnderwaterFusion);
    if adv_yard
        && f.income >= i64::from(k.navy.tech_income)
        && c.tot(Role::UnderwaterFusion) < k.eco.uw_fusion
        && (reactors == 0 || f.energy_short)
    {
        l.push(d(Role::UnderwaterFusion));
    }
    if !c.own.factories.is_empty() {
        let (t2_m, t2_e) = x.t2_factory_without_store;
        if c.tot(Role::UnderwaterMetalStorage) < k.eco.metal_storage && (t2_m || f.metal_pegged) {
            l.push(d(Role::UnderwaterMetalStorage));
        }
        if c.tot(Role::UnderwaterEnergyStorage) < k.eco.energy_storage
            && (t2_e || f.energy_fill >= ENERGY_STORE_FILL)
        {
            l.push(d(Role::UnderwaterEnergyStorage));
        }
    }
    l.push(d(Role::Mex));
    l.push(d(Role::UnderwaterMex));
    l.push(d(Role::Solar));
    l.out
}

/// Positions of our extractors further than the outpost distance (§4.13).
pub fn outlying(c: &Census<'_>, home: Option<Pos>) -> i32 {
    let Some(home) = home else { return 0 };
    let n = c
        .own
        .units
        .iter()
        .filter(|u| {
            (c.is(u.unit.def, Role::Mex) || c.is(u.unit.def, Role::UnderwaterMex))
                && !u.unit.pos.within(home, OUTPOST_DISTANCE)
        })
        .count();
    i32::try_from(n).unwrap_or(i32::MAX)
}
