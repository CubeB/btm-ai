//! A tiny built-in roster per side. The ids are the stock ARM / CORE unit
//! ids used only as role labels (SPEC Appendix B); every stat is made up for
//! the test harness and none comes from game data.

use crate::view::{Amount, DefId, MoveClass, Tick, TransportDef, UnitDef, WeaponDef};

/// The two sides of the roster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Arm,
    Core,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Arm => "ARM",
            Side::Core => "CORE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Commander,
    Mex,
    Solar,
    Maker,
    MetalStore,
    EnergyStore,
    Lab,
    Ck,
    Raider,
    Rocket,
    Artillery,
    AaKbot,
    Llt,
    AaTower,
    Radar,
    Wall,
    VehiclePlant,
    ScoutVehicle,
    LightTank,
    AirPlant,
    ScoutPlane,
    Fighter,
    AirTransport,
    Shipyard,
    Destroyer,
}

const KINDS: [(Kind, &str, &str); 25] = [
    (Kind::Commander, "ARMCOM", "CORCOM"),
    (Kind::Mex, "ARMMEX", "CORMEX"),
    (Kind::Solar, "ARMSOLAR", "CORSOLAR"),
    (Kind::Maker, "ARMMAKR", "CORMAKR"),
    (Kind::MetalStore, "ARMMSTOR", "CORMSTOR"),
    (Kind::EnergyStore, "ARMESTOR", "CORESTOR"),
    (Kind::Lab, "ARMLAB", "CORLAB"),
    (Kind::Ck, "ARMCK", "CORCK"),
    (Kind::Raider, "ARMPW", "CORAK"),
    (Kind::Rocket, "ARMROCK", "CORSTORM"),
    (Kind::Artillery, "ARMHAM", "CORTHUD"),
    (Kind::AaKbot, "ARMJETH", "CORCRASH"),
    (Kind::Llt, "ARMLLT", "CORLLT"),
    (Kind::AaTower, "ARMRL", "CORRL"),
    (Kind::Radar, "ARMRAD", "CORRAD"),
    (Kind::Wall, "ARMDRAG", "CORDRAG"),
    (Kind::VehiclePlant, "ARMVP", "CORVP"),
    (Kind::ScoutVehicle, "ARMFAV", "CORFAV"),
    (Kind::LightTank, "ARMFLASH", "CORGATOR"),
    (Kind::AirPlant, "ARMAP", "CORAP"),
    (Kind::ScoutPlane, "ARMPEEP", "CORFINK"),
    (Kind::Fighter, "ARMFIG", "CORVENG"),
    (Kind::AirTransport, "ARMATLAS", "CORVALK"),
    (Kind::Shipyard, "ARMSY", "CORSY"),
    (Kind::Destroyer, "ARMROY", "CORROY"),
];

const BASE_MENU: &[Kind] = &[
    Kind::Mex,
    Kind::Solar,
    Kind::Maker,
    Kind::MetalStore,
    Kind::EnergyStore,
    Kind::Lab,
    Kind::VehiclePlant,
    Kind::AirPlant,
    Kind::Shipyard,
    Kind::Llt,
    Kind::AaTower,
    Kind::Radar,
    Kind::Wall,
];

fn menu(k: Kind) -> Option<&'static [Kind]> {
    match k {
        Kind::Commander | Kind::Ck => Some(BASE_MENU),
        Kind::Lab => Some(&[
            Kind::Ck,
            Kind::Raider,
            Kind::Rocket,
            Kind::Artillery,
            Kind::AaKbot,
        ]),
        Kind::VehiclePlant => Some(&[Kind::ScoutVehicle, Kind::LightTank]),
        Kind::AirPlant => Some(&[Kind::ScoutPlane, Kind::Fighter, Kind::AirTransport]),
        Kind::Shipyard => Some(&[Kind::Destroyer]),
        _ => None,
    }
}

const fn mc(max_slope: i32, min_depth: i32, max_depth: i32, hovers: bool) -> MoveClass {
    MoveClass {
        max_slope,
        min_depth,
        max_depth,
        hovers,
    }
}

