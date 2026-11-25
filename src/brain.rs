//! The AI's core loop: the per-tick pipeline of SPEC §3.1, the shared world
//! model the managers coordinate through, the manager seam, and the order
//! dispatcher (micro limiter §2.3, reservation gate §2.4).
//!
//! # Layout
//!
//! - [`Ai`] is the public entry point. The host calls [`Ai::tick`] once per
//!   simulated tick (SPEC §7) with a [`GameView`], the [`FogLeaks`] and an
//!   [`Orders`] sink.
//! - [`World`] is the world model: own state, contact memory, map analysis,
//!   reachability, influence grid, claims, ownership and the coordination
//!   flags. The core fills what it can before any manager runs.
//! - [`Manager`] is the seam. Each slot ([`Source`]) holds one manager:
//!   [`economy::EconomyManager`] (SPEC §4), [`army::ArmyManager`] (§5),
//!   [`army::navy::NavyManager`], [`army::scouts::ScoutManager`],
//!   [`army::transport::TransportManager`] and [`army::air::AirManager`].
//!   Managers read and write the world through [`Ctx`] and stage orders in
//!   [`Staging`]; they never talk to each other directly.
//!
//! # The pipeline (SPEC §3.1), every tick
//!
//! | # | Step | Who | When |
//! |---|---|---|---|
//! | 1 | [`Step::Sample`]: own-state sampling, classification, ownership | core | every tick |
//! | 2 | [`Step::Contacts`]: contact memory and derived facts, claims | core | every tick |
//! | 3 | [`Step::Grid`]: influence grid | core | `cadence.threat_grid_ticks` |
//! | 4 | [`Step::Reach`]: reachability re-home | core | first chance, then 20 s |
//! | 5 | [`Step::Phase`]: phase machine, threat picture, radar sample | army | every tick |
//! | 6 | [`Step::MetalMakers`] | economy | every tick |
//! | 6b | [`Step::Sweep`]: builder safety, commander repair, defence watch | army | 15 ticks |
//! | 7 | [`Step::Plan`]: build planning | economy | `cadence.plan_ticks` |
//! | 8 | [`Step::Scouts`] | scouts | `cadence.scout_ticks` |
//! | 9 | [`Step::Transports`] | transport | `cadence.tactics_ticks` |
//! | 10 | [`Step::Army`], then [`Step::Navy`] | army, navy | `cadence.tactics_ticks` |
//! | 11 | [`Step::Air`] | air | `cadence.tactics_ticks` |
//! | 12-13 | [`Step::Dispatch`]: ownership, conflicts, de-dup, limiter, gate | core | every tick |
//!
//! The 15-tick sweep is not placed by the spec; it runs before the planner
//! so that the planner sees this tick's shelter and flee flags (§4.2 rule
//! 1). See [`cadence`] for the first tick of each pass.
//!
//! # Determinism (SPEC §7)
//!
//! Every collection that can influence an order is a `Vec` in a defined
//! order or a `BTreeMap`/`BTreeSet`; no hash map is iterated. The only
//! randomness is the seat's [`AiRng`], reached through [`Ctx::rng`]. No
//! float is used: positions are integer world units, amounts are milli-units,
//! knobs are [`crate::knobs::Fixed`].

pub mod army;
pub mod cadence;
pub mod contacts;
pub mod dispatch;
pub mod economy;
pub mod grid;
pub mod limiter;
pub mod mapintel;
pub mod own;
pub mod ownership;
pub mod reach;
pub mod roles;
pub mod staging;
pub mod threat;
pub mod world;

#[cfg(test)]
mod tests;

use std::fmt;

