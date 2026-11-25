//! Per-unit micro (SPEC §5.12): acquisition, kiting, blocked shots and
//! stalled attacks. The navy may reuse [`StallClock`] and [`stand_off`].

use std::collections::{BTreeMap, BTreeSet};

use super::geom::{inside, lerp, line_clear, sidestep};
use super::{dist, secs, ArmyManager, MAP_MARGIN, SAME_MOVE, STALL_FORWARD};
use crate::brain::{Ctx, Standing};
use crate::view::{Command, Pos, Tick, UnitDef, UnitId};

/// The no-damage clock of one (unit, target) pair (§5.12 "Stalled
/// attacks"). It runs only while the target is in reach, and restarts
/// whenever the target's health changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallClock {
    /// The target's health when the clock last restarted.
    pub health: i32,
    /// Ticks in reach without damage.
    pub elapsed: Tick,
    /// The last tick the clock was looked at.
    pub last: Tick,
    /// Repositions made.
    pub tries: i32,
    /// The point a reposition is heading for.
    pub moving_to: Option<Pos>,
}

impl StallClock {
    pub fn new(health: i32, now: Tick) -> Self {
        StallClock {
            health,
            elapsed: 0,
            last: now,
            tries: 0,
            moving_to: None,
        }
    }

    /// Advances the clock; true once `limit` ticks passed without damage.
    pub fn tick(&mut self, now: Tick, health: i32, in_reach: bool, limit: Tick) -> bool {
        if health != self.health {
            self.health = health;
            self.elapsed = 0;
        } else if in_reach {
            self.elapsed = self.elapsed.saturating_add(now.saturating_sub(self.last));
        }
        self.last = now;
        self.elapsed >= limit
    }
}

/// The stand-off distance of an outranging shooter (navy §5.12):
/// max(their range + margin, our range − margin).
pub fn stand_off(their_range: i64, our_range: i64, margin: i64) -> i64 {
    (their_range + margin).max(our_range - margin)
}

/// What the micro remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MicroState {
    /// Stall clocks by (unit, target).
    pub(crate) stalls: BTreeMap<(UnitId, UnitId), StallClock>,
    /// Targets a unit gave up on, until a tick.
    pub(crate) ignored: BTreeMap<(UnitId, UnitId), Tick>,
}

/// The acquisition radius (§5.12): max(`army.engage`, longest range −
/// `army.kite_margin`) with `army.acquire_at_range`, else `army.engage`.
pub fn acquire_radius(ctx: &Ctx<'_>, d: &UnitDef) -> i64 {
    let k = &ctx.knobs.army;
    let engage = dist(k.engage);
    if k.acquire_at_range {
        engage.max(i64::from(d.max_range()) - dist(k.kite_margin))
    } else {
        engage
    }
}

/// Whether some weapon of `d` is meant for the target's layer (anti-air
/// weapons for aircraft, the others for the rest).
fn can_engage(d: &UnitDef, air: bool) -> bool {
    d.weapons.iter().any(|w| w.anti_air == air)
}

/// A direct-fire shooter: a weapon that is neither lobbed nor dropped.
fn direct_fire(d: &UnitDef) -> bool {
    d.weapons.iter().any(|w| !w.lobbed && !w.dropped)
}

impl MicroState {
    pub(crate) fn prune(&mut self, ctx: &Ctx<'_>, alive: &BTreeSet<UnitId>) {
        let tick = ctx.tick;
        let anti_air: BTreeSet<UnitId> = ctx.world.own.anti_air.iter().copied().collect();
        let keep = |u: &UnitId, t: &UnitId| {
            (alive.contains(u) || anti_air.contains(u)) && ctx.world.contacts.get(*t).is_some()
        };
        self.stalls.retain(|(u, t), _| keep(u, t));
        self.ignored
            .retain(|(u, t), until| tick < *until && keep(u, t));
    }