const KBOT: MoveClass = mc(30, 0, 20, false);
const TANK: MoveClass = mc(18, 0, 15, false);
const COMMANDER: MoveClass = mc(36, 0, 1000, false);
const SHIP: MoveClass = mc(1000, 15, 100_000, false);
const AIR: MoveClass = mc(100_000, 0, 100_000, true);
const LAND_BUILDING: MoveClass = mc(20, 0, 0, false);
const SEA_BUILDING: MoveClass = mc(1000, 30, 100_000, false);

fn weapon(name: &str, range: i32, damage: i32, reload: Tick) -> WeaponDef {
    WeaponDef {
        name: name.to_string(),
        range,
        damage,
        burst: 1,
        reload,
        ..WeaponDef::default()
    }
}

fn building(m: i64, e: i64, bt: i64, hp: i32, fp: i32) -> UnitDef {
    UnitDef {
        cost_metal: Amount::units(m),
        cost_energy: Amount::units(e),
        build_time: bt,
        health: hp,
        footprint: (fp, fp),
        move_class: LAND_BUILDING,
        sight: 200,
        ..UnitDef::default()
    }
}

fn mobile(m: i64, e: i64, bt: i64, hp: i32, speed: i32, mc: MoveClass) -> UnitDef {
    UnitDef {
        cost_metal: Amount::units(m),
        cost_energy: Amount::units(e),
        build_time: bt,
        health: hp,
        speed,
        footprint: (2, 2),
        move_class: mc,
        sight: 350,
        mobile: true,
        can_be_transported: true,
        size: 1,
        ..UnitDef::default()
    }
}

