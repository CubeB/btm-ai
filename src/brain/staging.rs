//! Staging: where managers put their orders before the dispatcher filters
//! them (ownership, conflicts, de-duplication, the micro limiter of SPEC
//! §2.3 and the reservation gate of §2.4) and sends them.
//!
//! Also [`InFlight`]: the orders sent in the last few ticks that the game
//! has not applied yet. AI orders reach the game through the command buffer
//! at a fixed delay of 7 ticks (SPEC §7), so for that long a unit's visible
//! queue does not show them. Managers must treat a unit with orders in flight
//! as busy, and the dispatcher drops an identical group re-sent while the
//! first is still in flight.

use std::collections::BTreeMap;

use super::ownership::Source;
use crate::view::{Order, Orders, Tick, UnitId};

/// How strongly a manager wants its orders for a unit this tick.
///
/// When two managers stage orders for the same (shared) unit in one tick,
/// the higher priority wins and the loser's orders for that unit are dropped;
/// ties go to the earlier pipeline step (SPEC §3.1: the planner records that
/// it tasked the commander "so that the army pass does not overwrite it").
/// Priority never changes the micro limiter's service order (SPEC §2.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Ordinary orders. The default.
    #[default]
    Routine,
    /// A job the unit should keep: a planner build or repair.
    Tasked,
    /// Survival: fleeing, sheltering, the d-gun.
    Urgent,
}

/// One staged order with its origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Staged {
    pub order: Order,
    pub source: Source,
    pub priority: Priority,
}

/// The orders staged this tick, in staging order. Managers write to it
/// through [`Orders`] (`out.now(unit, command)`), which tags each order with
/// the running manager's [`Source`] and the current [`Priority`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Staging {
    entries: Vec<Staged>,
    source: Option<Source>,
    priority: Priority,
}

impl Staging {
    /// Starts a manager's turn: later orders carry `source`, at Routine.
    pub(crate) fn begin(&mut self, source: Source) {
        self.source = Some(source);
        self.priority = Priority::Routine;
    }

    /// The manager whose turn it is.
    pub fn source(&self) -> Option<Source> {
        self.source
    }

    /// Sets the priority of the following orders; returns the previous one.
    pub fn set_priority(&mut self, p: Priority) -> Priority {
        std::mem::replace(&mut self.priority, p)
    }

    pub fn priority(&self) -> Priority {
        self.priority
    }

    /// Stages one order at an explicit priority.
    pub fn issue_with(&mut self, order: Order, priority: Priority) {
        if let Some(source) = self.source {
            self.entries.push(Staged {
                order,
                source,
                priority,
            });
        }
    }

    /// Everything staged so far this tick, in staging order.
    pub fn staged(&self) -> &[Staged] {
        &self.entries
    }

    /// Whether any manager staged an order for `unit` this tick.
    pub fn has_order_for(&self, unit: UnitId) -> bool {
        self.entries.iter().any(|s| s.order.unit == unit)
    }

    /// Whether `source` staged an order for `unit` this tick.
    pub fn has_order_from(&self, unit: UnitId, source: Source) -> bool {
        self.entries
            .iter()
            .any(|s| s.order.unit == unit && s.source == source)
    }

    pub(crate) fn take(&mut self) -> Vec<Staged> {
        self.source = None;
        std::mem::take(&mut self.entries)
    }
}

impl Orders for Staging {
    fn issue(&mut self, order: Order) {
        self.issue_with(order, self.priority);
    }
}

/// One group of orders sent to a unit in one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentGroup {
    pub tick: Tick,
    pub orders: Vec<Order>,
}

/// Orders sent but not yet visible in the game (SPEC §7 fixed delay).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InFlight {
    delay: Tick,
    sent: BTreeMap<UnitId, Vec<SentGroup>>,
}

impl Default for InFlight {
    fn default() -> Self {
        InFlight::new(DEFAULT_ORDER_DELAY)
    }
}

/// The command buffer depth for AI orders (SPEC §7).
pub const DEFAULT_ORDER_DELAY: Tick = 7;

impl InFlight {
    pub fn new(delay: Tick) -> Self {
        InFlight {
            delay,
            sent: BTreeMap::new(),
        }
    }

    /// The order delay in ticks.
    pub fn delay(&self) -> Tick {
        self.delay
    }

    pub(crate) fn set_delay(&mut self, delay: Tick) {
        self.delay = delay;
    }

    fn live(&self, g: &SentGroup, now: Tick) -> bool {
        now <= g.tick.saturating_add(self.delay)
    }

    /// The groups sent to `unit` that are still in flight, oldest first.
    pub fn groups(&self, unit: UnitId, now: Tick) -> impl Iterator<Item = &SentGroup> + '_ {
        self.sent
            .get(&unit)
            .into_iter()
            .flatten()
            .filter(move |g| self.live(g, now))
    }

    /// The newest in-flight group for `unit`, if any.
    pub fn orders(&self, unit: UnitId, now: Tick) -> Option<&[Order]> {
        self.groups(unit, now).last().map(|g| g.orders.as_slice())
    }

    /// Every in-flight order, by unit id, then send order.
    pub fn all(&self, now: Tick) -> impl Iterator<Item = &Order> + '_ {
        self.sent
            .values()
            .flatten()
            .filter(move |g| self.live(g, now))
            .flat_map(|g| g.orders.iter())
    }

    pub(crate) fn record(&mut self, unit: UnitId, tick: Tick, orders: Vec<Order>) {
        self.sent
            .entry(unit)
            .or_default()
            .push(SentGroup { tick, orders });
    }

    /// Drops groups that have landed.
    pub(crate) fn prune(&mut self, now: Tick) {
        let delay = self.delay;
        for groups in self.sent.values_mut() {
            groups.retain(|g| now <= g.tick.saturating_add(delay));
        }
        self.sent.retain(|_, g| !g.is_empty());
    }
}