pub use cadence::{Cadence, CadenceError, Due};
pub use contacts::{Contact, ContactMemory, ContactRole, EnemyPicture, Standing};
pub use dispatch::{DispatchStats, ReservationTable};
pub use grid::InfluenceGrid;
pub use mapintel::{Claim, Deposit, MapCharacter, MapIntel, MetalClaims, ShipyardSite};
pub use own::{EconomySummary, Loss, OwnState, Sampled, UnitClass, WreckSite};
pub use ownership::{LoanReason, Owner, Ownership, OwnershipError, Source};
pub use reach::{Layer, Reachability};
pub use roles::{Role, RoleTable};
pub use staging::{InFlight, Priority, Staged, Staging, DEFAULT_ORDER_DELAY};
pub use threat::{sector_of, SectorMemory, ThreatPicture};
pub use world::{ArmyShared, Coordination, EconomyShared, GuardRequest, Phase, World};

use contacts::PictureInputs;
use dispatch::Dispatcher;

use crate::knobs::Knobs;
use crate::personality::{Difficulty, Resolved};
use crate::rng::AiRng;
use crate::view::{
    EnemyUnit, FogLeaks, GameView, Order, Orders, PlayerId, Pos, Tick, UnitDef, UnitId,
    TICKS_PER_SECOND,
};

/// One step of the per-tick pipeline (SPEC §3.1), in pipeline order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    Sample,
    Contacts,
    Grid,
    Reach,
    Phase,
    MetalMakers,
    Sweep,
    Plan,
    Scouts,
    Transports,
    Army,
    Navy,
    Air,
    Dispatch,
}

impl Step {
    /// Number of steps.
    pub const COUNT: usize = 14;

    pub fn index(self) -> usize {
        self as usize
    }
}

/// A manager: one slot of the pipeline. The core calls [`Manager::update`]
/// at each of the slot's steps; [`Ctx::step`] says which, and [`Ctx::due`]
/// which passes are due. Orders go to `out`, tagged with the slot.
pub trait Manager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging);
}

/// What a manager may touch during its step.
pub struct Ctx<'a> {
    /// Fog-honest reads (SPEC §1.2).
    pub view: &'a dyn GameView,
    /// The reference AI's named fog leaks (SPEC §1.4); every use is explicit.
    pub leaks: &'a dyn FogLeaks,
    /// The seat's tuning.
    pub knobs: &'a Knobs,
    /// The seat's random stream: only for exact ties in site choice (§7).
    pub rng: &'a mut AiRng,
    /// The current tick.
    pub tick: Tick,
    /// The step being run.
    pub step: Step,
    /// The passes due this tick.
    pub due: Due,
    /// The shared world model.
    pub world: &'a mut World,
    /// Orders sent but not yet visible (SPEC §7 fixed delay).
    pub in_flight: &'a InFlight,
    /// The difficulty tier.
    pub difficulty: Difficulty,
}

impl Ctx<'_> {
    /// A unit definition.
    pub fn def(&self, id: crate::view::DefId) -> Option<&UnitDef> {
        self.view.unit_def(id)
    }

    /// The newest in-flight group for `unit`.
    pub fn in_flight(&self, unit: UnitId) -> Option<&[Order]> {
        self.in_flight.orders(unit, self.tick)
    }

    /// A unit has visible orders or orders in flight.
    pub fn busy(&self, unit: UnitId) -> bool {
        self.in_flight(unit).is_some()
            || self
                .world
                .own
                .get(unit)
                .is_some_and(|s| !s.unit.commands.is_empty())
    }

    /// The "standing" rule for a contact (SPEC §1.3 step 3).
    pub fn standing(&self, id: UnitId) -> Option<Standing<'_>> {
        self.world.contacts.standing(self.view, id)
    }

    /// The live enemy, only if seen now: required to aim an order at it.
    pub fn target(&self, id: UnitId) -> Option<EnemyUnit> {
        self.world.contacts.target(self.view, id)
    }

    /// The home point.
    pub fn home(&self) -> Option<Pos> {
        self.world.own.home
    }
}

