//! The micro limiter (SPEC §2.3).
//!
//! After the army and air passes, their move, attack, patrol, d-gun and guard
//! orders are rate-limited to the difficulty's orders per minute
//! ([`crate::personality::DifficultyProfile::micro_limit`]; no limit on Hard
//! and Brutal):
//!
//! - a token bucket with a burst of 4 that refills pro rata each tick and
//!   starts full, and a sliding 60 s window whose total may not exceed the
//!   rate;
//! - a unit's orders in one tick are admitted or refused as a group;
//! - units are served never-served first, then least recently served (ties
//!   by unit id);
//! - once one unit is refused, every later unit that tick is refused too;
//! - refused orders are dropped; the next pass regenerates them.
//!
//! Integer arithmetic only: one order is worth [`TICKS_PER_MINUTE`] bucket
//! units, and the bucket gains `rate` units per tick.
//!
//! A group of more than 4 orders could never fit the bucket and would then
//! block every unit served after it, for ever. Such a group is charged the
//! full burst instead (the window still counts every order).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::view::{Tick, UnitId};

/// Ticks in one minute.
pub const TICKS_PER_MINUTE: Tick = 60 * 30;
/// The bucket's burst, in orders.
pub const BURST: u32 = 4;

/// The limiter's state. `rate == None` is no limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MicroLimiter {
    rate: Option<u32>,
    tokens: i64,
    last_refill: Option<Tick>,
    window: VecDeque<(Tick, u32)>,
    window_total: u32,
    served: BTreeMap<UnitId, Tick>,
}

impl MicroLimiter {
    /// A full limiter for `rate` orders per minute (`None`: unlimited).
    pub fn new(rate: Option<u32>) -> Self {
        MicroLimiter {
            rate,
            tokens: Self::capacity(),
            last_refill: None,
            window: VecDeque::new(),
            window_total: 0,
            served: BTreeMap::new(),
        }
    }

    fn capacity() -> i64 {
        i64::from(BURST) * i64::from(TICKS_PER_MINUTE)
    }

    /// The configured rate.
    pub fn rate(&self) -> Option<u32> {
        self.rate
    }

    /// Orders admitted in the last 60 s.
    pub fn window_total(&self) -> u32 {
        self.window_total
    }

    /// Admits or refuses each unit's group. `groups` is `(unit, orders)`;
    /// the result is the set of admitted units.
    pub fn admit(&mut self, groups: &[(UnitId, u32)], tick: Tick) -> BTreeSet<UnitId> {
        let Some(rate) = self.rate else {
            return groups.iter().map(|&(u, _)| u).collect();
        };
        self.refill(rate, tick);
        let mut order: Vec<(UnitId, u32)> = groups.to_vec();
        order.sort_by_key(|&(u, _)| (self.served.get(&u).copied(), u));
        let mut admitted: BTreeSet<UnitId> = order
            .iter()
            .filter(|&&(_, c)| c == 0)
            .map(|&(u, _)| u)
            .collect();
        order.retain(|&(_, c)| c > 0);
        for (unit, cost) in order {
            let charge = i64::from(cost.min(BURST)) * i64::from(TICKS_PER_MINUTE);
            let fits_window = self.window_total.saturating_add(cost) <= rate;
            if charge > self.tokens || !fits_window {
                break;
            }
            self.tokens -= charge;
            self.window_total += cost;
            self.window.push_back((tick, cost));
            self.served.insert(unit, tick);
            admitted.insert(unit);
        }
        admitted
    }

    /// Forgets units that no longer exist (keeps the map small).
    pub fn retain_units(&mut self, alive: impl Fn(UnitId) -> bool) {
        self.served.retain(|&u, _| alive(u));
    }

    fn refill(&mut self, rate: u32, tick: Tick) {
        let elapsed = match self.last_refill {
            Some(t) => tick.saturating_sub(t),
            None => 0,
        };
        self.last_refill = Some(tick);
        self.tokens = self
            .tokens
            .saturating_add(i64::from(rate).saturating_mul(i64::from(elapsed)))
            .min(Self::capacity());
        while let Some(&(t, n)) = self.window.front() {
            if tick.saturating_sub(t) < TICKS_PER_MINUTE {
                break;
            }
            self.window.pop_front();
            self.window_total = self.window_total.saturating_sub(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_admits_everything() {
        let mut l = MicroLimiter::new(None);
        let g: Vec<_> = (0..50).map(|i| (UnitId(i), 3)).collect();
        assert_eq!(l.admit(&g, 0).len(), 50);
    }

    #[test]
    fn burst_then_pro_rata_refill_and_stop_after_first_refusal() {
        let mut l = MicroLimiter::new(Some(120));
        // Burst of 4: units 1..=4 with one order each; the fifth is refused,
        // and so is the sixth.
        let g = [
            (UnitId(1), 1),
            (UnitId(2), 1),
            (UnitId(3), 1),
            (UnitId(4), 1),
            (UnitId(5), 1),
            (UnitId(6), 1),
        ];
        let a = l.admit(&g, 0);
        assert_eq!(a.len(), 4);
        assert!(!a.contains(&UnitId(5)) && !a.contains(&UnitId(6)));
        // 120/min is one order per 15 ticks.
        assert!(l.admit(&g, 14).is_empty());
        let a = l.admit(&g, 15);
        // Never-served units first: 5 is served before 1.
        assert_eq!(a.into_iter().collect::<Vec<_>>(), vec![UnitId(5)]);
        // An empty group passes without being served.
        assert!(l.admit(&[(UnitId(9), 0)], 16).contains(&UnitId(9)));
    }

    #[test]
    fn sliding_window_caps_a_minute() {
        let mut l = MicroLimiter::new(Some(60));
        let mut sent = 0;
        for t in 0..TICKS_PER_MINUTE {
            sent += l.admit(&[(UnitId(t % 7), 1)], t).len();
        }
        assert!(sent <= 60, "{sent}");
        assert!(sent >= 59, "{sent}");
    }
}
