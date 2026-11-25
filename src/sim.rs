//! A small headless TA-like simulation, for behaviour tests only.
//!
//! It is a test harness, not a game, and says nothing about real strength.
//! It implements [`crate::view::GameView`], [`crate::view::FogLeaks`] (per
//! seat, through [`SeatView`]) and accepts [`crate::view::Orders`].
//!
//! Scope: a tile grid map with metal, water and impassable cells; two or more
//! players with line-of-sight and radar fog; metal and energy with
//! extractors, solars, makers and storage; builders with a worker rate and a
//! nano-lathe reach simplified to adjacency; factories; 8-way A* movement
//! (buildings do not block movement); hitscan combat with range, damage,
//! reload and energy per shot; wrecks; reclaim; capture; and transports that
//! load and unload after a fixed delay.
//!
//! Determinism: integer maths only, a seeded xorshift stream owned by the
//! sim, and every loop in unit, feature or player id order.

mod map;
mod rng;
mod roster;
mod script;
mod seat;
mod units;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

pub use map::{Map, MapBuilder, TileRect};
pub use roster::Side;
pub use script::Script;
pub use seat::{SeatOrders, SeatView};

use crate::view::{
    footprint_origin, Amount, Command, DefId, FeatureId, FeatureKind, Order, PlayerId, Pos, TeamId,
    Tick, Tile, UnitDef, UnitId, VisCell, TICKS_PER_SECOND, TILE_SIZE, VIS_CELL_SIZE,
};
use rng::XorShift;

/// Resource sub-units per milli-unit. Stocks are kept in milli × 30, so a
/// rate of R milli per second adds exactly R sub-units per tick.
const SUB: i64 = TICKS_PER_SECOND as i64;
/// Position sub-units per world unit.
const POS_SCALE: i64 = 256;
/// Builders reach a footprint within this distance (nano-lathe adjacency).
const BUILD_REACH: i64 = 64;
/// Ticks a transport takes to load or set down one unit.
const TRANSPORT_DELAY: Tick = 30;
/// Wrecks keep this percentage of the unit's metal cost.
const WRECK_METAL_PCT: i64 = 60;

/// Errors from setting up or poking the simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimError {
    BadMap(&'static str),
    NoPlayers,
    NotEnoughStarts { players: usize, starts: usize },
    UnknownDef(String),
    BadPlayer(PlayerId),
    OffMap(Pos),
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SimError::BadMap(why) => write!(f, "bad map: {why}"),
            SimError::NoPlayers => write!(f, "no players"),
            SimError::NotEnoughStarts { players, starts } => {
                write!(f, "{players} players but {starts} start positions")
            }
            SimError::UnknownDef(n) => write!(f, "unknown unit definition {n}"),
            SimError::BadPlayer(p) => write!(f, "no player {}", p.0),
            SimError::OffMap(p) => write!(f, "position {},{} is off the map", p.x, p.z),
        }
    }
}

impl std::error::Error for SimError {}

/// Why an order was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    NoSuchUnit,
    NotOwner,
    IsFrame,
    /// The unit cannot carry out this command (wrong kind, not in its menu).
    CannotDo,
    /// A build site was illegal or out of reach when the builder got there.
    BadSite,
}

/// A dropped order, for tests to inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub tick: Tick,
    pub player: PlayerId,
    pub order: Order,
    pub reason: Reject,
}

/// Something that happened, for tests to inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Completed {
        tick: Tick,
        unit: UnitId,
        owner: PlayerId,
        def: DefId,
    },
    Died {
        tick: Tick,
        unit: UnitId,
        owner: PlayerId,
        def: DefId,
        killer: Option<UnitId>,
    },
}

/// One seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerSetup {
    pub side: Side,
    pub team: Option<TeamId>,
    /// Income multiplier in permille (SPEC §6.2 `cheat.income_factor`).
    pub income_factor_permille: i64,
    /// Ignore fog for this seat (SPEC §1.1 `cheat.full_vision`).
    pub full_vision: bool,
}