/// Why the AI could not be built or ticked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainError {
    /// A cadence knob is not a positive tick count.
    Cadence(CadenceError),
    /// The view belongs to another seat.
    WrongSeat { expected: PlayerId, got: PlayerId },
    /// The tick went backwards. After a load the host must build a new `Ai`
    /// (SPEC §7: AI state is not saved).
    TickWentBack { last: Tick, now: Tick },
}

impl fmt::Display for BrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrainError::Cadence(e) => write!(f, "cadence: {e}"),
            BrainError::WrongSeat { expected, got } => {
                write!(f, "view is for player {}, not {}", got.0, expected.0)
            }
            BrainError::TickWentBack { last, now } => {
                write!(f, "tick went back from {last} to {now}")
            }
        }
    }
}

impl std::error::Error for BrainError {}

impl From<CadenceError> for BrainError {
    fn from(e: CadenceError) -> Self {
        BrainError::Cadence(e)
    }
}

/// Counters for tests and tuning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiStats {
    /// Ticks processed.
    pub ticks: u64,
    /// Times each step ran, by [`Step::index`].
    pub steps: [u64; Step::COUNT],
    /// The dispatcher's counters.
    pub dispatch: DispatchStats,
}

impl AiStats {
    /// Times a step ran.
    pub fn runs(&self, step: Step) -> u64 {
        self.steps.get(step.index()).copied().unwrap_or(0)
    }
}

/// One AI seat.
pub struct Ai {
    player: PlayerId,
    knobs: Knobs,
    difficulty: Difficulty,
    rng: AiRng,
    cadence: Cadence,
    world: World,
    managers: Vec<Box<dyn Manager>>,
    dispatcher: Dispatcher,
    staging: Staging,
    last_tick: Option<Tick>,
    started: bool,
    stats: AiStats,
    pass_log: Option<Vec<(Tick, Step)>>,
}

