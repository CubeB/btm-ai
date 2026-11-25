//! The write side of the contract: the order stream (SPEC §2.1).

use super::types::{DefId, FeatureId, Pos, UnitId};

/// How an order joins a unit's queue (SPEC §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Queue {
    /// Replace the queue.
    Immediate,
    /// Append to the queue.
    Queued,
}

/// One command, as a human could give it (SPEC §2.1 order table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// Build type `def` with its footprint centred at `at` (builders, §4).
    Build { def: DefId, at: Pos },
    /// Factory queue: add `count` of `def`, or remove `-count` if negative
    /// (§4.4). The queue mode is ignored.
    FactoryQueue { def: DefId, count: i32 },
    /// Move to a point. A move to the unit's own position is "stop" (§2.1).
    Move(Pos),
    /// Attack a unit. The target must be seen now (§1.3 step 3).
    Attack(UnitId),
    /// Attack ground / attack-move: fight whatever is met on the way (waves).
    AttackMove(Pos),
    /// One patrol point; send the first Immediate, the rest Queued (§2.1).
    Patrol(Pos),
    /// Guard a unit (AA escorts, builders assisting a factory).
    Guard(UnitId),
    /// Repair a unit, or help build a frame.
    Repair(UnitId),
    /// Reclaim a feature (wrecks, §4.12).
    ReclaimFeature(FeatureId),
    /// Reclaim a unit (own extractor before an upgrade, §4.5).
    ReclaimUnit(UnitId),
    /// Capture a unit. In the sink for completeness: the AI never issues it
    /// (§2.1, §5.6).
    Capture(UnitId),
    /// Fire the commander's special weapon at a unit (§5.7).
    DGun(UnitId),
    /// A transport loads a unit (§5.9).
    Load(UnitId),
    /// A transport sets down one unit at a point (§5.9). Never queue two
    /// identical unloads: the game toggles duplicates off.
    Unload(Pos),
    /// Clear the queue (transports calling off a trip).
    Stop,
    /// Switch on or off (metal makers, §4.9). The queue mode is ignored.
    SetOn(bool),
}

impl Command {
    /// Orders the micro limiter rate-limits: move, attack, attack-move,
    /// patrol, d-gun and guard (SPEC §2.3). Repairs, builds, factory and
    /// transport orders pass untouched.
    pub fn is_micro(&self) -> bool {
        matches!(
            self,
            Command::Move(_)
                | Command::Attack(_)
                | Command::AttackMove(_)
                | Command::Patrol(_)
                | Command::DGun(_)
                | Command::Guard(_)
        )
    }

    /// Orders a build or queue entry of a type, for the mission reservation
    /// gate (SPEC §2.4). Removals are not counted.
    pub fn produces(&self) -> Option<DefId> {
        match *self {
            Command::Build { def, .. } => Some(def),
            Command::FactoryQueue { def, count } if count > 0 => Some(def),
            _ => None,
        }
    }
}

/// An order to one unit (SPEC §2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Order {
    pub unit: UnitId,
    pub command: Command,
    pub queue: Queue,
}

/// The order sink (SPEC §2.1). Orders go through the normal player command
/// stream; the host applies its fixed delay (SPEC §7).
pub trait Orders {
    fn issue(&mut self, order: Order);

    /// Shorthand for an Immediate order.
    fn now(&mut self, unit: UnitId, command: Command) {
        self.issue(Order {
            unit,
            command,
            queue: Queue::Immediate,
        });
    }

    /// Shorthand for a Queued order.
    fn queued(&mut self, unit: UnitId, command: Command) {
        self.issue(Order {
            unit,
            command,
            queue: Queue::Queued,
        });
    }
}

/// An [`Orders`] sink that records orders in issue order. Used by tests, and
/// as a staging buffer between an AI pass and the host (the micro limiter
/// and the reservation gate filter it before sending, SPEC §2.3, §2.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderRecorder {
    orders: Vec<Order>,
}

impl OrderRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every recorded order, in issue order.
    pub fn orders(&self) -> &[Order] {
        &self.orders
    }

    /// The recorded orders for one unit, in issue order.
    pub fn for_unit(&self, unit: UnitId) -> impl Iterator<Item = &Order> + '_ {
        self.orders.iter().filter(move |o| o.unit == unit)
    }

    pub fn len(&self) -> usize {
        self.orders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Removes and returns every recorded order.
    pub fn take(&mut self) -> Vec<Order> {
        std::mem::take(&mut self.orders)
    }

    /// Keeps only the orders for which `keep` is true (limiter, gate).
    pub fn retain(&mut self, keep: impl FnMut(&Order) -> bool) {
        self.orders.retain(keep);
    }
}

impl Orders for OrderRecorder {
    fn issue(&mut self, order: Order) {
        self.orders.push(order);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_keeps_issue_order_and_modes() {
        let mut rec = OrderRecorder::new();
        let a = UnitId(1);
        let b = UnitId(2);
        rec.now(a, Command::Patrol(Pos::new(1, 1)));
        rec.queued(a, Command::Patrol(Pos::new(2, 2)));
        rec.now(b, Command::Stop);
        rec.issue(Order {
            unit: b,
            command: Command::FactoryQueue {
                def: DefId(3),
                count: -2,
            },
            queue: Queue::Queued,
        });
        assert_eq!(rec.len(), 4);
        let mine: Vec<_> = rec.for_unit(a).collect();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].queue, Queue::Immediate);
        assert_eq!(mine[1].command, Command::Patrol(Pos::new(2, 2)));
        assert_eq!(rec.orders()[2].command, Command::Stop);
        rec.retain(|o| !o.command.is_micro());
        assert_eq!(rec.len(), 2);
        let all = rec.take();
        assert_eq!(all[0].unit, b);
        assert!(rec.is_empty());
    }

    #[test]
    fn command_classes() {
        assert!(Command::DGun(UnitId(1)).is_micro());
        assert!(Command::Guard(UnitId(1)).is_micro());
        assert!(!Command::Repair(UnitId(1)).is_micro());
        assert!(!Command::Load(UnitId(1)).is_micro());
        let b = Command::Build {
            def: DefId(4),
            at: Pos::new(0, 0),
        };
        assert_eq!(b.produces(), Some(DefId(4)));
        let rm = Command::FactoryQueue {
            def: DefId(4),
            count: -1,
        };
        assert_eq!(rm.produces(), None);
    }
}
