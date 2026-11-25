//! The contact memory: the AI's fog-honest model of the enemy (SPEC §1.3).
//!
//! Every other part of the AI reads enemy information only from here, or by
//! asking whether a remembered contact can be seen now
//! ([`ContactMemory::standing`]). Contacts are kept in a `BTreeMap`, so every
//! iteration is in unit id order (SPEC §7).

use std::collections::BTreeMap;

use crate::view::{Amount, DefId, EnemyUnit, GameView, PlayerId, Pos, Tick, UnitDef, UnitId};

/// One remembered enemy unit (SPEC §1.3 step 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contact {
    pub id: UnitId,
    pub def: DefId,
    pub owner: PlayerId,
    /// Last known position.
    pub pos: Pos,
    /// Health when last seen.
    pub health: i32,
    /// Tick last seen.
    pub last_seen: Tick,
    /// Tick first seen, kept across refreshes.
    pub first_seen: Tick,
    /// A building (immobile).
    pub building: bool,
    /// Has any weapon.
    pub armed: bool,
    /// An aircraft.
    pub air: bool,
    pub commander: bool,
    pub builder: bool,
    /// Still a frame when last seen.
    pub frame: bool,
    /// Metal cost of its type (game data).
    pub metal: Amount,
}

impl Contact {
    /// "Seen now": refreshed this tick (SPEC §1.3 step 3).
    pub fn seen_now(&self, tick: Tick) -> bool {
        self.last_seen == tick
    }

    /// A mobile, armed, non-air contact (the army's "armed ground").
    pub fn armed_ground_mobile(&self) -> bool {
        self.armed && !self.building && !self.air
    }

    /// Its role for the enemy mix (SPEC §5.3), first match.
    pub fn role(&self, def: Option<&UnitDef>) -> ContactRole {
        if self.commander || !self.armed {
            ContactRole::Other
        } else if self.builder {
            ContactRole::Builder
        } else if self.air {
            ContactRole::Air
        } else if self.building {
            ContactRole::StaticDefence
        } else if self.metal >= Amount::units(200) {
            ContactRole::Tank
        } else if def.is_some_and(|d| d.max_range() >= 300) {
            ContactRole::RocketArtillery
        } else {
            ContactRole::Raider
        }
    }
}

/// The role of a contact in the enemy mix (SPEC §5.3 table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContactRole {
    /// A commander, an unarmed building or an unarmed mobile unit.
    Other,
    Builder,
    Air,
    StaticDefence,
    Tank,
    RocketArtillery,
    Raider,
}

impl ContactRole {
    pub const ALL: [ContactRole; 7] = [
        ContactRole::Other,
        ContactRole::Builder,
        ContactRole::Air,
        ContactRole::StaticDefence,
        ContactRole::Tank,
        ContactRole::RocketArtillery,
        ContactRole::Raider,
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The answer to "is this contact still standing?" (SPEC §1.3 step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing<'a> {
    /// We are looking at its last known spot and it is alive and visible:
    /// the live unit may be read, and orders may target it.
    Live(EnemyUnit),
    /// We are not looking: assumed standing, but only the remembered fields
    /// may be used, and no order may target it.
    Remembered(&'a Contact),
}

/// What changed in one [`ContactMemory::update`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactChanges {
    /// Contacts forgotten this tick, in id order.
    pub forgotten: Vec<UnitId>,
    /// Contacts recorded for the first time this tick, in id order.
    pub new: Vec<UnitId>,
}

/// The remembered enemy units.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactMemory {
    contacts: BTreeMap<UnitId, Contact>,
    tick: Tick,
}

impl ContactMemory {
    /// Runs SPEC §1.3 steps 1 and 2 for this tick: forget, then record.
    ///
    /// `enemies` are the non-allied players (by id order); a visible unit of
    /// any other owner is never recorded.
    pub fn update(
        &mut self,
        view: &dyn GameView,
        enemies: &[PlayerId],
        tick: Tick,
    ) -> ContactChanges {
        self.tick = tick;
        let visible: Vec<EnemyUnit> = view
            .visible_enemies()
            .into_iter()
            .filter(|e| !e.immune && e.health > 0 && enemies.contains(&e.owner))
            .collect();
        let seen = |id: UnitId| visible.binary_search_by_key(&id, |e| e.id).is_ok();
        let mut changes = ContactChanges::default();

        // 1. Forget: looking at the last known position and not visible.
        //    With full vision `in_los` is always true, so a contact goes the
        //    moment it dies.
        let full = view.full_vision();
        self.contacts.retain(|&id, c| {
            let looking = full || view.in_los(c.pos);
            let keep = !looking || seen(id);
            if !keep {
                changes.forgotten.push(id);
            }
            keep
        });

        // 2. Record every visible living enemy.
        for e in &visible {
            let d = view.unit_def(e.def);
            let first_seen = match self.contacts.get(&e.id) {
                Some(c) => c.first_seen,
                None => {
                    changes.new.push(e.id);
                    tick
                }
            };
            self.contacts.insert(
                e.id,
                Contact {
                    id: e.id,
                    def: e.def,
                    owner: e.owner,
                    pos: e.pos,
                    health: e.health,
                    last_seen: tick,
                    first_seen,
                    building: d.is_some_and(|d| !d.mobile),
                    armed: d.is_some_and(UnitDef::armed),
                    air: d.is_some_and(|d| d.flies),
                    commander: d.is_some_and(|d| d.commander),
                    builder: d.is_some_and(|d| d.builder),
                    frame: e.is_frame,
                    metal: d.map(|d| d.cost_metal).unwrap_or_default(),
                },
            );
        }
        changes
    }

