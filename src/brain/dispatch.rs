//! The dispatcher: turns the tick's [`super::Staging`] into sent orders (SPEC §3.1
//! steps 12-13).
//!
//! In order:
//!
//! 1. **Ours and orderable.** Orders to units that are not ours this tick are
//!    dropped, and so are orders to held units (scripted units and frames,
//!    SPEC §3.2).
//! 2. **Ownership.** Orders from a manager that may not order the unit
//!    ([`super::Ownership::may_order`]) are dropped.
//! 3. **One manager per unit per tick.** If several managers staged orders
//!    for one unit, the highest [`super::Priority`] wins, ties to the earliest
//!    staged (pipeline order).
//! 4. **De-duplication.** Consecutive identical orders collapse to one
//!    (factory queue orders excepted: two "+1" are two units). A metal maker
//!    gets at most one on/off command per second (SPEC §2.2), and none that
//!    matches its state. A group is dropped when it repeats a group still in
//!    flight ([`InFlight`], SPEC §7), or when it would replace the unit's
//!    queue with what the queue already holds.
//! 5. **The micro limiter** (SPEC §2.3) over the army, navy and air groups
//!    that hold micro orders. A refused unit loses all its orders from those
//!    passes this tick.
//! 6. **The reservation gate** (SPEC §2.4), when the host gave a table.
//! 7. **Send**, in staging order, and remember the groups as in flight.

use std::collections::BTreeMap;

use super::limiter::MicroLimiter;
use super::own::OwnState;
use super::ownership::{Owner, Ownership, Source};
use super::staging::{InFlight, Staged};
use crate::view::{Command, DefId, Order, Orders, OwnUnit, Queue, Tick, UnitId, TICKS_PER_SECOND};

/// Counters kept by the dispatcher, for tests and tuning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DispatchStats {
    /// Orders staged by managers.
    pub staged: u64,
    /// Orders sent to the game.
    pub sent: u64,
    /// Dropped: not our unit (dead, or never ours).
    pub not_ours: u64,
    /// Dropped: a scripted unit or an unfinished frame.
    pub held: u64,
    /// Dropped: the staging manager does not own the unit.
    pub not_owner: u64,
    /// Dropped: another manager won the unit this tick.
    pub conflict: u64,
    /// Dropped by de-duplication.
    pub duplicate: u64,
    /// Micro-limited orders admitted.
    pub limiter_admitted: u64,
    /// Micro-limited orders refused.
    pub limiter_refused: u64,
    /// Dropped by the reservation gate.
    pub gated: u64,
}

/// A mission's per-type limit table (SPEC §2.4). A type mapped to 0 is
/// forbidden; a type not in the table is unlimited. Skirmish has no table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReservationTable {
    pub limits: BTreeMap<DefId, u32>,
}

/// The dispatcher's state across ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatcher {
    pub(crate) limiter: MicroLimiter,
    pub(crate) in_flight: InFlight,
    pub(crate) reservations: Option<ReservationTable>,
    last_set_on: BTreeMap<UnitId, Tick>,
    pub(crate) stats: DispatchStats,
}

struct Group {
    unit: UnitId,
    source: Source,
    /// (staging index, order)
    orders: Vec<(usize, Order)>,
}

impl Dispatcher {
    pub fn new(micro_limit: Option<u32>, in_flight: InFlight) -> Self {
        Dispatcher {
            limiter: MicroLimiter::new(micro_limit),
            in_flight,
            reservations: None,
            last_set_on: BTreeMap::new(),
            stats: DispatchStats::default(),
        }
    }

