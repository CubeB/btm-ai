//! The read side of the contract: [`GameView`] (fog-honest) and [`FogLeaks`]
//! (the reference AI's leaks, named one by one).

use super::defs::{MoveClass, UnitDef};
use super::orders::Command;
use super::types::{
    Amount, DefId, FeatureId, PlayerId, Pos, TeamId, Tick, Tile, UnitId, VisCell, TILE_SIZE,
};

/// Build progress of a frame (SPEC §1.2 "build progress").
///
/// Both fields count in work-ticks: 1/30 of a work unit, so a builder with
/// worker rate W adds W per tick, and the ticks left for it are
/// `(total − done) / W` (SPEC §4.1 build seconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Progress {
    pub done: i64,
    pub total: i64,
}

/// A factory's queue and state (SPEC §1.2 "factory queue and factory state").
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FactoryState {
    /// Queued entries, head first. The head is the one being built.
    pub queue: Vec<DefId>,
    /// The frame the factory is holding, if any.
    pub building: Option<UnitId>,
}

/// One of our own units, readable in full (SPEC §1.1, §1.2 own units row).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OwnUnit {
    pub id: UnitId,
    pub def: DefId,
    pub pos: Pos,
    pub health: i32,
    /// `Some` while the unit is still a frame.
    pub frame: Option<Progress>,
    /// The order queue, head first.
    pub commands: Vec<Command>,
    /// `Some` for factories.
    pub factory: Option<FactoryState>,
    /// Units this transport carries.
    pub carrying: Vec<UnitId>,
    pub carried_by: Option<UnitId>,
    /// The on/off switch (always true for units without one).
    pub on: bool,
    /// The frame this builder is working on, if any.
    pub working_on: Option<UnitId>,
    /// Held by a mission script: count it, never order it (SPEC §3.2).
    pub scripted: bool,
}

/// An enemy unit the seat can see *now* (SPEC §1.2 "Can I see unit U now?").
///
/// Only what a human sees by looking: type, position, health bar, and whether
/// it is still a frame. Allies are never returned (SPEC §1.3 step 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EnemyUnit {
    pub id: UnitId,
    pub owner: PlayerId,
    pub def: DefId,
    pub pos: Pos,
    pub health: i32,
    pub is_frame: bool,
    /// Mission immunity: immune units are never contacts (SPEC §1.2).
    pub immune: bool,
}

/// A radar dot: an enemy under our radar but not in line of sight. It carries
/// no type (SPEC §1.2 radar row, §5.4 radar warning).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RadarDot {
    pub id: UnitId,
    pub pos: Pos,
}

/// Kind of a map feature (SPEC §1.2 features row).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FeatureKind {
    /// What is left of a dead unit of this type.
    Wreck(DefId),
    Rock,
    Tree,
}

/// A map feature (SPEC §1.2 features row, §4.12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Feature {
    pub id: FeatureId,
    pub kind: FeatureKind,
    pub pos: Pos,
    /// Footprint in tiles.
    pub footprint: (i32, i32),
    pub metal: Amount,
    pub energy: Amount,
    pub reclaimable: bool,
}

/// A building of any owner, seen or not. Only [`FogLeaks`] returns these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BuildingSighting {
    pub id: UnitId,
    pub owner: PlayerId,
    pub def: DefId,
    pub pos: Pos,
    pub is_frame: bool,
}

/// A player's public status (SPEC §1.2 players row).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerInfo {
    pub id: PlayerId,
    /// `None` means no team: allied with nobody.
    pub team: Option<TeamId>,
    pub alive: bool,
    /// Side name, e.g. `ARM` or `CORE`; compared ignoring case (Appendix B).
    pub side: String,
}

impl PlayerInfo {
    /// Same non-empty team (SPEC §1.3 step 2). A player is not its own ally.
    pub fn allied_with(&self, other: &PlayerInfo) -> bool {
        self.id != other.id && self.team.is_some() && self.team == other.team
    }
}

/// One resource of our economy (SPEC §1.2 own player row, §4.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Resource {
    pub stock: Amount,
    pub storage: Amount,
    /// Last second's income, per second.
    pub income: Amount,
    /// Last second's desired consumption, per second.
    pub demand: Amount,
    /// Demand went unmet during the last second.
    pub stalled: bool,
}