impl fmt::Debug for Ai {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ai")
            .field("player", &self.player)
            .field("difficulty", &self.difficulty)
            .field("last_tick", &self.last_tick)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl Ai {
    /// A seat for `player` with its resolved tuning. `seed` is the one value
    /// the seat drew from the simulation's random stream (SPEC §7).
    pub fn new(player: PlayerId, resolved: &Resolved, seed: u64) -> Result<Ai, BrainError> {
        let cadence = Cadence::from_knobs(&resolved.knobs)?;
        let managers: Vec<Box<dyn Manager>> = Source::ALL
            .iter()
            .map(|&s| -> Box<dyn Manager> {
                match s {
                    Source::Economy => Box::new(economy::EconomyManager::default()),
                    Source::Army => Box::new(army::ArmyManager::default()),
                    Source::Navy => Box::new(army::navy::NavyManager::default()),
                    Source::Scouts => Box::new(army::scouts::ScoutManager::default()),
                    Source::Transport => Box::new(army::transport::TransportManager::default()),
                    Source::Air => Box::new(army::air::AirManager::default()),
                }
            })
            .collect();
        Ok(Ai {
            player,
            knobs: resolved.knobs.clone(),
            difficulty: resolved.difficulty,
            rng: AiRng::from_sim_draw(seed),
            cadence,
            world: World::new(player),
            managers,
            dispatcher: Dispatcher::new(resolved.micro_limit, InFlight::default()),
            staging: Staging::default(),
            last_tick: None,
            started: false,
            stats: AiStats::default(),
            pass_log: None,
        })
    }

    /// Replaces the manager in a slot (alternative managers, tests).
    pub fn set_manager(&mut self, slot: Source, manager: Box<dyn Manager>) {
        if let Some(m) = self.managers.get_mut(slot.index()) {
            *m = manager;
        }
    }

    /// The host's command buffer depth for AI orders (default 7, SPEC §7).
    pub fn set_order_delay(&mut self, delay: Tick) {
        self.dispatcher.in_flight.set_delay(delay);
    }

    /// The mission reservation table (SPEC §2.4); `None` in skirmish.
    pub fn set_reservations(&mut self, table: Option<ReservationTable>) {
        self.dispatcher.reservations = table;
    }

    /// Starts (or stops) recording which step ran on which tick.
    pub fn record_passes(&mut self, on: bool) {
        self.pass_log = on.then(Vec::new);
    }

    /// The recorded passes, oldest first.
    pub fn pass_log(&self) -> &[(Tick, Step)] {
        self.pass_log.as_deref().unwrap_or(&[])
    }

    pub fn player(&self) -> PlayerId {
        self.player
    }

    pub fn knobs(&self) -> &Knobs {
        &self.knobs
    }

    pub fn cadence(&self) -> &Cadence {
        &self.cadence
    }

    /// The world model, for inspection.
    pub fn world(&self) -> &World {
        &self.world
    }

    pub fn stats(&self) -> &AiStats {
        &self.stats
    }

    /// Runs one simulated tick. Call it exactly once per simulation tick; a
    /// second call for the same tick does nothing (SPEC §7: orders are fed
    /// once per simulated tick, never per attempt to tick).
    pub fn tick(
        &mut self,
        view: &impl GameView,
        leaks: &impl FogLeaks,
        out: &mut impl Orders,
    ) -> Result<(), BrainError> {
        let tick = view.tick();
        if view.me() != self.player {
            return Err(BrainError::WrongSeat {
                expected: self.player,
                got: view.me(),
            });
        }
        match self.last_tick {
            Some(t) if tick == t => return Ok(()),
            Some(t) if tick < t => return Err(BrainError::TickWentBack { last: t, now: tick }),
            _ => {}
        }
        self.last_tick = Some(tick);
        if self.knobs.misc.idle {
            return Ok(());
        }
        self.run(view, leaks, out, tick);
        Ok(())
    }

    fn log(&mut self, tick: Tick, step: Step) {
        if let Some(s) = self.stats.steps.get_mut(step.index()) {
            *s += 1;
        }
        if let Some(l) = self.pass_log.as_mut() {
            l.push((tick, step));
        }
    }

    fn run(&mut self, view: &dyn GameView, leaks: &dyn FogLeaks, out: &mut dyn Orders, tick: Tick) {
        self.stats.ticks += 1;
        let players = view.players();
        let me = players.iter().find(|p| p.id == self.player);
        if !self.started {
            self.started = true;
            self.world.side = me.map(|p| p.side.clone()).unwrap_or_default();
            self.world.roles = RoleTable::resolve(view, &self.world.side);
            self.world.map = MapIntel::analyse(view);
        }
        self.world.enemies = players
            .iter()
            .filter(|p| p.id != self.player && p.alive && me.is_none_or(|m| !m.allied_with(p)))
            .map(|p| p.id)
            .collect();
        // Every enemy, dead or alive, for recording (a dead player's units
        // may still stand until the game removes them).
        let hostile: Vec<PlayerId> = players
            .iter()
            .filter(|p| p.id != self.player && me.is_none_or(|m| !m.allied_with(p)))
            .map(|p| p.id)
            .collect();

        // 1. Own-state sampling.
        let w = &mut self.world;
        w.own.sample(
            view,
            &w.roles,
            &mut w.ownership,
            &self.dispatcher.in_flight,
            tick,
        );
        w.economy = EconomySummary {
            raw: view.economy(),
            committed_metal: w.own.committed_metal,
        };
        w.coord.commander_tasked = false;
        if let Some(b) = w.coord.borrowed_scout {
            if w.ownership.loan(b).is_none() {
                w.coord.borrowed_scout = None;
            }
        }
        self.log(tick, Step::Sample);

        // 2. Contact memory and derived facts; claims.
        let w = &mut self.world;
        w.contacts.update(view, &hostile, tick);
        let our_army_metal = w
            .own
            .combat
            .iter()
            .filter_map(|&u| w.own.get(u))
            .filter_map(|s| view.unit_def(s.unit.def))
            .fold(crate::view::Amount::ZERO, |a, d| a + d.cost_metal);
        let factories: Vec<(UnitId, Pos)> = w
            .own
            .factories
            .iter()
            .filter_map(|&f| w.own.get(f).map(|s| (f, s.unit.pos)))
            .collect();
        let k = &self.knobs;
        w.enemy.update(
            &w.contacts,
            view,
            PictureInputs {
                tick,
                home: w.own.home,
                alive_enemies: &w.enemies,
                one_enemy: k.army.one_enemy,
                switch_margin: k.army.enemy_switch_margin.floor(),
                mix_memory: k.mix.memory,
                mix_halflife: secs_knob(k.mix.memory_halflife_s),
                our_army_metal,
                factories: &factories,
                frame_deaths: &w.own.frame_deaths,
                harass_radius: k.build.harass_radius.floor(),
            },
        );
        w.claims.rebuild(
            view,
            w.own.units.iter().map(|s| &s.unit),
            self.dispatcher.in_flight.all(tick),
            tick,
        );
        self.log(tick, Step::Contacts);

        // 3-4. Grid and reachability, when due.
        let reach_due = self.world.reach.due(tick, &self.world.own);
        let due = self.cadence.due(tick, reach_due);
        if due.grid {
            self.world.grid.rebuild(view, &self.world.contacts);
            self.log(tick, Step::Grid);
        }
        if due.reach {
            let w = &mut self.world;
            w.reach.update(view, &w.roles, &w.own, &w.map, tick);
            self.log(tick, Step::Reach);
        }

        // 5-11. Managers.
        self.call(Step::Phase, Source::Army, view, leaks, tick, due);
        self.call(Step::MetalMakers, Source::Economy, view, leaks, tick, due);
        if due.sweep {
            self.call(Step::Sweep, Source::Army, view, leaks, tick, due);
        }
        if due.plan {
            self.call(Step::Plan, Source::Economy, view, leaks, tick, due);
        }
        if let Some(c) = self.world.own.commander {
            self.world.coord.commander_tasked = self.staging.has_order_from(c, Source::Economy);
        }
        if due.scouts {
            self.call(Step::Scouts, Source::Scouts, view, leaks, tick, due);
        }
        if due.tactics {
            self.call(Step::Transports, Source::Transport, view, leaks, tick, due);
            self.call(Step::Army, Source::Army, view, leaks, tick, due);
            self.call(Step::Navy, Source::Navy, view, leaks, tick, due);
            self.call(Step::Air, Source::Air, view, leaks, tick, due);
        }

        // 12-13. Dispatch.
        let staged = self.staging.take();
        self.dispatcher
            .dispatch(staged, &self.world.own, &self.world.ownership, tick, out);
        self.stats.dispatch = self.dispatcher.stats;
        self.log(tick, Step::Dispatch);
    }

    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        step: Step,
        slot: Source,
        view: &dyn GameView,
        leaks: &dyn FogLeaks,
        tick: Tick,
        due: Due,
    ) {
        let Ai {
            managers,
            world,
            knobs,
            rng,
            dispatcher,
            staging,
            difficulty,
            ..
        } = self;
        let Some(m) = managers.get_mut(slot.index()) else {
            return;
        };
        staging.begin(slot);
        let mut ctx = Ctx {
            view,
            leaks,
            knobs,
            rng,
            tick,
            step,
            due,
            world,
            in_flight: &dispatcher.in_flight,
            difficulty: *difficulty,
        };
        m.update(&mut ctx, staging);
        self.log(tick, step);
    }
}

/// A seconds knob as ticks (negative values count as 0).
fn secs_knob(s: i32) -> Tick {
    u32::try_from(s)
        .unwrap_or(0)
        .saturating_mul(TICKS_PER_SECOND)
}