    /// Runs the dispatch for one tick and sends the survivors to `out`.
    pub(crate) fn dispatch(
        &mut self,
        staged: Vec<Staged>,
        own: &OwnState,
        ownership: &Ownership,
        tick: Tick,
        out: &mut dyn Orders,
    ) {
        self.in_flight.prune(tick);
        self.limiter.retain_units(|u| own.get(u).is_some());
        self.last_set_on.retain(|&u, _| own.get(u).is_some());
        self.stats.staged += staged.len() as u64;

        // 1-3: filter, then pick one source per unit.
        let mut by_unit: BTreeMap<UnitId, Vec<(usize, Staged)>> = BTreeMap::new();
        for (i, s) in staged.into_iter().enumerate() {
            let unit = s.order.unit;
            match ownership.owner(unit) {
                None => self.stats.not_ours += 1,
                Some(Owner::Held) => self.stats.held += 1,
                Some(_) if !ownership.may_order(unit, s.source) => self.stats.not_owner += 1,
                Some(_) => by_unit.entry(unit).or_default().push((i, s)),
            }
        }
        let mut groups: Vec<Group> = Vec::new();
        for (unit, entries) in by_unit {
            let Some(winner) = pick_source(&entries) else {
                continue;
            };
            let before = entries.len();
            let orders: Vec<(usize, Order)> = entries
                .into_iter()
                .filter(|(_, s)| s.source == winner)
                .map(|(i, s)| (i, s.order))
                .collect();
            self.stats.conflict += (before - orders.len()) as u64;
            groups.push(Group {
                unit,
                source: winner,
                orders,
            });
        }

        // 4: de-duplication.
        for g in &mut groups {
            let before = g.orders.len();
            g.orders.dedup_by(|b, a| {
                a.1 == b.1 && !matches!(a.1.command, Command::FactoryQueue { .. })
            });
            let unit = own.get(g.unit).map(|s| &s.unit);
            let last_set_on = self.last_set_on.get(&g.unit).copied();
            let mut set_on_kept = false;
            g.orders.retain(|(_, o)| match o.command {
                Command::SetOn(on) => {
                    let throttled =
                        last_set_on.is_some_and(|t| tick.saturating_sub(t) < TICKS_PER_SECOND);
                    let keep = !throttled && !set_on_kept && unit.is_none_or(|u| u.on != on);
                    set_on_kept |= keep;
                    keep
                }
                _ => true,
            });
            let plain: Vec<Order> = g.orders.iter().map(|&(_, o)| o).collect();
            let repeat = self
                .in_flight
                .groups(g.unit, tick)
                .any(|sent| sent.orders == plain);
            if repeat || unit.is_some_and(|u| is_noop(&plain, u)) {
                g.orders.clear();
            }
            self.stats.duplicate += (before - g.orders.len()) as u64;
        }
        groups.retain(|g| !g.orders.is_empty());

        // 5: the micro limiter.
        let limited: Vec<(UnitId, u32)> = groups
            .iter()
            .filter(|g| g.source.is_rate_limited())
            .map(|g| {
                let micro = g
                    .orders
                    .iter()
                    .filter(|(_, o)| o.command.is_micro())
                    .count();
                (g.unit, u32::try_from(micro).unwrap_or(u32::MAX))
            })
            .filter(|&(_, n)| n > 0)
            .collect();
        if !limited.is_empty() {
            let admitted = self.limiter.admit(&limited, tick);
            for g in &mut groups {
                if !limited.iter().any(|&(u, _)| u == g.unit) {
                    continue;
                }
                let micro = g
                    .orders
                    .iter()
                    .filter(|(_, o)| o.command.is_micro())
                    .count() as u64;
                if admitted.contains(&g.unit) {
                    self.stats.limiter_admitted += micro;
                } else {
                    self.stats.limiter_refused += micro;
                    g.orders.clear();
                }
            }
            groups.retain(|g| !g.orders.is_empty());
        }

        // 6 and 7: the gate, then send in staging order.
        let mut sendable: Vec<(usize, Order)> = groups
            .iter()
            .flat_map(|g| g.orders.iter().copied())
            .collect();
        sendable.sort_by_key(|&(i, _)| i);
        if let Some(table) = &self.reservations {
            let mut counts = reservation_counts(own, &self.in_flight, tick);
            let before = sendable.len();
            sendable.retain(|(_, o)| gate_admits(table, &mut counts, &o.command));
            self.stats.gated += (before - sendable.len()) as u64;
        }
        let mut sent: BTreeMap<UnitId, Vec<Order>> = BTreeMap::new();
        for (_, o) in sendable {
            if matches!(o.command, Command::SetOn(_)) {
                self.last_set_on.insert(o.unit, tick);
            }
            out.issue(o);
            self.stats.sent += 1;
            sent.entry(o.unit).or_default().push(o);
        }
        for (unit, orders) in sent {
            self.in_flight.record(unit, tick, orders);
        }
    }
}

/// The winning source among one unit's staged entries: highest priority,
/// ties to the earliest staged.
fn pick_source(entries: &[(usize, Staged)]) -> Option<Source> {
    // (source, highest priority, first staging index)
    let mut per: Vec<(Source, super::Priority, usize)> = Vec::new();
    for (i, s) in entries {
        match per.iter_mut().find(|p| p.0 == s.source) {
            Some(p) => p.1 = p.1.max(s.priority),
            None => per.push((s.source, s.priority, *i)),
        }
    }
    per.into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)))
        .map(|p| p.0)
}

