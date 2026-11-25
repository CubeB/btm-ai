//! Which passes run on which ticks (SPEC §2.2).
//!
//! The AI is ticked every simulation tick and decides on fixed intervals of
//! game ticks; wall-clock time is never used. A pass with interval `every`
//! and first tick `first` runs on ticks `first, first + every, ...`.
//!
//! | Pass | First tick | Interval |
//! |---|---|---|
//! | Own-state sampling, contact memory, phase machine, metal makers | 0 | every tick |
//! | Influence grid | 0 | `cadence.threat_grid_ticks` (30) |
//! | Reachability re-home | first tick with a home | 600 (20 s) |
//! | Build planning | 0 | `cadence.plan_ticks` (30; Easy 60, Hard 20) |
//! | Builder-safety sweep, commander-repair dispatch, defence attack watch | 5 | 15 |
//! | Army, navy, air and transports | 15 | `cadence.tactics_ticks` (15; Easy 30, Hard 10) |
//! | Scouts | 10 | `cadence.scout_ticks` (60; Easy 120, Hard 45) |
//! | Radar warning sample | 20 | 30 (1 s) |
//!
//! The spec fixes only the first tactics pass ("on the 15th tick, not tick
//! 0"). The other first ticks are this implementation's stagger: they keep
//! the 15-tick sweep, the scouts and the radar sample off the ticks where the
//! planner and the grid run, so that no tick carries every pass at once.
//! Within a tick the pipeline order of SPEC §3.1 always holds.

use std::fmt;

use crate::knobs::Knobs;
use crate::view::Tick;

/// A pass's schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Every {
    pub first: Tick,
    pub every: Tick,
}

impl Every {
    pub const fn new(first: Tick, every: Tick) -> Self {
        Every { first, every }
    }

    /// Whether the pass runs on `tick`.
    pub fn due(self, tick: Tick) -> bool {
        tick >= self.first && (tick - self.first).is_multiple_of(self.every.max(1))
    }
}

/// The seat's pass schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    pub grid: Every,
    pub plan: Every,
    pub sweep: Every,
    pub tactics: Every,
    pub scouts: Every,
    pub radar: Every,
}

/// A cadence knob is not a positive tick count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CadenceError {
    pub knob: &'static str,
    pub value: i32,
}

impl fmt::Display for CadenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` must be at least 1 tick, not {}",
            self.knob, self.value
        )
    }
}

impl std::error::Error for CadenceError {}

/// The 15-tick sweep's interval (SPEC §2.2).
pub const SWEEP_TICKS: Tick = 15;
/// The first tactics pass (SPEC §2.2).
pub const FIRST_TACTICS: Tick = 15;
/// The radar warning's sampling interval: 1 s.
pub const RADAR_TICKS: Tick = 30;

impl Cadence {
    /// The schedule for a seat's knobs.
    pub fn from_knobs(k: &Knobs) -> Result<Cadence, CadenceError> {
        let ticks = |knob: &'static str, value: i32| {
            u32::try_from(value)
                .ok()
                .filter(|&v| v > 0)
                .ok_or(CadenceError { knob, value })
        };
        Ok(Cadence {
            grid: Every::new(
                0,
                ticks("cadence.threat_grid_ticks", k.cadence.threat_grid_ticks)?,
            ),
            plan: Every::new(0, ticks("cadence.plan_ticks", k.cadence.plan_ticks)?),
            sweep: Every::new(5, SWEEP_TICKS),
            tactics: Every::new(
                FIRST_TACTICS,
                ticks("cadence.tactics_ticks", k.cadence.tactics_ticks)?,
            ),
            scouts: Every::new(10, ticks("cadence.scout_ticks", k.cadence.scout_ticks)?),
            radar: Every::new(20, RADAR_TICKS),
        })
    }

    /// Which passes are due on `tick` (the re-home is decided separately,
    /// from the reachability state).
    pub fn due(&self, tick: Tick, reach: bool) -> Due {
        Due {
            grid: self.grid.due(tick),
            reach,
            plan: self.plan.due(tick),
            sweep: self.sweep.due(tick),
            tactics: self.tactics.due(tick),
            scouts: self.scouts.due(tick),
            radar: self.radar.due(tick),
        }
    }
}

/// The passes due this tick, given to managers in [`super::Ctx`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Due {
    /// The influence grid was rebuilt this tick.
    pub grid: bool,
    /// Reachability was re-homed this tick.
    pub reach: bool,
    /// Build planning (economy).
    pub plan: bool,
    /// Builder-safety sweep, commander-repair dispatch, defence attack
    /// watch (army).
    pub sweep: bool,
    /// Army, navy, air and transports.
    pub tactics: bool,
    /// Scouts.
    pub scouts: bool,
    /// Radar warning sample (army, in its phase step).
    pub radar: bool,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::personality::Difficulty;

    #[test]
    fn standard_schedule() {
        let c = Cadence::from_knobs(&Knobs::default()).unwrap();
        let ticks = |e: Every| (0..100).filter(|&t| e.due(t)).collect::<Vec<_>>();
        assert_eq!(ticks(c.plan), vec![0, 30, 60, 90]);
        assert_eq!(ticks(c.tactics), vec![15, 30, 45, 60, 75, 90]);
        assert_eq!(ticks(c.sweep), vec![5, 20, 35, 50, 65, 80, 95]);
        assert_eq!(ticks(c.scouts), vec![10, 70]);
    }

    #[test]
    fn tiers_change_intervals_and_bad_knobs_are_errors() {
        let mut k = Knobs::default();
        Difficulty::Easy.apply(&mut k).unwrap();
        let c = Cadence::from_knobs(&k).unwrap();
        assert_eq!(
            (c.plan.every, c.tactics.every, c.scouts.every),
            (60, 30, 120)
        );
        let mut k = Knobs::default();
        Difficulty::Hard.apply(&mut k).unwrap();
        let c = Cadence::from_knobs(&k).unwrap();
        assert_eq!(
            (c.plan.every, c.tactics.every, c.scouts.every),
            (20, 10, 45)
        );
        k.cadence.plan_ticks = 0;
        assert_eq!(
            Cadence::from_knobs(&k),
            Err(CadenceError {
                knob: "cadence.plan_ticks",
                value: 0
            })
        );
    }
}
