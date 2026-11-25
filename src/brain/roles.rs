//! The role table of SPEC Appendix B: which unit definition fills each role
//! for the side this AI plays.
//!
//! The side name is compared ignoring case. CORE gets the CORE column and any
//! other side the ARM column. An id missing from the loaded definitions leaves
//! its role empty, and every rule that names an empty role does nothing. The
//! air repair pad role is emptied unless its definition is both an air base
//! and a builder.

use crate::view::{DefId, GameView};

macro_rules! roles {
    ($( $(#[$m:meta])* $role:ident = $arm:literal, $core:literal; )*) => {
        /// A role of SPEC Appendix B.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Role {
            $( $(#[$m])* $role, )*
        }

        impl Role {
            /// Every role, in table order.
            pub const ALL: &'static [Role] = &[ $( Role::$role, )* ];

            /// The stock (ARM, CORE) unit ids of this role.
            pub const fn stock_ids(self) -> (&'static str, &'static str) {
                match self {
                    $( Role::$role => ($arm, $core), )*
                }
            }
        }
    };
}

roles! {
    Mex = "ARMMEX", "CORMEX";
    Solar = "ARMSOLAR", "CORSOLAR";
    /// The first kbot lab.
    KbotLab = "ARMLAB", "CORLAB";
    /// The level-one constructor (kbot).
    Constructor = "ARMCK", "CORCK";
    RaiderKbot = "ARMPW", "CORAK";
    RocketKbot = "ARMROCK", "CORSTORM";
    LightLaserTower = "ARMLLT", "CORLLT";
    Radar = "ARMRAD", "CORRAD";
    /// The land metal maker.
    MetalMaker = "ARMMAKR", "CORMAKR";
    AirPlant = "ARMAP", "CORAP";
    ScoutPlane = "ARMPEEP", "CORFINK";
    AirTransport = "ARMATLAS", "CORVALK";
    Fighter = "ARMFIG", "CORVENG";
    Bomber = "ARMTHUND", "CORSHAD";
    AdvancedAirPlant = "ARMAAP", "CORAAP";
    Gunship = "ARMBRAWL", "CORAPE";
    AirRepairPad = "ARMASP", "CORASP";
    AirConstructor = "ARMCA", "CORCA";
    VehiclePlant = "ARMVP", "CORVP";
    ScoutVehicle = "ARMFAV", "CORFAV";
    LightTank = "ARMFLASH", "CORGATOR";
    MissileTruck = "ARMSAM", "CORMIST";
    MediumTank = "ARMSTUMP", "CORRAID";
    ArtilleryKbot = "ARMHAM", "CORTHUD";
    /// Dragon's teeth.
    Wall = "ARMDRAG", "CORDRAG";
    AntiAirTower = "ARMRL", "CORRL";
    /// "Mobile anti-air" of SPEC §3.2.
    AntiAirKbot = "ARMJETH", "CORCRASH";
    AdvancedKbotLab = "ARMALAB", "CORALAB";
    AdvancedConstructor = "ARMACK", "CORACK";
    AssaultKbot = "ARMZEUS", "CORCAN";
    HeavyLaserTower = "ARMHLT", "CORHLT";
    HeavyPlasmaTower = "ARMGUARD", "CORPUN";
    AdvancedRadar = "ARMARAD", "CORARAD";
    /// The tier-two extractor.
    Moho = "ARMMOHO", "CORMOHO";
    Fusion = "ARMFUS", "CORFUS";
    Shipyard = "ARMSY", "CORSY";
    ConstructionShip = "ARMCS", "CORCS";
    ScoutShip = "ARMPT", "CORPT";
    Destroyer = "ARMROY", "CORROY";
    SeaTransport = "ARMTSHIP", "CORTSHIP";
    Submarine = "ARMSUB", "CORSUB";
    AdvancedShipyard = "ARMASY", "CORASY";
    Cruiser = "ARMCRUS", "CORCRUS";
    Battleship = "ARMBATS", "CORBATS";
    AntiAirShip = "ARMAAS", "CORARCH";
    AdvancedConstructionSub = "ARMACSUB", "CORACSUB";
    SeaplanePlant = "ARMPLAT", "CORPLAT";
    SeaplaneFighter = "ARMSFIG", "CORSFIG";
    TorpedoSeaplane = "ARMSEAP", "CORSEAP";
    HoverPlant = "ARMHP", "CORHP";
    HoverTank = "ARMANAC", "CORSNAP";
    HoverAntiAir = "ARMAH", "CORAH";
    Tidal = "ARMTIDE", "CORTIDE";
    Sonar = "ARMSONAR", "CORSONAR";
    TorpedoLauncher = "ARMTL", "CORTL";
    UnderwaterMex = "ARMUWMEX", "CORUWMEX";
    FloatingMaker = "ARMFMKR", "CORFMKR";
    Geothermal = "ARMGEO", "CORGEO";
    MetalStorage = "ARMMSTOR", "CORMSTOR";
    EnergyStorage = "ARMESTOR", "CORESTOR";
    UnderwaterMetalStorage = "ARMUWMS", "CORUWMS";
    UnderwaterEnergyStorage = "ARMUWES", "CORUWES";
    UnderwaterFusion = "ARMUWFUS", "CORUWFUS";
}

/// The resolved role table for one side. Lookups both ways are by sorted
/// vectors, never hash order (SPEC §7).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleTable {
    /// `(role, def)` sorted by role.
    by_role: Vec<(Role, DefId)>,
    /// `(def, role)` sorted by def, then role.
    by_def: Vec<(DefId, Role)>,
}

impl RoleTable {
    /// Resolves the table for a side name (`ARM`, `CORE`, ...).
    pub fn resolve(view: &dyn GameView, side: &str) -> RoleTable {
        let core = side.eq_ignore_ascii_case("CORE");
        let mut by_role = Vec::new();
        for &role in Role::ALL {
            let (arm, core_id) = role.stock_ids();
            let Some(def) = view.def_named(if core { core_id } else { arm }) else {
                continue;
            };
            if role == Role::AirRepairPad
                && !view.unit_def(def).is_some_and(|d| d.air_base && d.builder)
            {
                continue;
            }
            by_role.push((role, def));
        }
        by_role.sort();
        let mut by_def: Vec<(DefId, Role)> = by_role.iter().map(|&(r, d)| (d, r)).collect();
        by_def.sort();
        RoleTable { by_role, by_def }
    }

    /// The definition filling `role`, if any.
    pub fn get(&self, role: Role) -> Option<DefId> {
        self.by_role
            .binary_search_by_key(&role, |&(r, _)| r)
            .ok()
            .and_then(|i| self.by_role.get(i))
            .map(|&(_, d)| d)
    }

    /// Whether `def` fills `role`.
    pub fn is(&self, def: DefId, role: Role) -> bool {
        self.get(role) == Some(def)
    }

    /// The first role (in table order) that `def` fills.
    pub fn role_of(&self, def: DefId) -> Option<Role> {
        let i = self.by_def.partition_point(|&(d, _)| d < def);
        self.by_def
            .get(i)
            .filter(|&&(d, _)| d == def)
            .map(|&(_, r)| r)
    }

    /// A scout type: the scout plane or the scout vehicle (Appendix B).
    pub fn is_scout_type(&self, def: DefId) -> bool {
        self.is(def, Role::ScoutPlane) || self.is(def, Role::ScoutVehicle)
    }
}
