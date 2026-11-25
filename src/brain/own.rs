//! Own-state sampling (SPEC §3.1 step 1) and own-unit classification (§3.2).
//!
//! Runs first every tick. Everything here is own data, free to read
//! (SPEC §1.1). Lists are in unit id order (SPEC §7).

use std::collections::{BTreeMap, BTreeSet};

use super::ownership::{default_owner, LoanReason, Owner, Ownership, Source};
use super::roles::{Role, RoleTable};
use super::staging::InFlight;
use crate::view::{Amount, Command, DefId, Economy, GameView, OwnUnit, Pos, Tick, UnitDef, UnitId};

/// The class of one of our units, first match (SPEC §3.2), plus the classes
/// the rules leave over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitClass {
    /// 1. An immobile builder.
    Factory,
    /// 2. A mobile non-builder transport.
    Transport,
    /// 3. A mobile naval combat unit (navy).
    Hull,
    /// 4. A mobile scout type, or an unarmed non-builder aircraft.
    Scout,
    /// 5. A mobile builder (idle or assisting), the commander included.
    Builder,
    /// 6. Mobile anti-air of the side's anti-air kbot type.
    AntiAir,
    /// 7. A mobile, armed ground unit.
    Combat,
    /// An armed aircraft: not a combat unit; the air pass owns it.
    Aircraft,
    /// Any other building.
    Structure,
    /// Anything else (an unarmed ground unit that is no scout or builder).
    Other,
}

/// Classifies a definition (SPEC §3.2), first match.
pub fn classify(def: DefId, d: &UnitDef, roles: &RoleTable) -> UnitClass {
    let naval = !d.flies && !d.move_class.hovers && d.move_class.min_depth > 0;
    if d.builder && !d.mobile {
        UnitClass::Factory
    } else if !d.mobile {
        UnitClass::Structure
    } else if !d.builder && d.transport.is_some() {
        UnitClass::Transport
    } else if naval && d.armed() {
        UnitClass::Hull
    } else if roles.is_scout_type(def) || (d.flies && !d.armed() && !d.builder) {
        UnitClass::Scout
    } else if d.builder {
        UnitClass::Builder
    } else if roles.is(def, Role::AntiAirKbot) {
        UnitClass::AntiAir
    } else if d.armed() && !d.flies {
        UnitClass::Combat
    } else if d.armed() {
        UnitClass::Aircraft
    } else {
        UnitClass::Other
    }
}

/// One of our units with its classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sampled {
    pub unit: OwnUnit,
    pub class: UnitClass,
    pub commander: bool,
    /// Immobile.
    pub building: bool,
    /// Its default owner (before loans).
    pub owner: Owner,
}

impl Sampled {
    pub fn id(&self) -> UnitId {
        self.unit.id
    }
    pub fn is_frame(&self) -> bool {
        self.unit.frame.is_some()
    }
}

/// A unit or building of ours that vanished (SPEC §3.1 step 1 losses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loss {
    pub id: UnitId,
    pub def: DefId,
    pub pos: Pos,
    pub tick: Tick,
    pub was_frame: bool,
}

/// A death of ours near home: a wreck worth reclaiming (§3.1, §4.12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WreckSite {
    pub pos: Pos,
    pub def: DefId,
    pub tick: Tick,
}

const BUILDING_LOSSES: usize = 8;
const BUILDING_LOSS_TICKS: Tick = 60 * 30;
const UNIT_LOSSES: usize = 16;
const UNIT_LOSS_TICKS: Tick = 30 * 30;
const FRAME_DEATH_TICKS: Tick = 30 * 30;
const WRECK_SITES: usize = 32;
const WRECK_RADIUS: i64 = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Prev {
    def: DefId,
    pos: Pos,
    frame: bool,
    building: bool,
}