    /// The nearest seen, standing, non-frame contact within `radius` that
    /// the unit has not given up on and has a weapon for (§5.12).
    pub(crate) fn acquire(
        &self,
        ctx: &Ctx<'_>,
        u: UnitId,
        here: Pos,
        d: &UnitDef,
        radius: i64,
    ) -> Option<UnitId> {
        ctx.world
            .contacts
            .near(here, radius)
            .filter(|c| !c.frame && ctx.world.contacts.seen_now(c.id))
            .filter(|c| can_engage(d, c.air))
            .filter(|c| !self.ignored.contains_key(&(u, c.id)))
            .filter(|c| matches!(ctx.standing(c.id), Some(Standing::Live(_))))
            .min_by_key(|c| (c.pos.dist2(here), c.id))
            .map(|c| c.id)
    }
}

impl ArmyManager {
    /// The order for a unit with a target in reach: kite, clear a blocked
    /// shot, reposition after a stall, or attack (§5.12).
    pub(crate) fn engage(&mut self, ctx: &Ctx<'_>, u: UnitId, t: UnitId) -> Option<Command> {
        let k = &ctx.knobs.army;
        let s = ctx.world.own.get(u)?;
        let d = ctx.def(s.unit.def)?;
        let c = *ctx.world.contacts.get(t)?;
        let here = s.unit.pos;
        let range = i64::from(d.max_range());
        let gap = here.dist(c.pos);
        let td = ctx.def(c.def);

        // A reposition under way is finished first.
        if let Some(clock) = self.micro.stalls.get_mut(&(u, t)) {
            if let Some(p) = clock.moving_to {
                let moving = matches!(super::current_order(ctx, u), Some(Command::Move(_)));
                if !here.within(p, SAME_MOVE) && moving {
                    return Some(Command::Move(p));
                }
                clock.moving_to = None;
            }
        }

        // Kiting: step back from a shorter-ranged mobile target.
        if k.kite && !c.building {
            let margin = dist(k.kite_margin);
            let theirs = i64::from(td.map_or(0, UnitDef::max_range));
            if theirs + margin < range && gap < theirs + margin {
                let back = c.pos.toward(here, theirs + margin);
                let back = if here == c.pos { here } else { back };
                return Some(Command::Move(inside(ctx.view, back, MAP_MARGIN)));
            }
        }

        let air = d.flies || c.air;
        let blocked = k.blocked_shots
            && direct_fire(d)
            && !air
            && gap <= range
            && !line_clear(ctx.view, here, c.pos);
        if blocked && !k.stalls {
            return Some(Command::Move(clear_point(ctx, here, c.pos)));
        }

        // The stall clock.
        if k.stalls {
            let limit = secs(k.stall_s);
            let clock = self
                .micro
                .stalls
                .entry((u, t))
                .or_insert_with(|| StallClock::new(c.health, ctx.tick));
            let stalled = clock.tick(ctx.tick, c.health, gap <= range, limit);
            if blocked {
                let p = clear_point(ctx, here, c.pos);
                clock.moving_to = Some(p);
                return Some(Command::Move(p));
            }
            if stalled {
                if clock.tries < k.stall_tries {
                    clock.tries += 1;
                    clock.elapsed = 0;
                    let side = if clock.tries % 2 == 1 { 1 } else { -1 };
                    let p = sidestep(here, c.pos, side * dist(k.stall_sidestep), STALL_FORWARD);
                    let p = inside(ctx.view, p, MAP_MARGIN);
                    clock.moving_to = Some(p);
                    return Some(Command::Move(p));
                }
                // Give up on it for a while, and stop.
                self.micro.stalls.remove(&(u, t));
                self.micro
                    .ignored
                    .insert((u, t), ctx.tick.saturating_add(secs(k.stall_forget_s)));
                return Some(Command::Move(here));
            }
        }
        Some(Command::Attack(t))
    }
}

/// The first of 8 points along the approach (1/9 to 8/9 of the way) with a
/// clear line to the target, else the target itself (§5.12).
fn clear_point(ctx: &Ctx<'_>, from: Pos, to: Pos) -> Pos {
    (1..=8)
        .map(|i| lerp(from, to, i, 9))
        .find(|&p| line_clear(ctx.view, p, to))
        .unwrap_or(to)
}