    /// One contact by id.
    pub fn get(&self, id: UnitId) -> Option<&Contact> {
        self.contacts.get(&id)
    }

    /// Every contact, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Contact> + '_ {
        self.contacts.values()
    }

    pub fn len(&self) -> usize {
        self.contacts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.contacts.is_empty()
    }

    /// Whether the contact was refreshed this tick.
    pub fn seen_now(&self, id: UnitId) -> bool {
        self.contacts
            .get(&id)
            .is_some_and(|c| c.seen_now(self.tick))
    }

    /// The "standing" rule (SPEC §1.3 step 3). `None` means not standing (or
    /// not remembered). Call it after this tick's update.
    pub fn standing<'a>(&'a self, view: &dyn GameView, id: UnitId) -> Option<Standing<'a>> {
        let c = self.contacts.get(&id)?;
        if view.full_vision() || view.in_los(c.pos) || c.seen_now(self.tick) {
            view.enemy(id).filter(|e| e.health > 0).map(Standing::Live)
        } else {
            Some(Standing::Remembered(c))
        }
    }

    /// The live unit, only when it is seen now: the precondition for any
    /// order aimed at it (SPEC §1.3 step 3).
    pub fn target(&self, view: &dyn GameView, id: UnitId) -> Option<EnemyUnit> {
        if self.seen_now(id) {
            view.enemy(id)
        } else {
            None
        }
    }

    /// Contacts within `r` of `p`, in id order.
    pub fn near(&self, p: Pos, r: i64) -> impl Iterator<Item = &Contact> + '_ {
        self.contacts.values().filter(move |c| c.pos.within(p, r))
    }
}

/// Facts derived from the memory each tick (SPEC §1.3 step 4), except the
/// intruder, sector and radar facts, which the army keeps in
/// [`super::ThreatPicture`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnemyPicture {
    /// Per enemy player: the centroid of its remembered buildings, else its
    /// commander's last position.
    pub bases: BTreeMap<PlayerId, Pos>,
    /// The focus enemy (`army.one_enemy`).
    pub focus: Option<PlayerId>,
    /// An enemy aircraft is remembered, or was within the last 300 s.
    pub air_threat: bool,
    /// Last tick an enemy aircraft was remembered.
    pub last_air: Option<Tick>,
    /// The peak number of armed enemy aircraft remembered at one time.
    pub peak_armed_air: u32,
    /// Known enemy metal in mobile armed ground units.
    pub ground_metal: Amount,
    /// Known enemy metal in mobile armed aircraft.
    pub air_metal: Amount,
    /// Our army's metal: combat units, excluding the borrowed scout (§5.1).
    pub our_army_metal: Amount,
    /// The decaying role histogram (`mix.memory`, SPEC §5.3), indexed by
    /// [`ContactRole::index`].
    pub roles: [u32; 7],
    roles_halved: Option<Tick>,
    /// Factories that lost a frame within `build.harass_radius` in 30 s.
    pub harassed_factories: Vec<UnitId>,
    /// Harassed factories with an armed contact within 400 (SPEC §1.3).
    pub besieged_factories: Vec<UnitId>,
}

impl EnemyPicture {
    /// The enemy base: the focus enemy's, else the nearest known to `home`.
    pub fn enemy_base(&self, home: Option<Pos>) -> Option<Pos> {
        if let Some(f) = self.focus {
            return self.bases.get(&f).copied();
        }
        let h = home?;
        self.bases
            .values()
            .copied()
            .min_by_key(|b| (b.dist2(h), b.x, b.z))
    }

    /// The remembered commander of `player`, if any (lowest id first).
    pub fn commander_of(memory: &ContactMemory, player: PlayerId) -> Option<&Contact> {
        memory.iter().find(|c| c.commander && c.owner == player)
    }

    /// The share of `role` in the histogram, in permille (0 when empty).
    pub fn role_share_permille(&self, role: ContactRole) -> u32 {
        let total: u32 = self.roles.iter().sum();
        (self.roles.get(role.index()).copied().unwrap_or(0) * 1000)
            .checked_div(total)
            .unwrap_or(0)
    }
}