/// Our own state this tick (SPEC §3.1 step 1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OwnState {
    /// Every own unit, classified, in id order.
    pub units: Vec<Sampled>,
    /// The commander, if we have one.
    pub commander: Option<UnitId>,
    /// The home point: the commander's first position, never revised. Before
    /// a commander is seen: the first factory, else any own unit.
    pub home: Option<Pos>,
    home_locked: bool,
    /// Finished units per type.
    pub completed: BTreeMap<DefId, u32>,
    /// Finished + frames + build orders not yet placed (in queues or in
    /// flight), per type.
    pub total: BTreeMap<DefId, u32>,
    /// Metal per second committed by builders and factories on frames.
    pub committed_metal: Amount,
    /// The part of `committed_metal` on frames a factory is building.
    pub factory_committed_metal: Amount,
    /// The last 8 building losses within 60 s, oldest first.
    pub building_losses: Vec<Loss>,
    /// The last 16 mobile unit losses within 30 s, oldest first.
    pub unit_losses: Vec<Loss>,
    /// Our frames that died in the last 30 s (position, tick): feeds the
    /// harassed-factory test (SPEC §1.3).
    pub frame_deaths: Vec<(Pos, Tick)>,
    /// Own wreck sites: deaths within 1200 of home, at most 32, oldest first.
    pub wreck_sites: Vec<WreckSite>,
    /// Building frames no builder or factory is attending.
    pub orphan_frames: Vec<UnitId>,
    /// Builders with no orders (or only guard orders) and nothing in flight.
    pub idle_builders: Vec<UnitId>,
    /// Finished factories.
    pub factories: Vec<UnitId>,
    /// Finished builders with the economy (the commander included).
    pub builders: Vec<UnitId>,
    /// Finished combat units with the army (not lent).
    pub combat: Vec<UnitId>,
    /// Finished mobile anti-air with the army (not lent).
    pub anti_air: Vec<UnitId>,
    /// Finished hulls with the navy (not lent).
    pub hulls: Vec<UnitId>,
    /// Finished units with the scouts, borrowed ones included.
    pub scouts: Vec<UnitId>,
    /// Finished armed aircraft.
    pub aircraft: Vec<UnitId>,
    /// Finished transports.
    pub transports: Vec<UnitId>,
    /// Every own frame.
    pub frames: Vec<UnitId>,
    /// Land combat units, less one if a combat unit is the borrowed scout.
    pub army_size: u32,
    /// **Written by the economy manager:** units it is removing on purpose
    /// (an extractor reclaimed for an upgrade, §4.5). Their disappearance is
    /// not a loss. Entries are dropped once the unit is gone.
    pub deliberate_removals: BTreeSet<UnitId>,
    prev: BTreeMap<UnitId, Prev>,
}

impl OwnState {
    /// One unit by id.
    pub fn get(&self, id: UnitId) -> Option<&Sampled> {
        self.units
            .binary_search_by_key(&id, Sampled::id)
            .ok()
            .and_then(|i| self.units.get(i))
    }

    /// Finished units of a type.
    pub fn completed_of(&self, def: DefId) -> u32 {
        self.completed.get(&def).copied().unwrap_or(0)
    }

    /// Finished + frames + unplaced build orders of a type.
    pub fn total_of(&self, def: DefId) -> u32 {
        self.total.get(&def).copied().unwrap_or(0)
    }

    /// Runs SPEC §3.1 step 1.
    pub(crate) fn sample(
        &mut self,
        view: &dyn GameView,
        roles: &RoleTable,
        ownership: &mut Ownership,
        in_flight: &InFlight,
        tick: Tick,
    ) {
        let mut raw = view.own_units();
        raw.sort_by_key(|u| u.id);
        self.units = raw
            .into_iter()
            .filter_map(|u| {
                let d = view.unit_def(u.def)?;
                let class = classify(u.def, d, roles);
                let owner = default_owner(class, d.commander, u.scripted || u.frame.is_some());
                Some(Sampled {
                    class,
                    commander: d.commander,
                    building: !d.mobile,
                    owner,
                    unit: u,
                })
            })
            .collect();
        let defaults: Vec<(UnitId, Owner)> = self.units.iter().map(|s| (s.id(), s.owner)).collect();
        ownership.sync(&defaults, tick);

        self.record_losses(tick);
        self.find_home();
        self.count(view, in_flight, tick);
        self.lists(ownership, in_flight, tick);
    }

