//! The world model: the one piece of state every pass reads and writes, in
//! pipeline order (SPEC §3.1).
//!
//! **Filled by the core loop** every tick, before any manager runs:
//! [`World::own`], [`World::economy`], [`World::contacts`], [`World::enemy`],
//! [`World::claims`], [`World::ownership`] (defaults and loan expiry),
//! [`World::coord`]`.commander_tasked`; [`World::grid`] and [`World::reach`]
//! when due; [`World::map`] and [`World::roles`] once, on the first tick.
//!
//! **Filled by the managers** (documented field by field): [`World::threat`]
//! and [`World::army`] by the army, [`World::eco`] by the economy, and the
//! flags of [`World::coord`] by the step named on each.
//!
//! Coordination goes through these flags, each written by one step and read
//! by later ones (SPEC §3.1). A flag a later step reads may be from this
//! tick or, for a step that runs earlier in the pipeline, from the last
//! tick it ran.

use std::collections::BTreeMap;

use super::contacts::{ContactMemory, EnemyPicture};
use super::grid::InfluenceGrid;
use super::mapintel::{MapIntel, MetalClaims};
use super::own::{EconomySummary, OwnState};
use super::ownership::Ownership;
use super::reach::Reachability;
use super::roles::RoleTable;
use super::threat::ThreatPicture;
use crate::view::{DefId, PlayerId, Pos, Tick, UnitId, VisCell};

/// The strategic phase (SPEC §5.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    #[default]
    Setup,
    Growth,
    Assault,
    Defend,
}

/// A request that combat units guard a build site (SPEC §4.2, §5.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardRequest {
    pub site: Pos,
    pub builder: UnitId,
    pub tick: Tick,
}

/// The coordination flags of SPEC §3.1.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coordination {
    /// **Economy (planner) writes, army reads:** the build-site guard
    /// request (§4.2 "issue the job", §5.6). Default: none.
    pub site_guard: Option<GuardRequest>,
    /// **Transport writes, army reads:** the ferry muster/rally point used
    /// instead of the normal rally when the army needs a sea ferry (§5.2,
    /// §5.9). Default: none.
    pub ferry_rally: Option<Pos>,
    /// **Scouts write (by loan), everyone reads:** the combat unit or hull
    /// borrowed as the scout (§5.10). The loan itself is in
    /// [`super::Ownership`]; this mirrors it for quick reads. Default: none.
    pub borrowed_scout: Option<UnitId>,
    /// **Core writes, army reads:** the economy staged an order for the
    /// commander this tick (§3.1 step 7), so the army pass should not
    /// overwrite it. Set right after the planner step.
    pub commander_tasked: bool,
    /// **Army writes (sweep), economy reads:** the commander is fleeing
    /// (§5.7); the planner gives it nothing. Default: false.
    pub commander_fleeing: bool,
    /// **Army writes (sweep), economy reads:** builders in their shelter
    /// window, with the tick the window ends (§5.7, §4.2 rule 1). Default:
    /// empty.
    pub sheltering: BTreeMap<UnitId, Tick>,
}

impl Coordination {
    /// Whether a builder is sheltering at `tick`.
    pub fn is_sheltering(&self, unit: UnitId, tick: Tick) -> bool {
        self.sheltering.get(&unit).is_some_and(|&t| tick < t)
    }
}

/// State the army keeps that other passes read. **Written by the army
/// manager** (§5.1, §5.2); defaults are the start of a game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArmyShared {
    /// The phase (§5.1). Default: Setup.
    pub phase: Phase,
    /// The attack group (the wave), in id order (§5.2). Default: empty.
    pub wave: Vec<UnitId>,
    /// The reserve (§5.2). Default: empty.
    pub reserve: Vec<UnitId>,
    /// The wave centre (§5.2), while a wave is out.
    pub wave_centre: Option<Pos>,
    /// When the wave first made contact (§5.2).
    pub contact_since: Option<Tick>,
    /// The current wave objective (§5.2).
    pub objective: Option<Pos>,
    /// The rally point (§5.2).
    pub rally: Option<Pos>,
    /// The counter lean per answer type, in share points (§5.3, `mix.lean`),
    /// read by factory production (§4.4). Default: empty (no lean).
    pub lean: BTreeMap<DefId, i32>,
}

/// State the economy keeps that other passes read. **Written by the
/// economy manager** (§4); defaults are "nothing planned".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EconomyShared {
    /// The type the planner is saving for (§4.2). Default: none.
    pub saving_for: Option<DefId>,
    /// Pending tier-two extractor upgrade sites (§4.5). Default: empty.
    pub pending_upgrades: Vec<Pos>,
    /// Vision cells marked failed for building, with the tick the mark
    /// lapses (§4.2). Default: empty.
    pub failed_cells: BTreeMap<VisCell, Tick>,
    /// An extractor site on walkable ground home cannot reach (§5.9
    /// "expansion wanted"), checked at most every 10 s. Default: none.
    pub unreachable_expansion: Option<Pos>,
    /// Capacity short (§4.1), continuously for `eco.capacity_s`. Default:
    /// false.
    pub capacity_short: bool,
}

/// The world model for one seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct World {
    /// Our seat.
    pub me: PlayerId,
    /// Our side name.
    pub side: String,
    /// Living non-allied players other than us, in id order.
    pub enemies: Vec<PlayerId>,
    /// The role table (Appendix B), resolved once.
    pub roles: RoleTable,
    /// Own-state sampling (§3.1 step 1).
    pub own: OwnState,
    /// Our economy (§1.2, §4.1 fixed predicates).
    pub economy: EconomySummary,
    /// The contact memory (§1.3).
    pub contacts: ContactMemory,
    /// Facts derived from the memory (§1.3 step 4).
    pub enemy: EnemyPicture,
    /// The influence grid (§3.3).
    pub grid: InfluenceGrid,
    /// Map intel and deposits (§3.4, §4.5), computed once.
    pub map: MapIntel,
    /// Reachability layers and flags (§3.4).
    pub reach: Reachability,
    /// Extractor-site claims (§4.5).
    pub claims: MetalClaims,
    /// Unit ownership and loans (§3.1, §3.2).
    pub ownership: Ownership,
    /// Intruders, sectors and the radar warning (§5.4): army-filled.
    pub threat: ThreatPicture,
    /// The coordination flags (§3.1).
    pub coord: Coordination,
    /// Army state other passes read.
    pub army: ArmyShared,
    /// Economy state other passes read.
    pub eco: EconomyShared,
}

impl World {
    /// Home, or the ground home when the ground layer was re-homed (§3.4).
    pub fn ground_home(&self) -> Option<Pos> {
        self.reach.ground_home.or(self.own.home)
    }

    /// The enemy base: the focus enemy's, else the nearest known (§1.3).
    pub fn enemy_base(&self) -> Option<Pos> {
        self.enemy.enemy_base(self.own.home)
    }
}

impl World {
    /// An empty world for seat `me`; the first tick fills it.
    pub fn new(me: PlayerId) -> World {
        World {
            me,
            side: String::new(),
            enemies: Vec::new(),
            roles: RoleTable::default(),
            own: OwnState::default(),
            economy: EconomySummary::default(),
            contacts: ContactMemory::default(),
            enemy: EnemyPicture::default(),
            grid: InfluenceGrid::default(),
            map: MapIntel::default(),
            reach: Reachability::default(),
            claims: MetalClaims::default(),
            ownership: Ownership::default(),
            threat: ThreatPicture::default(),
            coord: Coordination::default(),
            army: ArmyShared::default(),
            eco: EconomyShared::default(),
        }
    }
}