/// Inputs of [`EnemyPicture::update`] that come from knobs and own state.
pub(crate) struct PictureInputs<'a> {
    pub tick: Tick,
    pub home: Option<Pos>,
    pub alive_enemies: &'a [PlayerId],
    pub one_enemy: bool,
    pub switch_margin: i64,
    pub mix_memory: bool,
    pub mix_halflife: Tick,
    pub our_army_metal: Amount,
    /// Own factories (id, position), in id order.
    pub factories: &'a [(UnitId, Pos)],
    /// Our frames that died recently (position, tick).
    pub frame_deaths: &'a [(Pos, Tick)],
    pub harass_radius: i64,
}

const AIR_THREAT_TICKS: Tick = 300 * 30;
const HARASS_TICKS: Tick = 30 * 30;
const BESIEGED_RADIUS: i64 = 400;

impl EnemyPicture {
    pub(crate) fn update(&mut self, memory: &ContactMemory, view: &dyn GameView, k: PictureInputs) {
        // Enemy bases.
        let mut sums: BTreeMap<PlayerId, (i64, i64, i64)> = BTreeMap::new();
        for c in memory.iter().filter(|c| c.building) {
            let e = sums.entry(c.owner).or_insert((0, 0, 0));
            e.0 += i64::from(c.pos.x);
            e.1 += i64::from(c.pos.z);
            e.2 += 1;
        }
        self.bases.clear();
        for (&p, &(x, z, n)) in &sums {
            if n > 0 {
                let cx = i32::try_from(x / n).unwrap_or(0);
                let cz = i32::try_from(z / n).unwrap_or(0);
                self.bases.insert(p, Pos::new(cx, cz));
            }
        }
        for c in memory.iter().filter(|c| c.commander) {
            self.bases.entry(c.owner).or_insert(c.pos);
        }

        // Focus enemy.
        self.focus = if k.one_enemy {
            self.choose_focus(k.home, k.alive_enemies, k.switch_margin)
        } else {
            None
        };

        // Air threat and the peak armed air count.
        let air: Vec<&Contact> = memory.iter().filter(|c| c.air).collect();
        if !air.is_empty() {
            self.last_air = Some(k.tick);
        }
        self.air_threat = self
            .last_air
            .is_some_and(|t| k.tick.saturating_sub(t) <= AIR_THREAT_TICKS);
        let armed_air = u32::try_from(air.iter().filter(|c| c.armed).count()).unwrap_or(u32::MAX);
        self.peak_armed_air = self.peak_armed_air.max(armed_air);

        // Army values.
        self.ground_metal = Amount::ZERO;
        self.air_metal = Amount::ZERO;
        for c in memory.iter().filter(|c| c.armed && !c.building) {
            if c.air {
                self.air_metal += c.metal;
            } else {
                self.ground_metal += c.metal;
            }
        }
        self.our_army_metal = k.our_army_metal;

        // Role histogram: raise to the current counts, halve on schedule.
        let mut now = [0u32; 7];
        for c in memory.iter() {
            let r = c.role(view.unit_def(c.def)).index();
            if let Some(n) = now.get_mut(r) {
                *n += 1;
            }
        }
        if k.mix_memory {
            let last = *self.roles_halved.get_or_insert(k.tick);
            if k.mix_halflife > 0 && k.tick.saturating_sub(last) >= k.mix_halflife {
                for r in &mut self.roles {
                    *r >>= 1;
                }
                self.roles_halved = Some(k.tick);
            }
            for (r, n) in self.roles.iter_mut().zip(now) {
                *r = (*r).max(n);
            }
        } else {
            self.roles = now;
        }

        // Harassed and besieged factories.
        self.harassed_factories.clear();
        self.besieged_factories.clear();
        for &(id, pos) in k.factories {
            let harassed = k.frame_deaths.iter().any(|&(p, t)| {
                k.tick.saturating_sub(t) <= HARASS_TICKS && p.within(pos, k.harass_radius)
            });
            if !harassed {
                continue;
            }
            self.harassed_factories.push(id);
            if memory.near(pos, BESIEGED_RADIUS).any(|c| c.armed) {
                self.besieged_factories.push(id);
            }
        }
    }

    fn choose_focus(&self, home: Option<Pos>, alive: &[PlayerId], margin: i64) -> Option<PlayerId> {
        let home = home?;
        let nearest = self
            .bases
            .iter()
            .filter(|(p, _)| alive.contains(p))
            .map(|(&p, b)| (b.dist(home), p))
            .min();
        let current = self
            .focus
            .filter(|p| alive.contains(p))
            .and_then(|p| self.bases.get(&p).map(|b| (b.dist(home), p)));
        match (current, nearest) {
            (Some((cd, cp)), Some((nd, np))) => {
                if np != cp && nd + margin < cd {
                    Some(np)
                } else {
                    Some(cp)
                }
            }
            (Some((_, cp)), None) => Some(cp),
            (None, n) => n.map(|(_, p)| p),
        }
    }
}

#[cfg(test)]
mod tests;
