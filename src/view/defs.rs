//! Unit and weapon definitions as plain data (SPEC §1.2, "Game data").

use super::types::{Amount, DefId, Tick, TICKS_PER_SECOND};

/// Slope and depth limits of a movement class (SPEC §1.2, §3.4).
///
/// For a mobile unit this is how it moves; for a building it is where it may
/// be placed. Depth is `sea level − ground height`, 0 on dry land.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MoveClass {
    /// Largest height step (world units) to a 4-neighbour tile it accepts.
    pub max_slope: i32,
    /// Least water depth it needs (ships > 0, land units 0).
    pub min_depth: i32,
    /// Greatest water depth it accepts.
    pub max_depth: i32,
    /// Crosses water of any depth on the surface (hovercraft).
    pub hovers: bool,
}

impl MoveClass {
    /// Whether a tile with this slope and water depth is standable.
    pub fn accepts(&self, slope: i32, depth: i32) -> bool {
        if slope > self.max_slope {
            return false;
        }
        if self.hovers {
            return true;
        }
        depth >= self.min_depth && depth <= self.max_depth
    }
}

/// A weapon definition (SPEC §1.2 weapon row).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct WeaponDef {
    pub name: String,
    /// Range in world units.
    pub range: i32,
    /// Damage per hit against the default armour.
    pub damage: i32,
    /// Shots per salvo; 0 or 1 both mean one.
    pub burst: i32,
    /// Ticks between salvoes.
    pub reload: Tick,
    pub anti_air: bool,
    pub lobbed: bool,
    pub dropped: bool,
    pub underwater: bool,
    pub energy_per_shot: Amount,
}

impl WeaponDef {
    /// Damage per second in milli-hitpoints: damage × max(1, burst) /
    /// max(0.1 s, reload) (SPEC §3.3), in integer arithmetic.
    pub fn dps_milli(&self) -> i64 {
        let reload = i64::from(self.reload.max(TICKS_PER_SECOND / 10));
        i64::from(self.damage) * i64::from(self.burst.max(1)) * i64::from(TICKS_PER_SECOND) * 1000
            / reload
    }
}

/// What a transport can carry (SPEC §1.2 flags, §5.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TransportDef {
    /// Number of units it can carry at once.
    pub capacity: u8,
    /// Largest [`UnitDef::size`] it can lift.
    pub max_size: u8,
}

/// A unit definition (SPEC §1.2 unit definition and build menu rows).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UnitDef {
    /// The game's unit id, e.g. `ARMMEX`. Role tables match it ignoring case.
    pub name: String,
    pub cost_metal: Amount,
    pub cost_energy: Amount,
    /// Work needed to build one, in work units.
    pub build_time: i64,
    /// Work this unit adds per second when building (0 for non-builders).
    /// A builder adds `worker_rate` per tick in [`super::Progress`] units.
    pub worker_rate: i64,
    pub health: i32,
    /// Top speed in world units per second (0 for buildings).
    pub speed: i32,
    /// Footprint in tiles, x by z.
    pub footprint: (i32, i32),
    /// Movement class of a mobile unit, placement limits of a building.
    pub move_class: MoveClass,
    /// Line-of-sight radius in world units.
    pub sight: i32,
    /// Radar radius in world units (0: no radar). SPEC flag "radar".
    pub radar: i32,
    pub mobile: bool,
    pub builder: bool,
    pub flies: bool,
    pub commander: bool,
    pub floats: bool,
    /// Can be switched on and off (metal makers).
    pub on_off: bool,
    pub air_base: bool,
    pub can_be_transported: bool,
    /// Transport size class of this unit.
    pub size: u8,
    pub transport: Option<TransportDef>,
    /// Extraction factor in permille of the metal under the footprint
    /// (0: not an extractor). SPEC flag "extracts metal".
    pub extracts_metal: i32,
    /// Metal made per second while on (makers, commanders). SPEC "makes metal".
    pub metal_make: Amount,
    /// Energy made per second (solars, commanders).
    pub energy_make: Amount,
    /// Energy used per second while on (makers). SPEC "energy use".
    pub energy_use: Amount,
    pub metal_storage: Amount,
    pub energy_storage: Amount,
    pub weapons: Vec<WeaponDef>,
    /// What this unit can build. `None` means no known menu: it may build
    /// anything (SPEC §1.2 build menus row).
    pub build_menu: Option<Vec<DefId>>,
}

impl UnitDef {
    pub fn armed(&self) -> bool {
        !self.weapons.is_empty()
    }

    /// An immobile builder is a factory (SPEC §3.2 rule 1).
    pub fn is_factory(&self) -> bool {
        self.builder && !self.mobile
    }

    /// Longest weapon range, 0 when unarmed.
    pub fn max_range(&self) -> i32 {
        self.weapons.iter().map(|w| w.range).max().unwrap_or(0)
    }

    /// Whether the build menu allows `def` (SPEC §1.2: no menu, anything).
    pub fn can_build(&self, def: DefId) -> bool {
        self.builder
            && self
                .build_menu
                .as_ref()
                .is_none_or(|menu| menu.contains(&def))
    }
}
