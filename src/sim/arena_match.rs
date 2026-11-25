//! The arena's [`Match`] for the sim: each seat is this AI, tuned or not.
//!
//! An untuned seat plays the *control* personality; a tuned seat plays the
//! *tuned* personality plus the candidate's overrides. With the two the
//! same, this is the §8 arena as written (the default tuning against
//! itself). With them different, a paired game compares the tuned
//! personality against the control one on the same seed: the control game
//! is control against control, the tuned game puts the tuned personality
//! in the scored seat.
//!
//! The sim is not the real game, so what it measures is how the AI copes
//! with the sim, not how strong it is in the engine.

use super::{Game, Map, Pilot, PlayerSetup, Side, SimConfig};
use crate::arena::{Ended, Match, Outcome, SeatOutcome, Sides};
use crate::personality::{resolve, Difficulty, Faction, Personality, ResolveError};
use crate::view::{PlayerId, Tick, TICKS_PER_SECOND};

/// Plays arena games in the sim on one map.
#[derive(Clone, Debug)]
pub struct SimMatch {
    map: Map,
    control: Personality,
    tuned: Personality,
    difficulty: Difficulty,
    /// Games that could not be played, with why. Such a game is reported
    /// as a timeout with every seat dead, so it scores as a tie.
    pub failures: Vec<(u64, String)>,
    /// Games played.
    pub games: u64,
}

impl SimMatch {
    /// A runner on `map`. Both personalities are resolved once for each
    /// side here, so a bad one fails now rather than game by game.
    pub fn new(
        map: Map,
        control: Personality,
        tuned: Personality,
        difficulty: Difficulty,
    ) -> Result<SimMatch, ResolveError> {
        for p in [&control, &tuned] {
            for f in [Faction::Arm, Faction::Core] {
                resolve(p, difficulty, f, &[])?;
            }
        }
        Ok(SimMatch {
            map,
            control,
            tuned,
            difficulty,
            failures: Vec::new(),
            games: 0,
        })
    }

    fn game(&self, seed: u64, sides: &Sides) -> Result<Outcome, String> {
        let mut cfg = SimConfig::two_player(seed);
        cfg.players = sides
            .seats
            .iter()
            .map(|s| {
                PlayerSetup::new(match s.faction {
                    Faction::Arm => Side::Arm,
                    Faction::Core => Side::Core,
                })
            })
            .collect();
        let mut pilots = Vec::with_capacity(sides.seats.len());
        for s in &sides.seats {
            let (p, ov): (&Personality, Vec<(&str, &str)>) = if s.tuned {
                let ov = s
                    .overrides
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                (&self.tuned, ov)
            } else {
                (&self.control, Vec::new())
            };
            let r = resolve(p, self.difficulty, s.faction, &ov).map_err(|e| e.to_string())?;
            pilots.push(Pilot::Ai(Box::new(r)));
        }
        let mut g = Game::new(self.map.clone(), cfg, pilots).map_err(|e| e.to_string())?;
        let cap = Tick::from(sides.length_s).saturating_mul(TICKS_PER_SECOND);
        let decided = g.run_to(cap).map_err(|e| e.to_string())?;
        let seats = (0..sides.seats.len())
            .map(|i| {
                let p = PlayerId(u8::try_from(i).unwrap_or(u8::MAX));
                SeatOutcome {
                    alive: g.sim.player_alive(p),
                    units: g.sim.unit_count(p),
                    wasted: 0,
                }
            })
            .collect();
        Ok(Outcome {
            ended: if decided {
                Ended::Decided
            } else {
                Ended::Timeout
            },
            seats,
        })
    }
}

impl Match for SimMatch {
    fn play(&mut self, seed: u64, sides: &Sides) -> Outcome {
        self.games = self.games.saturating_add(1);
        match self.game(seed, sides) {
            Ok(o) => o,
            Err(why) => {
                self.failures.push((seed, why));
                Outcome {
                    ended: Ended::Timeout,
                    seats: vec![
                        SeatOutcome {
                            alive: false,
                            units: 0,
                            wasted: 0,
                        };
                        sides.seats.len()
                    ],
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::arena::SeedSchedule;
    use crate::arena::{run_pairs, tuned_sides, Candidate, ControlCache, Phase, Scenario};
    use crate::personality::Registry;

    fn runner(control: &str, tuned: &str) -> SimMatch {
        let reg = Registry::builtin();
        SimMatch::new(
            Map::two_player(),
            reg.get(control).unwrap().clone(),
            reg.get(tuned).unwrap().clone(),
            Difficulty::Standard,
        )
        .unwrap()
    }

    #[test]
    fn a_game_is_a_function_of_its_seed_and_sides() {
        let mut m = runner("Balanced", "Rush");
        let sides = tuned_sides(Scenario::ArmMirror, 3, None, 90, &[]).unwrap();
        let a = m.play(3, &sides);
        let b = m.play(3, &sides);
        assert_eq!(a, b);
        assert_eq!(a.seats.len(), 2);
        assert!(a.seats.iter().all(|s| s.alive && s.units > 1), "{a:?}");
        assert!(m.failures.is_empty(), "{:?}", m.failures);
    }

    #[test]
    fn pairs_share_the_seed_and_the_control_game_is_cached() {
        let mut m = runner("Balanced", "Rush");
        let mut cache = ControlCache::new();
        let mut sched = SeedSchedule::new(11);
        let block = sched.allocate(Phase::Evaluate, 2).unwrap();
        let cand = Candidate::new("Rush", &[]).unwrap();
        let pairs = run_pairs(
            &mut m,
            &mut cache,
            &cand,
            Scenario::ArmMirror,
            None,
            &block,
            60,
        )
        .unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            pairs.iter().map(|p| p.seed).collect::<Vec<_>>(),
            vec![11, 12]
        );
        assert_eq!((m.games, cache.len()), (4, 2));
        // Played again, the control games come from the cache.
        run_pairs(
            &mut m,
            &mut cache,
            &cand,
            Scenario::ArmMirror,
            None,
            &block,
            60,
        )
        .unwrap();
        assert_eq!((m.games, cache.len()), (6, 2));
    }

    #[test]
    fn a_bad_override_is_a_recorded_failure_not_a_panic() {
        let mut m = runner("Balanced", "Balanced");
        let ov = vec![("army.wave_size".to_string(), "lots".to_string())];
        let sides = tuned_sides(Scenario::ArmMirror, 1, None, 30, &ov).unwrap();
        let o = m.play(1, &sides);
        assert_eq!(o.ended, Ended::Timeout);
        assert!(o.seats.iter().all(|s| !s.alive));
        assert_eq!(m.failures.len(), 1);
    }
}