/// Whether `group` would only replace the unit's queue with what it already
/// holds. Factory queue, on/off and stop orders are never no-ops by this
/// test (their effect is not in the visible queue).
fn is_noop(group: &[Order], unit: &OwnUnit) -> bool {
    let Some(first) = group.first() else {
        return false;
    };
    if first.queue != Queue::Immediate || group.iter().skip(1).any(|o| o.queue != Queue::Queued) {
        return false;
    }
    if group.iter().any(|o| {
        matches!(
            o.command,
            Command::FactoryQueue { .. } | Command::SetOn(_) | Command::Stop
        )
    }) {
        return false;
    }
    group.len() == unit.commands.len()
        && group
            .iter()
            .zip(&unit.commands)
            .all(|(o, c)| o.command == *c)
}

/// Counts for the gate: existing units, frames, unplaced build orders (in
/// queues and in flight), and queued factory entries less the unit each
/// factory is holding; plus factory orders in flight.
fn reservation_counts(own: &OwnState, in_flight: &InFlight, tick: Tick) -> BTreeMap<DefId, u64> {
    let mut counts: BTreeMap<DefId, u64> =
        own.total.iter().map(|(&d, &n)| (d, u64::from(n))).collect();
    for s in &own.units {
        if let Some(f) = &s.unit.factory {
            for (i, &d) in f.queue.iter().enumerate() {
                if i == 0 && f.building.is_some() {
                    continue;
                }
                *counts.entry(d).or_insert(0) += 1;
            }
        }
    }
    for o in in_flight.all(tick) {
        if let Command::FactoryQueue { def, count } = o.command {
            if count > 0 {
                *counts.entry(def).or_insert(0) += u64::from(count.unsigned_abs());
            }
        }
    }
    counts
}

fn gate_admits(table: &ReservationTable, counts: &mut BTreeMap<DefId, u64>, c: &Command) -> bool {
    let Some(def) = c.produces() else {
        return true;
    };
    let n = match *c {
        Command::FactoryQueue { count, .. } => u64::from(count.unsigned_abs()),
        _ => 1,
    };
    let Some(&limit) = table.limits.get(&def) else {
        return true;
    };
    let have = counts.entry(def).or_insert(0);
    if *have + n > u64::from(limit) {
        return false;
    }
    *have += n;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::Priority;
    use crate::view::{DefId, Pos};

    fn st(unit: u32, source: Source, priority: Priority, command: Command) -> (usize, Staged) {
        (
            0,
            Staged {
                order: Order {
                    unit: UnitId(unit),
                    command,
                    queue: Queue::Immediate,
                },
                source,
                priority,
            },
        )
    }

    #[test]
    fn priority_then_pipeline_order_picks_the_source() {
        let mv = Command::Move(Pos::new(1, 1));
        let mut e = vec![
            st(1, Source::Economy, Priority::Tasked, mv),
            st(1, Source::Army, Priority::Routine, mv),
        ];
        for (i, x) in e.iter_mut().enumerate() {
            x.0 = i;
        }
        assert_eq!(pick_source(&e), Some(Source::Economy));
        e[1].1.priority = Priority::Urgent;
        assert_eq!(pick_source(&e), Some(Source::Army));
        e[1].1.priority = Priority::Tasked;
        assert_eq!(pick_source(&e), Some(Source::Economy));
    }

    #[test]
    fn gate_counts_and_caps() {
        let t = ReservationTable {
            limits: [(DefId(1), 2), (DefId(2), 0)].into_iter().collect(),
        };
        let mut counts: BTreeMap<DefId, u64> = [(DefId(1), 1)].into_iter().collect();
        let b1 = Command::Build {
            def: DefId(1),
            at: Pos::new(0, 0),
        };
        assert!(gate_admits(&t, &mut counts, &b1));
        assert!(!gate_admits(&t, &mut counts, &b1));
        let q2 = Command::FactoryQueue {
            def: DefId(2),
            count: 1,
        };
        assert!(!gate_admits(&t, &mut counts, &q2));
        let rm = Command::FactoryQueue {
            def: DefId(2),
            count: -3,
        };
        assert!(gate_admits(&t, &mut counts, &rm));
        assert!(gate_admits(&t, &mut counts, &Command::Move(Pos::new(0, 0))));
    }
}