impl Resource {
    /// Stock as a permille of storage (0 when there is no storage).
    pub fn fill_permille(&self) -> i64 {
        if self.storage.0 <= 0 {
            0
        } else {
            self.stock.0.saturating_mul(1000) / self.storage.0
        }
    }
}

/// Our economy (SPEC §1.2 own player row).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Economy {
    pub metal: Resource,
    pub energy: Resource,
}

/// Public map dimensions and constants (SPEC §1.2 terrain row).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MapInfo {
    pub width_tiles: i32,
    pub height_tiles: i32,
    pub sea_level: i32,
    /// Map-wide surface metal per tile; patch tiles exceed it (SPEC §4.5).
    pub surface_metal: i32,
}

impl MapInfo {
    pub fn width(&self) -> i32 {
        self.width_tiles * TILE_SIZE
    }
    pub fn height(&self) -> i32 {
        self.height_tiles * TILE_SIZE
    }
    pub fn contains(&self, p: Pos) -> bool {
        p.x >= 0 && p.z >= 0 && p.x < self.width() && p.z < self.height()
    }
    pub fn contains_tile(&self, t: Tile) -> bool {
        t.x >= 0 && t.z >= 0 && t.x < self.width_tiles && t.z < self.height_tiles
    }
}

/// Top-left tile of a footprint of `fp` tiles centred on `at`. Builders and
/// the game must agree on this snap for "build T at P" (SPEC §2.1).
pub fn footprint_origin(fp: (i32, i32), at: Pos) -> Tile {
    let half = TILE_SIZE / 2;
    Tile::new(
        (at.x - fp.0 * half + half).div_euclid(TILE_SIZE),
        (at.z - fp.1 * half + half).div_euclid(TILE_SIZE),
    )
}

/// Everything the AI may read without cheating (SPEC §1.1, §1.2).
///
/// Own units, game data, public map data and the own economy are free.
/// Enemy units are visible only as fog allows: only units seen now (or under
/// radar, as typeless dots). The contact memory of SPEC §1.3 is built on top
/// of [`GameView::visible_enemies`]; this trait keeps no memory itself.
/// With [`GameView::full_vision`] the fog queries answer as if every cell
/// were visible (SPEC §1.1 `cheat.full_vision`).
pub trait GameView {
    // --- Time and players (SPEC §1.2, §2.2) ---

    /// The current simulation tick.
    fn tick(&self) -> Tick;
    /// The seat this view belongs to.
    fn me(&self) -> PlayerId;
    /// Every player, in id order, including us (SPEC §1.2 players row).
    fn players(&self) -> Vec<PlayerInfo>;
    /// The host granted full vision (SPEC §1.1, §6.2 Brutal).
    fn full_vision(&self) -> bool;

    // --- Own data (SPEC §1.2 own player and own units rows) ---

    /// Our units, in id order.
    fn own_units(&self) -> Vec<OwnUnit>;
    /// Our economy.
    fn economy(&self) -> Economy;

    // --- Game data (SPEC §1.2 unit, weapon and build menu rows) ---

    /// Every unit definition; a [`DefId`] indexes this slice.
    fn unit_defs(&self) -> &[UnitDef];

    /// One definition by id.
    fn unit_def(&self, id: DefId) -> Option<&UnitDef> {
        self.unit_defs().get(usize::from(id.0))
    }

    /// Looks a definition up by unit id, ignoring case (Appendix B).
    fn def_named(&self, name: &str) -> Option<DefId> {
        self.unit_defs()
            .iter()
            .position(|d| d.name.eq_ignore_ascii_case(name))
            .and_then(|i| u16::try_from(i).ok())
            .map(DefId)
    }

    // --- Public map data (SPEC §1.2 terrain and start position rows) ---