fn template(k: Kind) -> UnitDef {
    match k {
        Kind::Commander => {
            let mut d = mobile(2500, 25000, 75000, 3000, 40, COMMANDER);
            d.commander = true;
            d.builder = true;
            d.worker_rate = 300;
            d.sight = 450;
            d.radar = 700;
            d.size = 3;
            d.metal_make = Amount(1500);
            d.energy_make = Amount::units(25);
            d.metal_storage = Amount::units(1000);
            d.energy_storage = Amount::units(1000);
            let mut dgun = weapon("DGUN", 250, 5000, 30);
            dgun.energy_per_shot = Amount::units(400);
            let mut uw = weapon("COMTORP", 280, 40, 30);
            uw.underwater = true;
            d.weapons = vec![weapon("COMLASER", 300, 50, 15), uw, dgun];
            d
        }
        Kind::Mex => {
            let mut d = building(50, 500, 1800, 300, 2);
            d.extracts_metal = 1000;
            d
        }
        Kind::Solar => {
            let mut d = building(150, 0, 2800, 500, 3);
            d.energy_make = Amount::units(20);
            d
        }
        Kind::Maker => {
            let mut d = building(1, 800, 2000, 200, 2);
            d.on_off = true;
            d.energy_use = Amount::units(60);
            d.metal_make = Amount::units(1);
            d
        }
        Kind::MetalStore => {
            let mut d = building(300, 500, 3000, 1000, 3);
            d.metal_storage = Amount::units(1000);
            d
        }
        Kind::EnergyStore => {
            let mut d = building(200, 1000, 3000, 1000, 3);
            d.energy_storage = Amount::units(6000);
            d
        }
        Kind::Lab | Kind::VehiclePlant | Kind::AirPlant => {
            let mut d = building(600, 1200, 6500, 2500, 6);
            d.builder = true;
            d.worker_rate = 100;
            d
        }
        Kind::Ck => {
            let mut d = mobile(120, 1200, 3500, 600, 35, KBOT);
            d.builder = true;
            d.worker_rate = 80;
            d.sight = 300;
            d
        }
        Kind::Raider => {
            let mut d = mobile(50, 900, 1600, 300, 55, KBOT);
            d.weapons = vec![weapon("EMG", 200, 12, 10)];
            d
        }
        Kind::Rocket => {
            let mut d = mobile(100, 1300, 2400, 400, 40, KBOT);
            d.weapons = vec![weapon("ROCKET", 450, 90, 75)];
            d
        }
        Kind::Artillery => {
            let mut d = mobile(140, 1500, 3000, 600, 35, KBOT);
            let mut w = weapon("PLASMA", 600, 120, 90);
            w.lobbed = true;
            d.weapons = vec![w];
            d
        }
        Kind::AaKbot => {
            let mut d = mobile(110, 1500, 2400, 500, 45, KBOT);
            let mut w = weapon("AAMISSILE", 600, 30, 20);
            w.anti_air = true;
            d.weapons = vec![w];
            d
        }
        Kind::Llt => {
            let mut d = building(90, 900, 2000, 800, 2);
            d.sight = 400;
            let mut w = weapon("LLASER", 400, 75, 30);
            w.energy_per_shot = Amount::units(15);
            d.weapons = vec![w];
            d
        }
        Kind::AaTower => {
            let mut d = building(80, 800, 1800, 600, 2);
            let mut w = weapon("AATOWER", 700, 40, 30);
            w.anti_air = true;
            d.weapons = vec![w];
            d
        }
        Kind::Radar => {
            let mut d = building(50, 600, 1100, 200, 2);
            d.sight = 400;
            d.radar = 2000;
            d
        }
        Kind::Wall => building(8, 0, 200, 2000, 2),
        Kind::ScoutVehicle => {
            let mut d = mobile(30, 300, 900, 120, 100, TANK);
            d.sight = 500;
            d.weapons = vec![weapon("FAVLASER", 160, 6, 10)];
            d
        }
        Kind::LightTank => {
            let mut d = mobile(110, 1000, 2000, 700, 70, TANK);
            d.weapons = vec![weapon("FLAMER", 220, 15, 6)];
            d
        }
        Kind::ScoutPlane => {
            let mut d = mobile(30, 1000, 900, 100, 300, AIR);
            d.flies = true;
            d.can_be_transported = false;
            d.sight = 600;
            d
        }
        Kind::Fighter => {
            let mut d = mobile(80, 2500, 2500, 200, 280, AIR);
            d.flies = true;
            d.can_be_transported = false;
            let mut w = weapon("AAGUN", 400, 25, 15);
            w.anti_air = true;
            d.weapons = vec![w];
            d
        }
        Kind::AirTransport => {
            let mut d = mobile(70, 1500, 2000, 300, 200, AIR);
            d.flies = true;
            d.can_be_transported = false;
            d.transport = Some(TransportDef {
                capacity: 1,
                max_size: 3,
            });
            d
        }
        Kind::Shipyard => {
            let mut d = building(600, 1000, 6000, 3000, 8);
            d.builder = true;
            d.worker_rate = 100;
            d.floats = true;
            d.move_class = SEA_BUILDING;
            d
        }
        Kind::Destroyer => {
            let mut d = mobile(900, 6000, 12000, 3000, 50, SHIP);
            d.floats = true;
            d.can_be_transported = false;
            d.footprint = (4, 4);
            d.weapons = vec![weapon("SHIPGUN", 500, 150, 45)];
            d
        }
    }
}

/// Every definition, ARM first, then CORE, in [`KINDS`] order.
pub fn roster() -> Vec<UnitDef> {
    let per_side = KINDS.len();
    let id_of = |side: usize, k: Kind| {
        let i = KINDS.iter().position(|(kk, _, _)| *kk == k).unwrap_or(0);
        DefId(u16::try_from(side * per_side + i).unwrap_or(0))
    };
    let mut out = Vec::with_capacity(per_side * 2);
    for side in 0..2 {
        for &(k, arm, core) in &KINDS {
            let mut d = template(k);
            d.name = if side == 0 { arm } else { core }.to_string();
            d.build_menu = menu(k).map(|m| m.iter().map(|&x| id_of(side, x)).collect());
            out.push(d);
        }
    }
    out
}

/// The commander's unit id for a side.
pub fn commander_name(side: Side) -> &'static str {
    match side {
        Side::Arm => KINDS[0].1,
        Side::Core => KINDS[0].2,
    }
}