    fn record_losses(&mut self, tick: Tick) {
        let now: BTreeSet<UnitId> = self.units.iter().map(Sampled::id).collect();
        let prev = std::mem::take(&mut self.prev);
        for (&id, p) in &prev {
            if now.contains(&id) {
                continue;
            }
            if self.deliberate_removals.remove(&id) {
                continue;
            }
            let loss = Loss {
                id,
                def: p.def,
                pos: p.pos,
                tick,
                was_frame: p.frame,
            };
            if p.building {
                self.building_losses.push(loss);
            } else {
                self.unit_losses.push(loss);
            }
            if p.frame {
                self.frame_deaths.push((p.pos, tick));
            }
            if self.home.is_some_and(|h| h.within(p.pos, WRECK_RADIUS)) {
                self.wreck_sites.push(WreckSite {
                    pos: p.pos,
                    def: p.def,
                    tick,
                });
            }
        }
        let fresh = |l: &Loss, ttl: Tick| tick.saturating_sub(l.tick) <= ttl;
        self.building_losses
            .retain(|l| fresh(l, BUILDING_LOSS_TICKS));
        trim_front(&mut self.building_losses, BUILDING_LOSSES);
        self.unit_losses.retain(|l| fresh(l, UNIT_LOSS_TICKS));
        trim_front(&mut self.unit_losses, UNIT_LOSSES);
        self.frame_deaths
            .retain(|&(_, t)| tick.saturating_sub(t) <= FRAME_DEATH_TICKS);
        trim_front(&mut self.wreck_sites, WRECK_SITES);
        self.deliberate_removals.retain(|id| now.contains(id));

        self.prev = self
            .units
            .iter()
            .map(|s| {
                (
                    s.id(),
                    Prev {
                        def: s.unit.def,
                        pos: s.unit.pos,
                        frame: s.is_frame(),
                        building: s.building,
                    },
                )
            })
            .collect();
    }

    fn find_home(&mut self) {
        self.commander = self
            .units
            .iter()
            .find(|s| s.commander && !s.is_frame())
            .map(Sampled::id);
        if self.home_locked {
            return;
        }
        if let Some(c) = self.commander.and_then(|c| self.get(c)) {
            self.home = Some(c.unit.pos);
            self.home_locked = true;
            return;
        }
        self.home = self
            .units
            .iter()
            .find(|s| s.class == UnitClass::Factory && !s.is_frame())
            .or_else(|| self.units.first())
            .map(|s| s.unit.pos)
            .or(self.home);
    }

    /// Metal per second (milli) that `rate` (work per tick on each frame)
    /// commits: remaining metal / remaining seconds of each frame, which is
    /// 30 × cost × rate / total work.
    fn committed(&self, view: &dyn GameView, rate: &BTreeMap<UnitId, i64>) -> i64 {
        let mut committed = 0i64;
        for (f, r) in rate {
            let Some(s) = self.get(*f) else { continue };
            let (Some(p), Some(d)) = (s.unit.frame, view.unit_def(s.unit.def)) else {
                continue;
            };
            if p.total > 0 && p.done < p.total {
                committed = committed.saturating_add(
                    d.cost_metal.milli().saturating_mul(30).saturating_mul(*r) / p.total,
                );
            }
        }
        committed
    }

    fn count(&mut self, view: &dyn GameView, in_flight: &InFlight, tick: Tick) {
        self.completed.clear();
        self.total.clear();
        // Work rate (per tick) on each frame, and who is attending it.
        let mut rate: BTreeMap<UnitId, i64> = BTreeMap::new();
        // Frames inside a factory (their work, assistants included, is the
        // factory spend).
        let in_factory: BTreeSet<UnitId> = self
            .units
            .iter()
            .filter_map(|s| s.unit.factory.as_ref().and_then(|f| f.building))
            .collect();
        let mut attended: BTreeSet<UnitId> = BTreeSet::new();
        for s in &self.units {
            let def = s.unit.def;
            if s.is_frame() {
                *self.total.entry(def).or_insert(0) += 1;
            } else {
                *self.completed.entry(def).or_insert(0) += 1;
                *self.total.entry(def).or_insert(0) += 1;
            }
            let worker = view.unit_def(def).map_or(0, |d| d.worker_rate);
            let mut target = s.unit.working_on;
            if target.is_none() {
                target = s.unit.factory.as_ref().and_then(|f| f.building);
            }
            if let Some(f) = target {
                *rate.entry(f).or_insert(0) += worker;
                attended.insert(f);
            }
            // Build orders not yet placed.
            let placed_head = target
                .and_then(|f| self.units.binary_search_by_key(&f, Sampled::id).ok())
                .and_then(|i| self.units.get(i))
                .map(|f| f.unit.def);
            for (i, c) in s.unit.commands.iter().enumerate() {
                match *c {
                    Command::Build { def: b, .. } if !(i == 0 && placed_head == Some(b)) => {
                        *self.total.entry(b).or_insert(0) += 1;
                    }
                    Command::Repair(f) if i == 0 => {
                        attended.insert(f);
                    }
                    _ => {}
                }
            }
        }
        for o in in_flight.all(tick) {
            match o.command {
                Command::Build { def, .. } => *self.total.entry(def).or_insert(0) += 1,
                Command::Repair(f) => {
                    attended.insert(f);
                }
                _ => {}
            }
        }
        // Committed metal: remaining metal / remaining seconds of each frame
        // worked on, which is 30 × cost × rate / total work.
        self.committed_metal = Amount(self.committed(view, &rate));
        let factory_rate: BTreeMap<UnitId, i64> = rate
            .iter()
            .filter(|(f, _)| in_factory.contains(f))
            .map(|(&f, &r)| (f, r))
            .collect();
        self.factory_committed_metal = Amount(self.committed(view, &factory_rate));
        self.orphan_frames = self
            .units
            .iter()
            .filter(|s| s.is_frame() && s.building && !attended.contains(&s.id()))
            .map(Sampled::id)
            .collect();
    }