impl PlayerSetup {
    pub fn new(side: Side) -> Self {
        Self {
            side,
            team: None,
            income_factor_permille: 1000,
            full_vision: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimConfig {
    pub seed: u64,
    pub players: Vec<PlayerSetup>,
    /// Ticks between submitting an order and applying it (SPEC §7 uses 7).
    pub order_delay: Tick,
    /// A player whose commander dies loses all its units.
    pub commander_death_ends: bool,
    /// Deal start positions by a Fisher-Yates shuffle from the back.
    pub random_starts: bool,
    pub start_metal: Amount,
    pub start_energy: Amount,
}

impl SimConfig {
    /// ARM against CORE with no teams.
    pub fn two_player(seed: u64) -> Self {
        Self {
            seed,
            players: vec![PlayerSetup::new(Side::Arm), PlayerSetup::new(Side::Core)],
            order_delay: 0,
            commander_death_ends: true,
            random_starts: false,
            start_metal: Amount::units(1000),
            start_energy: Amount::units(1000),
        }
    }
}

/// One resource pool. Stock and accumulators are in sub-units.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Pool {
    stock: i64,
    storage: i64,
    income_acc: i64,
    demand_acc: i64,
    stall_acc: bool,
    income: i64,
    demand: i64,
    stalled: bool,
}

impl Pool {
    fn add(&mut self, sub: i64) {
        self.income_acc += sub;
        self.stock = (self.stock + sub).min(self.storage * SUB).max(self.stock);
    }

    fn publish(&mut self) {
        self.income = self.income_acc / SUB;
        self.demand = self.demand_acc / SUB;
        self.stalled = self.stall_acc;
        self.income_acc = 0;
        self.demand_acc = 0;
        self.stall_acc = false;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Player {
    side: Side,
    team: Option<TeamId>,
    alive: bool,
    income_factor: i64,
    full_vision: bool,
    metal: Pool,
    energy: Pool,
    visible: Vec<bool>,
    explored: Vec<bool>,
    radar: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Frame {
    done: i64,
    total: i64,
    paid_m: i64,
    paid_e: i64,
    hp_given: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Unit {
    id: UnitId,
    owner: PlayerId,
    def: DefId,
    /// Position in 1/256 world units.
    x: i64,
    z: i64,
    /// Health in milli-hitpoints.
    hp: i64,
    frame: Option<Frame>,
    cmds: VecDeque<Command>,
    /// Remaining waypoints, last first.
    path: Vec<Pos>,
    path_goal: Option<Pos>,
    fqueue: VecDeque<DefId>,
    /// The frame a builder or factory is working on.
    building: Option<UnitId>,
    reload: Vec<Tick>,
    on: bool,
    carrying: Vec<UnitId>,
    carried_by: Option<UnitId>,
    /// Transport delay or capture progress.
    work: i64,
    /// Extractor output, milli-metal per second.
    extract: i64,
    last_attacker: Option<UnitId>,
}

impl Unit {
    fn pos(&self) -> Pos {
        Pos::new(
            (self.x.div_euclid(POS_SCALE)) as i32,
            (self.z.div_euclid(POS_SCALE)) as i32,
        )
    }

    fn set_pos(&mut self, p: Pos) {
        self.x = i64::from(p.x) * POS_SCALE + POS_SCALE / 2;
        self.z = i64::from(p.z) * POS_SCALE + POS_SCALE / 2;
    }

    fn health(&self) -> i32 {
        i32::try_from((self.hp + 999) / 1000).unwrap_or(i32::MAX)
    }

    fn pop(&mut self) {
        self.cmds.pop_front();
        self.path.clear();
        self.path_goal = None;
        self.work = 0;
        self.building = None;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FeatureState {
    id: FeatureId,
    kind: FeatureKind,
    pos: Pos,
    footprint: (i32, i32),
    /// Contents in sub-units.
    metal: i64,
    energy: i64,
    reclaimable: bool,
}

/// The simulation.
#[derive(Clone, Debug)]
pub struct Sim {
    tick: Tick,
    map: Map,
    defs: Vec<UnitDef>,
    players: Vec<Player>,
    starts: Vec<Pos>,
    units: BTreeMap<UnitId, Unit>,
    features: BTreeMap<FeatureId, FeatureState>,
    next_unit: u32,
    next_feature: u32,
    rng: XorShift,
    pending: VecDeque<(Tick, PlayerId, Order)>,
    order_delay: Tick,
    commander_death_ends: bool,
    scripts: Vec<(PlayerId, Script)>,
    /// Orders that were dropped, oldest first.
    pub rejected: Vec<Rejection>,
    /// Completions and deaths, oldest first.
    pub events: Vec<Event>,
}

impl Sim {
    /// A game on `map`: each player gets a commander on its start position.
    pub fn new(map: Map, cfg: SimConfig) -> Result<Sim, SimError> {
        if cfg.players.is_empty() {
            return Err(SimError::NoPlayers);
        }
        if cfg.players.len() > map.starts().len() || cfg.players.len() > 255 {
            return Err(SimError::NotEnoughStarts {
                players: cfg.players.len(),
                starts: map.starts().len(),
            });
        }
        let info = map.info();
        let cells = usize::try_from(vis_w(info.width_tiles) * vis_w(info.height_tiles))
            .map_err(|_| SimError::BadMap("size"))?;
        let mut rng = XorShift::new(cfg.seed);
        let mut order: Vec<usize> = (0..map.starts().len()).collect();
        if cfg.random_starts {
            for i in (1..order.len()).rev() {
                let j = usize::try_from(rng.below(i as u64 + 1)).unwrap_or(0);
                order.swap(i, j);
            }
        }
        let starts: Vec<Pos> = order.iter().map(|&i| map.starts()[i]).collect();
        let pool = |stock: Amount| Pool {
            stock: stock.0 * SUB,
            ..Pool::default()
        };
        let players = cfg
            .players
            .iter()
            .map(|s| Player {
                side: s.side,
                team: s.team,
                alive: true,
                income_factor: s.income_factor_permille,
                full_vision: s.full_vision,
                metal: pool(cfg.start_metal),
                energy: pool(cfg.start_energy),
                visible: vec![false; cells],
                explored: vec![false; cells],
                radar: vec![false; cells],
            })
            .collect();
        let mut sim = Sim {
            tick: 0,
            map,
            defs: roster::roster(),
            players,
            starts,
            units: BTreeMap::new(),
            features: BTreeMap::new(),
            next_unit: 1,
            next_feature: 1,
            rng,
            pending: VecDeque::new(),
            order_delay: cfg.order_delay,
            commander_death_ends: cfg.commander_death_ends,
            scripts: Vec::new(),
            rejected: Vec::new(),
            events: Vec::new(),
        };
        for (i, s) in cfg.players.iter().enumerate() {
            let p = PlayerId(u8::try_from(i).unwrap_or(0));
            let name = roster::commander_name(s.side);
            let def = sim
                .def_id(name)
                .ok_or_else(|| SimError::UnknownDef(name.to_string()))?;
            let at = sim.starts[i];
            sim.spawn(p, def, at, true)?;
        }
        sim.recompute_storage();
        // Starting stock is clamped to the starting storage.
        for pl in &mut sim.players {
            pl.metal.stock = pl.metal.stock.min(pl.metal.storage * SUB);
            pl.energy.stock = pl.energy.stock.min(pl.energy.storage * SUB);
        }
        sim.update_los();
        Ok(sim)
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn map(&self) -> &Map {
        &self.map
    }

    pub fn defs(&self) -> &[UnitDef] {
        &self.defs
    }

    pub fn def_id(&self, name: &str) -> Option<DefId> {
        self.defs
            .iter()
            .position(|d| d.name.eq_ignore_ascii_case(name))
            .and_then(|i| u16::try_from(i).ok())
            .map(DefId)
    }

    fn def(&self, id: DefId) -> &UnitDef {
        // DefIds inside the sim always index the roster; fall back to the
        // first entry rather than panic.
        self.defs
            .get(usize::from(id.0))
            .unwrap_or_else(|| &self.defs[0])
    }

    /// Start position of each player, in player order.
    pub fn player_start(&self, p: PlayerId) -> Option<Pos> {
        self.starts.get(usize::from(p.0)).copied()
    }

    pub fn player_alive(&self, p: PlayerId) -> bool {
        self.players
            .get(usize::from(p.0))
            .is_some_and(|pl| pl.alive)
    }

    /// One value from the sim's stream, to seed an AI seat (SPEC §7).
    pub fn draw_seat_seed(&mut self) -> u32 {
        (self.rng.next_u64() >> 33) as u32
    }

    /// A read-only view for one seat.
    pub fn view(&self, p: PlayerId) -> SeatView<'_> {
        SeatView::new(self, p)
    }

    /// An order sink for one seat.
    pub fn orders_for(&mut self, p: PlayerId) -> SeatOrders<'_> {
        SeatOrders::new(self, p)
    }

    /// Lets a scripted opponent play a seat.
    pub fn set_script(&mut self, p: PlayerId, s: Script) {
        self.scripts.retain(|(q, _)| *q != p);
        self.scripts.push((p, s));
    }

    /// Submits orders for a seat; they apply after the order delay.
    pub fn submit(&mut self, p: PlayerId, orders: impl IntoIterator<Item = Order>) {
        let due = self.tick + self.order_delay;
        for o in orders {
            self.pending.push_back((due, p, o));
        }
    }

    /// Puts a unit on the map, finished or as a fresh frame.
    pub fn spawn(
        &mut self,
        owner: PlayerId,
        def: DefId,
        at: Pos,
        complete: bool,
    ) -> Result<UnitId, SimError> {
        if usize::from(owner.0) >= self.players.len() {
            return Err(SimError::BadPlayer(owner));
        }
        if usize::from(def.0) >= self.defs.len() {
            return Err(SimError::UnknownDef(format!("#{}", def.0)));
        }
        if !self.map.info().contains(at) {
            return Err(SimError::OffMap(at));
        }
        let d = self.def(def);
        let total = d.build_time * SUB;
        let max_hp = i64::from(d.health) * 1000;
        let (weapons, mobile) = (d.weapons.len(), d.mobile);
        let id = UnitId(self.next_unit);
        self.next_unit += 1;
        let mut u = Unit {
            id,
            owner,
            def,
            x: 0,
            z: 0,
            hp: if complete { max_hp } else { 1000.min(max_hp) },
            frame: (!complete).then_some(Frame {
                done: 0,
                total: total.max(1),
                paid_m: 0,
                paid_e: 0,
                hp_given: 0,
            }),
            cmds: VecDeque::new(),
            path: Vec::new(),
            path_goal: None,
            fqueue: VecDeque::new(),
            building: None,
            reload: vec![0; weapons],
            on: true,
            carrying: Vec::new(),
            carried_by: None,
            work: 0,
            extract: 0,
            last_attacker: None,
        };
        if mobile {
            u.set_pos(at);
        } else {
            u.set_pos(self.footprint_center(def, at));
        }
        if complete {
            u.extract = self.extraction(&u);
        }
        self.units.insert(id, u);
        Ok(id)
    }

    /// Adds a feature (rock, tree or wreck) holding whole units of metal and
    /// energy.
    pub fn add_feature(
        &mut self,
        kind: FeatureKind,
        at: Pos,
        metal: i64,
        energy: i64,
    ) -> FeatureId {
        let footprint = match kind {
            FeatureKind::Wreck(d) => self.def(d).footprint,
            _ => (1, 1),
        };
        self.push_feature(FeatureState {
            id: FeatureId(0),
            kind,
            pos: at,
            footprint,
            metal: metal * 1000 * SUB,
            energy: energy * 1000 * SUB,
            reclaimable: true,
        })
    }

    fn push_feature(&mut self, mut f: FeatureState) -> FeatureId {
        let id = FeatureId(self.next_feature);
        self.next_feature += 1;
        f.id = id;
        self.features.insert(id, f);
        id
    }

    /// The centre of the snapped footprint of `def` placed at `at`.
    fn footprint_center(&self, def: DefId, at: Pos) -> Pos {
        let fp = self.def(def).footprint;
        let o = footprint_origin(fp, at).corner();
        Pos::new(o.x + fp.0 * TILE_SIZE / 2, o.z + fp.1 * TILE_SIZE / 2)
    }

    fn extraction(&self, u: &Unit) -> i64 {
        let d = self.def(u.def);
        if d.extracts_metal <= 0 {
            return 0;
        }
        let o = footprint_origin(d.footprint, u.pos());
        let mut sum = 0i64;
        for dz in 0..d.footprint.1 {
            for dx in 0..d.footprint.0 {
                sum += i64::from(self.map.metal(Tile::new(o.x + dx, o.z + dz)));
            }
        }
        sum * i64::from(d.extracts_metal) / 1000
    }

    /// Placement legality: terrain, buildings of any owner, and features.
    fn can_place(&self, def: DefId, at: Pos) -> bool {
        let d = self.def(def);
        let o = footprint_origin(d.footprint, at);
        let fits = (0..d.footprint.1).all(|dz| {
            (0..d.footprint.0).all(|dx| {
                self.map
                    .standable(Tile::new(o.x + dx, o.z + dz), d.move_class)
            })
        });
        if !fits {
            return false;
        }
        let rect = (o.x, o.z, o.x + d.footprint.0, o.z + d.footprint.1);
        let overlaps = |fp: (i32, i32), p: Pos| {
            let oo = footprint_origin(fp, p);
            oo.x < rect.2 && rect.0 < oo.x + fp.0 && oo.z < rect.3 && rect.1 < oo.z + fp.1
        };
        let blocked_by_unit = self.units.values().any(|u| {
            let ud = self.def(u.def);
            !ud.mobile && overlaps(ud.footprint, u.pos())
        });
        let blocked_by_feature = self.features.values().any(|f| overlaps(f.footprint, f.pos));
        !blocked_by_unit && !blocked_by_feature
    }

    fn allied(&self, a: PlayerId, b: PlayerId) -> bool {
        if a == b {
            return true;
        }
        let team = |p: PlayerId| self.players.get(usize::from(p.0)).and_then(|pl| pl.team);
        team(a).is_some() && team(a) == team(b)
    }

    fn cell_index(&self, c: VisCell) -> Option<usize> {
        let info = self.map.info();
        let (w, h) = (vis_w(info.width_tiles), vis_w(info.height_tiles));
        if c.x < 0 || c.z < 0 || c.x >= w || c.z >= h {
            None
        } else {
            usize::try_from(c.z * w + c.x).ok()
        }
    }

    /// Whether player `p` sees cell `c` now.
    fn sees_cell(&self, p: PlayerId, c: VisCell) -> bool {
        let Some(pl) = self.players.get(usize::from(p.0)) else {
            return false;
        };
        pl.full_vision || self.cell_index(c).is_some_and(|i| pl.visible[i])
    }

    /// Whether player `p` sees unit `u` now (allies' units always).
    fn sees_unit(&self, p: PlayerId, u: &Unit) -> bool {
        self.allied(p, u.owner) || (u.carried_by.is_none() && self.sees_cell(p, u.pos().vis_cell()))
    }

    /// Advances the simulation by one tick (1/30 s).
    pub fn step(&mut self) {
        self.run_scripts();
        self.apply_due_orders();
        self.economy_tick();
        let ids: Vec<UnitId> = self.units.keys().copied().collect();
        for &id in &ids {
            self.update_unit(id);
        }
        for &id in &ids {
            self.fire(id);
        }
        self.resolve_deaths();
        self.update_los();
        self.tick += 1;
        if self.tick.is_multiple_of(TICKS_PER_SECOND) {
            for pl in &mut self.players {
                pl.metal.publish();
                pl.energy.publish();
            }
        }
    }

    /// Steps `n` ticks.
    pub fn run(&mut self, n: u32) {
        for _ in 0..n {
            self.step();
        }
    }

    fn run_scripts(&mut self) {
        let mut scripts = std::mem::take(&mut self.scripts);
        for (p, s) in &mut scripts {
            if !self.player_alive(*p) {
                continue;
            }
            let mut rec = crate::view::OrderRecorder::new();
            s.think(&self.view(*p), &mut rec);
            self.submit(*p, rec.take());
        }
        self.scripts = scripts;
    }

    fn recompute_storage(&mut self) {
        let mut st: Vec<(i64, i64)> = vec![(0, 0); self.players.len()];
        for u in self.units.values() {
            if u.frame.is_none() {
                let d = self.def(u.def);
                if let Some(s) = st.get_mut(usize::from(u.owner.0)) {
                    s.0 += d.metal_storage.0;
                    s.1 += d.energy_storage.0;
                }
            }
        }
        for (pl, (m, e)) in self.players.iter_mut().zip(st) {
            pl.metal.storage = m;
            pl.energy.storage = e;
            pl.metal.stock = pl.metal.stock.min(m * SUB);
            pl.energy.stock = pl.energy.stock.min(e * SUB);
        }
    }

    fn economy_tick(&mut self) {
        self.recompute_storage();
        let ids: Vec<UnitId> = self.units.keys().copied().collect();
        for id in ids {
            let Some(u) = self.units.get(&id) else {
                continue;
            };
            if u.frame.is_some() || u.carried_by.is_some() {
                continue;
            }
            let d = self.def(u.def);
            let (owner, on, extract) = (usize::from(u.owner.0), u.on, u.extract);
            let (mm, em, eu, switchable) =
                (d.metal_make.0, d.energy_make.0, d.energy_use.0, d.on_off);
            let Some(pl) = self.players.get_mut(owner) else {
                continue;
            };
            let f = pl.income_factor;
            pl.energy.add(em * f / 1000);
            pl.metal.add(extract * f / 1000);
            if switchable {
                if on {
                    pl.energy.demand_acc += eu;
                    if pl.energy.stock >= eu {
                        pl.energy.stock -= eu;
                        pl.metal.add(mm * f / 1000);
                    } else {
                        pl.energy.stall_acc = true;
                    }
                }
            } else {
                pl.metal.add(mm * f / 1000);
            }
        }
    }

    fn apply_due_orders(&mut self) {
        while let Some(&(due, p, o)) = self.pending.front() {
            if due > self.tick {
                break;
            }
            self.pending.pop_front();
            if let Err(reason) = self.apply_order(p, o) {
                self.rejected.push(Rejection {
                    tick: self.tick,
                    player: p,
                    order: o,
                    reason,
                });
            }
        }
    }

    fn apply_order(&mut self, p: PlayerId, o: Order) -> Result<(), Reject> {
        let u = self.units.get(&o.unit).ok_or(Reject::NoSuchUnit)?;
        if u.owner != p {
            return Err(Reject::NotOwner);
        }
        if u.frame.is_some() {
            return Err(Reject::IsFrame);
        }
        let d = self.def(u.def);
        let ok = match o.command {
            Command::Build { def, .. } => d.builder && d.mobile && d.can_build(def),
            Command::FactoryQueue { def, .. } => d.is_factory() && d.can_build(def),
            Command::Move(_) | Command::Patrol(_) | Command::Guard(_) => d.mobile,
            Command::AttackMove(_) => d.mobile,
            Command::Attack(_) => d.armed(),
            Command::Repair(_) | Command::ReclaimFeature(_) | Command::ReclaimUnit(_) => {
                d.builder && d.mobile
            }
            Command::Capture(_) => d.builder && d.mobile,
            Command::DGun(_) => d.commander && d.weapons.len() > 2,
            Command::Load(_) | Command::Unload(_) => d.transport.is_some(),
            Command::Stop => true,
            Command::SetOn(_) => d.on_off,
        };
        if !ok {
            return Err(Reject::CannotDo);
        }
        let Some(u) = self.units.get_mut(&o.unit) else {
            return Err(Reject::NoSuchUnit);
        };
        let mut cancelled = None;
        match o.command {
            Command::Stop => {
                u.cmds.clear();
                u.pop();
            }
            Command::SetOn(on) => u.on = on,
            Command::FactoryQueue { def, count } => {
                if count >= 0 {
                    for _ in 0..count {
                        u.fqueue.push_back(def);
                    }
                } else {
                    let mut left = count.unsigned_abs();
                    let mut i = u.fqueue.len();
                    while left > 0 && i > 0 {
                        i -= 1;
                        if u.fqueue.get(i) == Some(&def) {
                            u.fqueue.remove(i);
                            left -= 1;
                            if i == 0 {
                                cancelled = u.building.take();
                                break;
                            }
                        }
                    }
                }
            }
            cmd => {
                if o.queue == crate::view::Queue::Immediate {
                    u.cmds.clear();
                    u.pop();
                }
                u.cmds.push_back(cmd);
            }
        }
        if let Some(f) = cancelled {
            self.units.remove(&f);
        }
        Ok(())
    }

    fn update_los(&mut self) {
        let info = self.map.info();
        let (w, h) = (vis_w(info.width_tiles), vis_w(info.height_tiles));
        let mut stamps: Vec<(PlayerId, Pos, i32, i32)> = Vec::new();
        for u in self.units.values() {
            if u.carried_by.is_some() {
                continue;
            }
            let d = self.def(u.def);
            let sight = if u.frame.is_some() { 0 } else { d.sight };
            stamps.push((
                u.owner,
                u.pos(),
                sight,
                if u.frame.is_some() { 0 } else { d.radar },
            ));
        }
        for pi in 0..self.players.len() {
            let me = PlayerId(u8::try_from(pi).unwrap_or(0));
            let mut vis = vec![false; self.players[pi].visible.len()];
            let mut rad = vec![false; vis.len()];
            for &(owner, at, sight, radar) in &stamps {
                if !self.allied(me, owner) {
                    continue;
                }
                stamp(&mut vis, w, h, at, sight);
                stamp(&mut rad, w, h, at, radar);
            }
            let pl = &mut self.players[pi];
            if pl.full_vision {
                vis.iter_mut().for_each(|v| *v = true);
            }
            for (e, v) in pl.explored.iter_mut().zip(&vis) {
                *e |= *v;
            }
            pl.visible = vis;
            pl.radar = rad;
        }
    }

    /// A 64-bit FNV-1a hash of the whole game state, for determinism tests.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.put(i64::from(self.tick));
        h.put(self.rng.state() as i64);
        for pl in &self.players {
            for v in [
                pl.metal.stock,
                pl.energy.stock,
                pl.metal.income,
                pl.energy.income,
            ] {
                h.put(v);
            }
            h.put(i64::from(pl.alive));
        }
        for u in self.units.values() {
            for v in [
                i64::from(u.id.0),
                i64::from(u.owner.0),
                i64::from(u.def.0),
                u.x,
                u.z,
                u.hp,
                u.frame.map_or(-1, |f| f.done),
                u.cmds.len() as i64,
                u.fqueue.len() as i64,
            ] {
                h.put(v);
            }
        }
        for f in self.features.values() {
            for v in [
                i64::from(f.id.0),
                i64::from(f.pos.x),
                i64::from(f.pos.z),
                f.metal,
            ] {
                h.put(v);
            }
        }
        h.0
    }
}

/// Vision cells along a side of `tiles` tiles.
fn vis_w(tiles: i32) -> i32 {
    let per = VIS_CELL_SIZE / TILE_SIZE;
    (tiles + per - 1) / per
}

/// Marks every cell whose centre is within `r` of `at`.
fn stamp(grid: &mut [bool], w: i32, h: i32, at: Pos, r: i32) {
    if r <= 0 {
        return;
    }
    let c = at.vis_cell();
    let cr = r / VIS_CELL_SIZE + 1;
    for z in (c.z - cr).max(0)..=(c.z + cr).min(h - 1) {
        for x in (c.x - cr).max(0)..=(c.x + cr).min(w - 1) {
            let cell = VisCell { x, z };
            if cell.center().within(at, i64::from(r)) {
                if let Ok(i) = usize::try_from(z * w + x) {
                    if let Some(g) = grid.get_mut(i) {
                        *g = true;
                    }
                }
            }
        }
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn put(&mut self, v: i64) {
        for b in v.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}