    fn map(&self) -> MapInfo;
    /// Ground height of a tile; 0 off the map.
    fn height(&self, t: Tile) -> i32;
    /// Metal value of a tile, in milli-metal per second at a 1000 permille
    /// extraction factor; 0 off the map.
    fn metal(&self, t: Tile) -> i32;
    /// Every tile whose metal exceeds the surface metal, in scan order. All
    /// patches are public from the start, as for a human (SPEC §1.4).
    fn metal_spots(&self) -> Vec<(Tile, i32)>;
    /// Whether a unit of this movement class can stand on the tile
    /// (slope, depth, impassable terrain). False off the map.
    fn standable(&self, t: Tile, class: MoveClass) -> bool;
    /// Water depth of a tile (0 on dry land).
    fn water_depth(&self, t: Tile) -> i32 {
        (self.map().sea_level - self.height(t)).max(0)
    }
    /// Geothermal vent positions.
    fn vents(&self) -> Vec<Pos>;
    /// Declared start positions, in slot order (public in the lobby).
    fn start_positions(&self) -> Vec<Pos>;
    /// Terrain-only placement test: in bounds and standable throughout the
    /// footprint. It knows nothing of buildings; see
    /// [`FogLeaks::leak_can_place`] for the occupancy test.
    fn can_place_terrain(&self, def: DefId, at: Pos) -> bool {
        let Some(d) = self.unit_def(def) else {
            return false;
        };
        let o = footprint_origin(d.footprint, at);
        (0..d.footprint.1).all(|dz| {
            (0..d.footprint.0).all(|dx| self.standable(Tile::new(o.x + dx, o.z + dz), d.move_class))
        })
    }

    // --- Fog-honest enemy knowledge (SPEC §1.2, §1.3) ---

    /// Every non-allied enemy unit seen now, in id order.
    fn visible_enemies(&self) -> Vec<EnemyUnit>;
    /// "Can I see unit U now?" `Some` only for a non-allied enemy seen now.
    fn enemy(&self, id: UnitId) -> Option<EnemyUnit>;
    /// "Is point P in my line of sight?"
    fn in_los(&self, p: Pos) -> bool;
    /// "Is point P under my radar?"
    fn in_radar(&self, p: Pos) -> bool;
    /// Non-allied enemies under our radar and out of sight, in id order.
    fn radar_dots(&self) -> Vec<RadarDot>;
    /// The per-cell "currently visible" grid (SPEC §1.2, §3.3 staleness).
    fn cell_visible(&self, c: VisCell) -> bool;
    /// Whether the cell was ever visible (SPEC §1.2 explored flag).
    fn cell_explored(&self, c: VisCell) -> bool;
    /// Features in cells visible now, in id order.
    fn visible_features(&self) -> Vec<Feature>;
    /// "Can this unit's weapons hit that target?" Answered only for our own
    /// shooter and a target seen now; otherwise false (SPEC §1.2).
    fn can_hit(&self, shooter: UnitId, target: UnitId) -> bool;
}

/// The reference AI's fog leaks (SPEC §1.4), each a named method so that every
/// use is explicit and can be audited, measured or removed. Code that takes
/// only a [`GameView`] cannot leak.
pub trait FogLeaks {
    /// Placement legality at a point: footprint, terrain and occupancy by
    /// every building and feature of every owner, seen or not (SPEC §1.4
    /// "placement tests see all buildings").
    fn leak_can_place(&self, def: DefId, at: Pos) -> bool;

    /// Every building of any owner, seen or not, with its centre within
    /// `radius` of `at`, in id order. Serves the clearance lanes (§4.6), the
    /// extractor search's avoidance of unseen enemy factories (§4.5) and the
    /// tier-two upgrade crowding test (SPEC §1.4).
    fn leak_buildings_near(&self, at: Pos, radius: i32) -> Vec<BuildingSighting>;

    /// Current contents of every feature in a cell we have ever explored,
    /// visible now or not, in id order (SPEC §1.4 "features").
    fn leak_explored_features(&self) -> Vec<Feature>;

    /// Current contents of every feature within `radius` of `at`, with no
    /// explored test at all: own wreck-site reclaim (SPEC §1.4, §4.12).
    fn leak_features_near(&self, at: Pos, radius: i32) -> Vec<Feature>;

    /// True occupancy of a geothermal vent: whether a building stands on it,
    /// seen or not (SPEC §1.4 "geothermal vents").
    fn leak_vent_occupied(&self, vent: Pos) -> bool;

    /// Whether the unit behind a radar dot is mobile, which a player's radar
    /// does not show (SPEC §1.4 "radar warning"). `None` if no such dot.
    fn leak_radar_dot_mobile(&self, id: UnitId) -> Option<bool>;

    /// Radar dots including allied mobile units, as the reference radar
    /// warning counts them (SPEC §1.4 "radar warning", §9). In id order.
    fn leak_radar_dots_with_allies(&self) -> Vec<RadarDot>;
}