    fn lists(&mut self, own: &Ownership, in_flight: &InFlight, tick: Tick) {
        let mut factories = Vec::new();
        let mut builders = Vec::new();
        let mut idle = Vec::new();
        let mut combat = Vec::new();
        let mut anti_air = Vec::new();
        let mut hulls = Vec::new();
        let mut scouts = Vec::new();
        let mut aircraft = Vec::new();
        let mut transports = Vec::new();
        let mut frames = Vec::new();
        let mut land_combat = 0u32;
        for s in &self.units {
            let id = s.id();
            if s.is_frame() {
                frames.push(id);
                continue;
            }
            if s.unit.scripted {
                // Counted, never ordered (SPEC §3.2).
                land_combat += u32::from(s.class == UnitClass::Combat);
                continue;
            }
            let lent = own.loan(id).is_some();
            match s.class {
                UnitClass::Factory => factories.push(id),
                UnitClass::Builder if !lent => {
                    builders.push(id);
                    let only_guard = s
                        .unit
                        .commands
                        .iter()
                        .all(|c| matches!(c, Command::Guard(_)));
                    if only_guard && in_flight.orders(id, tick).is_none() && !s.unit.scripted {
                        idle.push(id);
                    }
                }
                UnitClass::Combat => {
                    land_combat += 1;
                    if !lent {
                        combat.push(id);
                    }
                }
                UnitClass::AntiAir if !lent => anti_air.push(id),
                UnitClass::Hull if !lent => hulls.push(id),
                UnitClass::Aircraft if !lent => aircraft.push(id),
                UnitClass::Transport => transports.push(id),
                _ => {}
            }
            if own.is_with(id, Source::Scouts) {
                scouts.push(id);
            }
        }
        let borrowed_combat = own
            .lent_for(LoanReason::BorrowedScout)
            .any(|u| self.get(u).is_some_and(|s| s.class == UnitClass::Combat));
        self.army_size = land_combat.saturating_sub(u32::from(borrowed_combat));
        self.factories = factories;
        self.builders = builders;
        self.idle_builders = idle;
        self.combat = combat;
        self.anti_air = anti_air;
        self.hulls = hulls;
        self.scouts = scouts;
        self.aircraft = aircraft;
        self.transports = transports;
        self.frames = frames;
    }
}

fn trim_front<T>(v: &mut Vec<T>, max: usize) {
    if v.len() > max {
        let excess = v.len() - max;
        v.drain(..excess);
    }
}

/// Our economy this tick, with the fixed predicates of SPEC §4.1 that both
/// managers read. The economy manager adds what needs history (capacity
/// short, the value ratio) in [`super::EconomyShared`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EconomySummary {
    /// The raw reading (SPEC §1.2 own player row).
    pub raw: Economy,
    /// Metal per second committed to frames being worked on (§3.1 step 1).
    pub committed_metal: Amount,
}

impl EconomySummary {
    /// Metal stalled, or stock < 10% of storage.
    pub fn metal_short(&self) -> bool {
        self.raw.metal.stalled || self.raw.metal.fill_permille() < 100
    }
    /// Metal stock ≥ 80%.
    pub fn metal_full(&self) -> bool {
        self.raw.metal.fill_permille() >= 800
    }
    /// Metal stock ≥ 90%.
    pub fn metal_pegged(&self) -> bool {
        self.raw.metal.fill_permille() >= 900
    }
    /// Energy stalled, or stock < 25%.
    pub fn energy_short(&self) -> bool {
        self.raw.energy.stalled || self.raw.energy.fill_permille() < 250
    }
    /// Energy stalled, or stock < 50%.
    pub fn energy_wanted(&self) -> bool {
        self.raw.energy.stalled || self.raw.energy.fill_permille() < 500
    }
    /// Energy stock ≥ 80% and income > demand.
    pub fn energy_rich(&self) -> bool {
        self.raw.energy.fill_permille() >= 800 && self.raw.energy.income > self.raw.energy.demand
    }
}
