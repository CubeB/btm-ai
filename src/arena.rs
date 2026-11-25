//! The tuning arena (SPEC.md section 8): paired games, scoring, statistics and
//! a text report.
//!
//! The arena never plays a game itself. A game runner (the simulator, or a real
//! engine) implements [`Match`], and the arena asks it to play each pair:
//! a **control** game with no overrides and a **tuned** game with the
//! candidate's knob overrides, on the same seed. Control games are cached and
//! shared by all candidates.
//!
//! - [`schedule`]: disjoint seed blocks per phase, scenarios and side
//!   swapping.
//! - [`stats`]: Wilson score interval and the exact sign test.
//! - [`report`]: the plain-text report.
//!
//! The analysis implemented is the first of section 8.3 (Wilson interval on
//! the fraction of *better* games, sign test over better versus worse,
//! fitness-ranked); the second analyser (win/draw/loss, bootstrap and power)
//! is not built.

pub mod report;
pub mod schedule;
pub mod stats;

use std::collections::BTreeMap;
use std::fmt;

pub use report::{render_report, ReportInfo};
pub use schedule::{scored_seat, tuned_sides, Phase, Scenario, SeedBlock, SeedSchedule};
pub use stats::{sign_test, wilson, SignTest, DEFAULT_Z};

use crate::knobs::knob_info;
use crate::personality::Faction;

/// Why an arena operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArenaError {
    /// A seed block would run past `u64::MAX`.
    SeedOverflow,
    /// A faction restriction was given for a scenario other than the cross.
    RestrictionNeedsCross,
    /// An override names no knob, or its value does not parse. Unknown knobs
    /// are fatal in the arena (SPEC.md 6.1).
    BadOverride {
        /// The override's knob name.
        knob: String,
        /// Why it was rejected.
        reason: String,
    },
    /// The game length is zero.
    ZeroLength,
    /// The normal quantile is negative or not finite.
    BadZ,
    /// The scored seat does not exist.
    ScoredSeatOutOfRange {
        /// The seat asked for.
        seat: usize,
        /// How many seats the game has.
        seats: usize,
    },
    /// A game needs at least two seats.
    TooFewSeats,
    /// The match returned a different number of seats than it was given.
    OutcomeSeatMismatch {
        /// Seats the sides had.
        expected: usize,
        /// Seats the outcome reported.
        got: usize,
    },
}

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArenaError::SeedOverflow => write!(f, "seed block runs past the largest seed"),
            ArenaError::RestrictionNeedsCross => {
                write!(
                    f,
                    "a faction restriction only applies to the cross scenario"
                )
            }
            ArenaError::BadOverride { knob, reason } => {
                write!(f, "override `{knob}`: {reason}")
            }
            ArenaError::ZeroLength => write!(f, "game length must be above zero"),
            ArenaError::BadZ => write!(f, "z must be finite and not negative"),
            ArenaError::ScoredSeatOutOfRange { seat, seats } => {
                write!(f, "scored seat {seat} but the game has {seats} seats")
            }
            ArenaError::TooFewSeats => write!(f, "a game needs at least two seats"),
            ArenaError::OutcomeSeatMismatch { expected, got } => {
                write!(f, "outcome has {got} seats, expected {expected}")
            }
        }
    }
}

impl std::error::Error for ArenaError {}

/// One seat of a game to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatSpec {
    /// The seat's side.
    pub faction: Faction,
    /// True if the seat plays the candidate tuning.
    pub tuned: bool,
    /// The candidate's `knob=value` overrides for this seat (empty unless
    /// tuned), in command-line order.
    pub overrides: Vec<(String, String)>,
}

/// The sides of one game: who sits where, who is tuned, who is scored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sides {
    /// The seats, in seat order.
    pub seats: Vec<SeatSpec>,
    /// The seat whose result is scored.
    pub scored: usize,
    /// The game-time cap in seconds.
    pub length_s: u32,
}

impl Sides {
    /// Builds sides; the overrides go to every seat flagged in `tuned`.
    pub fn new(
        factions: Vec<Faction>,
        tuned: &[bool],
        scored: usize,
        length_s: u32,
        overrides: &[(String, String)],
    ) -> Sides {
        let seats = factions
            .into_iter()
            .enumerate()
            .map(|(i, faction)| {
                let is_tuned = tuned.get(i).copied().unwrap_or(false);
                SeatSpec {
                    faction,
                    tuned: is_tuned,
                    overrides: if is_tuned {
                        overrides.to_vec()
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();
        Sides {
            seats,
            scored,
            length_s,
        }
    }

    /// The control game for these sides: the same seats, nobody tuned.
    pub fn control(&self) -> Sides {
        Sides {
            seats: self
                .seats
                .iter()
                .map(|s| SeatSpec {
                    faction: s.faction,
                    tuned: false,
                    overrides: Vec::new(),
                })
                .collect(),
            scored: self.scored,
            length_s: self.length_s,
        }
    }

    fn validate(&self) -> Result<(), ArenaError> {
        if self.seats.len() < 2 {
            return Err(ArenaError::TooFewSeats);
        }
        if self.length_s == 0 {
            return Err(ArenaError::ZeroLength);
        }
        if self.scored >= self.seats.len() {
            return Err(ArenaError::ScoredSeatOutOfRange {
                seat: self.scored,
                seats: self.seats.len(),
            });
        }
        Ok(())
    }

    fn factions(&self) -> Vec<Faction> {
        self.seats.iter().map(|s| s.faction).collect()
    }
}

/// How a game ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The game reported a decision (a win or a draw).
    Decided,
    /// The game-time cap or the watchdog ended it.
    Timeout,
}

/// One seat's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeatOutcome {
    /// True if the seat still had its commander (or whatever the
    /// commander-death rule keeps a seat alive by).
    pub alive: bool,
    /// The seat's final unit count. Ignored when the seat is dead (a dead
    /// seat counts as 0).
    pub units: u32,
    /// Resources wasted over the game, in whatever unit the runner reports
    /// (used only when waste scoring is on).
    pub wasted: u64,
}

/// The result of one game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// How the game ended.
    pub ended: Ended,
    /// One entry per seat, in seat order.
    pub seats: Vec<SeatOutcome>,
}

/// A game runner. Given the same seed and sides it must return the same
/// outcome: a game is deterministic (SPEC.md section 8.1).
pub trait Match {
    /// Plays one headless game to its end (a decision, `sides.length_s`, or
    /// the runner's own watchdog).
    fn play(&mut self, seed: u64, sides: &Sides) -> Outcome;
}

/// The score of one game for its scored seat.
///
/// The margin is the scored seat's unit count minus the mean of its
/// opponents' counts (a dead seat counts 0). To stay exact it is kept as a
/// fraction: `margin = margin_num / opponents`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameScore {
    /// `opponents * own_units - sum(opponent_units)`.
    pub margin_num: i64,
    /// The number of opponents (seats minus one).
    pub opponents: u32,
    /// The game ended decided with the scored seat alive and all others dead.
    pub decided_win: bool,
    /// The scored seat's waste.
    pub wasted: u64,
}

impl GameScore {
    /// The margin as a number.
    pub fn margin(&self) -> f64 {
        self.margin_num as f64 / f64::from(self.opponents.max(1))
    }
}

/// Scores `outcome` for `scored`.
pub fn score(outcome: &Outcome, scored: usize) -> Result<GameScore, ArenaError> {
    let seat = outcome
        .seats
        .get(scored)
        .ok_or(ArenaError::ScoredSeatOutOfRange {
            seat: scored,
            seats: outcome.seats.len(),
        })?;
    if outcome.seats.len() < 2 {
        return Err(ArenaError::TooFewSeats);
    }
    let count = |s: &SeatOutcome| if s.alive { i64::from(s.units) } else { 0 };
    let opponents = outcome.seats.len() as i64 - 1;
    let others: i64 = outcome
        .seats
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != scored)
        .map(|(_, s)| count(s))
        .sum();
    let others_dead = outcome
        .seats
        .iter()
        .enumerate()
        .all(|(i, s)| i == scored || !s.alive);
    Ok(GameScore {
        margin_num: opponents * count(seat) - others,
        opponents: opponents as u32,
        decided_win: outcome.ended == Ended::Decided && seat.alive && others_dead,
        wasted: seat.wasted,
    })
}

/// The scoring and analysis parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatsConfig {
    /// The normal quantile of the Wilson interval (1.96 gives 95%).
    pub z: f64,
    /// A pair is *better* above `+margin_threshold` and *worse* below
    /// `-margin_threshold` (10 in the spec).
    pub margin_threshold: i64,
    /// The weight of the decided-win-rate difference in fitness (20).
    pub win_weight: f64,
    /// The weight of a waste increase in fitness (0.3).
    pub waste_weight: f64,
    /// Whether waste counts in fitness at all.
    pub waste_scoring: bool,
}

impl Default for StatsConfig {
    fn default() -> StatsConfig {
        StatsConfig {
            z: DEFAULT_Z,
            margin_threshold: 10,
            win_weight: 20.0,
            waste_weight: 0.3,
            waste_scoring: false,
        }
    }
}

impl StatsConfig {
    /// The default configuration with another `z`, which must be finite and
    /// not negative.
    pub fn with_z(z: f64) -> Result<StatsConfig, ArenaError> {
        if z.is_finite() && z >= 0.0 {
            Ok(StatsConfig {
                z,
                ..StatsConfig::default()
            })
        } else {
            Err(ArenaError::BadZ)
        }
    }
}

/// How a tuned game compares with its control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Delta above +threshold.
    Better,
    /// Delta within the threshold (inclusive).
    Same,
    /// Delta below -threshold.
    Worse,
}

/// A tuned game and its control, scored for the same seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairResult {
    /// The scenario.
    pub scenario: Scenario,
    /// The seed (shared by both games).
    pub seed: u64,
    /// The tuned game's score.
    pub tuned: GameScore,
    /// The control game's score.
    pub control: GameScore,
}

impl PairResult {
    /// `tuned margin - control margin` times the number of opponents, an
    /// exact integer (both games have the same number of opponents).
    pub fn delta_num(&self) -> i64 {
        self.tuned.margin_num - self.control.margin_num
    }

    /// The delta: tuned margin minus control margin.
    pub fn delta(&self) -> f64 {
        self.delta_num() as f64 / f64::from(self.tuned.opponents.max(1))
    }

    /// Better above +threshold, worse below -threshold, else same. The
    /// comparison is exact.
    pub fn verdict(&self, cfg: &StatsConfig) -> Verdict {
        let limit = cfg
            .margin_threshold
            .saturating_mul(i64::from(self.tuned.opponents.max(1)));
        let d = self.delta_num();
        if d > limit {
            Verdict::Better
        } else if d < -limit {
            Verdict::Worse
        } else {
            Verdict::Same
        }
    }
}

/// A named set of knob overrides under test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// A display name.
    pub name: String,
    /// `knob=value` overrides in command-line order. Validated on creation.
    pub overrides: Vec<(String, String)>,
}

impl Candidate {
    /// Builds a candidate, rejecting any unknown knob or unparseable value
    /// (an unknown knob is fatal in the arena).
    pub fn new(name: &str, overrides: &[(&str, &str)]) -> Result<Candidate, ArenaError> {
        for (knob, text) in overrides {
            let info = knob_info(knob).ok_or_else(|| ArenaError::BadOverride {
                knob: (*knob).to_string(),
                reason: "unknown knob".to_string(),
            })?;
            info.parse_value(text)
                .map_err(|e| ArenaError::BadOverride {
                    knob: (*knob).to_string(),
                    reason: e.to_string(),
                })?;
        }
        Ok(Candidate {
            name: name.to_string(),
            overrides: overrides
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        })
    }

    /// The candidate's id: a 64-bit FNV-1a hash of its knob values sorted by
    /// knob name, each as `name=value` with the value in canonical form (so
    /// `1.250` and `1.25` hash alike). Where a knob is overridden twice the
    /// later value wins.
    pub fn id(&self) -> u64 {
        let mut sorted: BTreeMap<&str, String> = BTreeMap::new();
        for (k, v) in &self.overrides {
            let canon = knob_info(k)
                .and_then(|i| i.parse_value(v).ok())
                .map_or_else(|| v.trim().to_string(), |val| val.to_string());
            sorted.insert(k.as_str(), canon);
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (k, v) in &sorted {
            for b in k.bytes().chain(*b"=").chain(v.bytes()).chain(*b"\n") {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }
}

type ControlKey = (u64, Vec<Faction>, u32);

/// Control games, played once per (seed, seating, length) and shared by every
/// candidate.
#[derive(Debug, Clone, Default)]
pub struct ControlCache {
    games: BTreeMap<ControlKey, Outcome>,
}

impl ControlCache {
    /// An empty cache.
    pub fn new() -> ControlCache {
        ControlCache::default()
    }

    /// How many control games are held.
    pub fn len(&self) -> usize {
        self.games.len()
    }

    /// True if no control game is held.
    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    fn get_or_play<M: Match>(
        &mut self,
        m: &mut M,
        seed: u64,
        control: &Sides,
    ) -> Result<&Outcome, ArenaError> {
        let key = (seed, control.factions(), control.length_s);
        if !self.games.contains_key(&key) {
            let outcome = m.play(seed, control);
            if outcome.seats.len() != control.seats.len() {
                return Err(ArenaError::OutcomeSeatMismatch {
                    expected: control.seats.len(),
                    got: outcome.seats.len(),
                });
            }
            self.games.insert(key.clone(), outcome);
        }
        self.games.get(&key).ok_or(ArenaError::OutcomeSeatMismatch {
            expected: control.seats.len(),
            got: 0,
        })
    }
}

/// Plays every seed of `block` as a pair for one candidate in one scenario.
///
/// The control game comes from `cache` (played on first use). `restrict`
/// tunes one faction in the cross (see [`tuned_sides`]).
pub fn run_pairs<M: Match>(
    m: &mut M,
    cache: &mut ControlCache,
    candidate: &Candidate,
    scenario: Scenario,
    restrict: Option<Faction>,
    block: &SeedBlock,
    length_s: u32,
) -> Result<Vec<PairResult>, ArenaError> {
    let mut results = Vec::new();
    for seed in block.seeds() {
        let sides = tuned_sides(scenario, seed, restrict, length_s, &candidate.overrides)?;
        sides.validate()?;
        let control_sides = sides.control();
        let control_outcome = cache.get_or_play(m, seed, &control_sides)?.clone();
        let tuned_outcome = m.play(seed, &sides);
        if tuned_outcome.seats.len() != sides.seats.len() {
            return Err(ArenaError::OutcomeSeatMismatch {
                expected: sides.seats.len(),
                got: tuned_outcome.seats.len(),
            });
        }
        results.push(PairResult {
            scenario,
            seed,
            tuned: score(&tuned_outcome, sides.scored)?,
            control: score(&control_outcome, sides.scored)?,
        });
    }
    Ok(results)
}

/// A candidate's results over all its pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateSummary {
    /// The candidate's name.
    pub name: String,
    /// The candidate's id (see [`Candidate::id`]).
    pub id: u64,
    /// Games (pairs) played.
    pub n: u64,
    /// Pairs better than control.
    pub better: u64,
    /// Pairs within the threshold of control.
    pub same: u64,
    /// Pairs worse than control.
    pub worse: u64,
    /// Mean delta.
    pub mean_delta: f64,
    /// The Wilson interval on `better / n`.
    pub wilson: (f64, f64),
    /// The sign test over better versus worse.
    pub sign: SignTest,
    /// The tuned decided-win rate.
    pub tuned_win_rate: f64,
    /// The control decided-win rate.
    pub control_win_rate: f64,
    /// Mean tuned waste minus mean control waste (0 when waste scoring is
    /// off).
    pub waste_increase: f64,
    /// Mean delta + `win_weight` x (tuned rate - control rate) - `waste_weight`
    /// x any increase in waste.
    pub fitness: f64,
    /// Mean delta per scenario, with the pair count, in scenario order.
    pub per_scenario: Vec<(Scenario, u64, f64)>,
}

impl CandidateSummary {
    /// Tuned win rate minus control win rate.
    pub fn decided_delta(&self) -> f64 {
        self.tuned_win_rate - self.control_win_rate
    }
}

/// Aggregates one candidate's pairs.
pub fn summarise(name: &str, id: u64, pairs: &[PairResult], cfg: &StatsConfig) -> CandidateSummary {
    let n = pairs.len() as u64;
    let (mut better, mut same, mut worse) = (0u64, 0u64, 0u64);
    let mut delta_sum = 0.0;
    let (mut tuned_wins, mut control_wins) = (0u64, 0u64);
    let (mut tuned_waste, mut control_waste) = (0u64, 0u64);
    let mut by_scenario: BTreeMap<Scenario, (u64, f64)> = BTreeMap::new();
    for p in pairs {
        match p.verdict(cfg) {
            Verdict::Better => better += 1,
            Verdict::Same => same += 1,
            Verdict::Worse => worse += 1,
        }
        delta_sum += p.delta();
        tuned_wins += u64::from(p.tuned.decided_win);
        control_wins += u64::from(p.control.decided_win);
        tuned_waste = tuned_waste.saturating_add(p.tuned.wasted);
        control_waste = control_waste.saturating_add(p.control.wasted);
        let slot = by_scenario.entry(p.scenario).or_insert((0, 0.0));
        slot.0 += 1;
        slot.1 += p.delta();
    }
    let per_n = |x: f64| if n == 0 { 0.0 } else { x / n as f64 };
    let mean_delta = per_n(delta_sum);
    let tuned_win_rate = per_n(tuned_wins as f64);
    let control_win_rate = per_n(control_wins as f64);
    let waste_increase = if cfg.waste_scoring {
        (per_n(tuned_waste as f64) - per_n(control_waste as f64)).max(0.0)
    } else {
        0.0
    };
    CandidateSummary {
        name: name.to_string(),
        id,
        n,
        better,
        same,
        worse,
        mean_delta,
        // Ties count as failures: the interval is over all games.
        wilson: wilson(better, n, cfg.z),
        sign: sign_test(better, worse),
        tuned_win_rate,
        control_win_rate,
        waste_increase,
        fitness: mean_delta + cfg.win_weight * (tuned_win_rate - control_win_rate)
            - cfg.waste_weight * waste_increase,
        per_scenario: by_scenario
            .into_iter()
            .map(|(s, (count, sum))| (s, count, sum / count as f64))
            .collect(),
    }
}

/// Sorts summaries best first: by fitness, then by the Wilson lower bound,
/// then by id so the order is total and repeatable.
pub fn rank(summaries: &mut [CandidateSummary]) {
    summaries.sort_by(|a, b| {
        b.fitness
            .total_cmp(&a.fitness)
            .then(b.wilson.0.total_cmp(&a.wilson.0))
            .then(a.id.cmp(&b.id))
    });
}

/// What to run in [`evaluate`].
#[derive(Debug, Clone)]
pub struct EvalPlan {
    /// The scenarios to play on every seed of the block.
    pub scenarios: Vec<Scenario>,
    /// A faction restriction for the cross, if any.
    pub restrict: Option<Faction>,
    /// The seeds (pairs per scenario).
    pub block: SeedBlock,
    /// The game-time cap in seconds.
    pub length_s: u32,
}

/// Runs every candidate over the plan, sharing control games, and returns the
/// ranked summaries.
pub fn evaluate<M: Match>(
    m: &mut M,
    cache: &mut ControlCache,
    candidates: &[Candidate],
    plan: &EvalPlan,
    cfg: &StatsConfig,
) -> Result<Vec<CandidateSummary>, ArenaError> {
    if plan.length_s == 0 {
        return Err(ArenaError::ZeroLength);
    }
    let mut out = Vec::with_capacity(candidates.len());
    for cand in candidates {
        let mut pairs = Vec::new();
        for scenario in &plan.scenarios {
            pairs.extend(run_pairs(
                m,
                cache,
                cand,
                *scenario,
                plan.restrict,
                &plan.block,
                plan.length_s,
            )?);
        }
        out.push(summarise(&cand.name, cand.id(), &pairs, cfg));
    }
    rank(&mut out);
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A fake game: every seat has `40 + seed % 5` units, plus a bonus per
    /// tuned seat that depends on its overrides. Counts how many games were
    /// played.
    struct Fake {
        games: u32,
        bonus_for: fn(&[(String, String)]) -> i64,
    }

    impl Match for Fake {
        fn play(&mut self, seed: u64, sides: &Sides) -> Outcome {
            self.games += 1;
            let base = 40 + (seed % 5) as i64;
            let seats = sides
                .seats
                .iter()
                .map(|s| {
                    let units = (base + (self.bonus_for)(&s.overrides)).max(0) as u32;
                    SeatOutcome {
                        alive: true,
                        units,
                        wasted: 0,
                    }
                })
                .collect();
            Outcome {
                ended: Ended::Timeout,
                seats,
            }
        }
    }

    fn bonus(ov: &[(String, String)]) -> i64 {
        ov.iter()
            .map(|(k, v)| match k.as_str() {
                "army.wave_size" => v.parse::<i64>().unwrap_or(0) * 5,
                "def.towers" => -20,
                _ => 0,
            })
            .sum()
    }

    fn block(first: u64, count: u64) -> SeedBlock {
        SeedBlock {
            phase: Phase::Evaluate,
            first,
            count,
        }
    }

    fn out(ended: Ended, seats: &[(bool, u32)]) -> Outcome {
        Outcome {
            ended,
            seats: seats
                .iter()
                .map(|&(alive, units)| SeatOutcome {
                    alive,
                    units,
                    wasted: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn margin_scoring() {
        // Two seats: 50 units against 30.
        let s = score(&out(Ended::Timeout, &[(true, 50), (true, 30)]), 0).unwrap();
        assert_eq!((s.margin_num, s.opponents), (20, 1));
        assert!(!s.decided_win);
        // A dead seat counts 0 whatever its unit count says.
        let s = score(&out(Ended::Timeout, &[(false, 50), (true, 30)]), 0).unwrap();
        assert_eq!(s.margin(), -30.0);
        // Four seats: 60 against a mean of (30 + 0 + 45) / 3 = 25.
        let s = score(
            &out(
                Ended::Timeout,
                &[(true, 60), (true, 30), (false, 99), (true, 45)],
            ),
            0,
        )
        .unwrap();
        assert_eq!((s.margin_num, s.opponents), (3 * 60 - 75, 3));
        assert_eq!(s.margin(), 35.0);
    }

    #[test]
    fn decided_win_needs_a_decision_survival_and_dead_opponents() {
        let win = out(Ended::Decided, &[(true, 10), (false, 0)]);
        assert!(score(&win, 0).unwrap().decided_win);
        assert!(!score(&win, 1).unwrap().decided_win);
        let timeout = out(Ended::Timeout, &[(true, 10), (false, 0)]);
        assert!(!score(&timeout, 0).unwrap().decided_win);
        let draw = out(Ended::Decided, &[(false, 0), (false, 0)]);
        assert!(!score(&draw, 0).unwrap().decided_win);
        let alive_opponent = out(Ended::Decided, &[(true, 10), (true, 5), (false, 0)]);
        assert!(!score(&alive_opponent, 0).unwrap().decided_win);
        assert_eq!(
            score(&win, 5),
            Err(ArenaError::ScoredSeatOutOfRange { seat: 5, seats: 2 })
        );
    }

    #[test]
    fn verdict_thresholds_are_exclusive_and_exact() {
        let cfg = StatsConfig::default();
        let pair = |tuned: i64, control: i64, opp: u32| PairResult {
            scenario: Scenario::ArmMirror,
            seed: 1,
            tuned: GameScore {
                margin_num: tuned,
                opponents: opp,
                decided_win: false,
                wasted: 0,
            },
            control: GameScore {
                margin_num: control,
                opponents: opp,
                decided_win: false,
                wasted: 0,
            },
        };
        assert_eq!(pair(11, 0, 1).verdict(&cfg), Verdict::Better);
        assert_eq!(pair(10, 0, 1).verdict(&cfg), Verdict::Same);
        assert_eq!(pair(-10, 0, 1).verdict(&cfg), Verdict::Same);
        assert_eq!(pair(-11, 0, 1).verdict(&cfg), Verdict::Worse);
        // Three opponents: a delta of exactly 10 is 30 in numerator terms.
        assert_eq!(pair(30, 0, 3).verdict(&cfg), Verdict::Same);
        assert_eq!(pair(31, 0, 3).verdict(&cfg), Verdict::Better);
        assert_eq!(pair(31, 0, 3).delta(), 31.0 / 3.0);
    }

    #[test]
    fn control_games_are_played_once_and_shared() {
        let a = Candidate::new("a", &[("army.wave_size", "6")]).unwrap();
        let b = Candidate::new("b", &[("def.towers", "0")]).unwrap();
        let mut m = Fake {
            games: 0,
            bonus_for: bonus,
        };
        let mut cache = ControlCache::new();
        let plan = EvalPlan {
            scenarios: vec![Scenario::ArmMirror, Scenario::CoreMirror],
            restrict: None,
            block: block(101, 4),
            length_s: 600,
        };
        let ranked = evaluate(&mut m, &mut cache, &[a, b], &plan, &StatsConfig::default()).unwrap();
        // 2 candidates x 2 scenarios x 4 seeds tuned games, plus 2 x 4 controls.
        assert_eq!(m.games, 16 + 8);
        assert_eq!(cache.len(), 8);
        // a: +30 for the tuned seat: always better. b: -20: always worse.
        assert_eq!(ranked[0].name, "a");
        assert_eq!((ranked[0].n, ranked[0].better, ranked[0].worse), (8, 8, 0));
        assert_eq!(ranked[1].name, "b");
        assert_eq!((ranked[1].n, ranked[1].better, ranked[1].worse), (8, 0, 8));
        assert_eq!(ranked[0].mean_delta, 30.0);
        assert_eq!(ranked[1].mean_delta, -20.0);
        assert_eq!(ranked[0].per_scenario.len(), 2);
        assert_eq!(ranked[0].sign.exact, Some((2, 8)));
        assert_eq!(ranked[1].sign.exact, Some((2, 8)));
    }

    #[test]
    fn cross_control_does_not_depend_on_who_is_scored() {
        // Both seats are tuned in the cross, so the delta is computed for the
        // parity seat against a control where neither is.
        let a = Candidate::new("a", &[("army.wave_size", "6")]).unwrap();
        let mut m = Fake {
            games: 0,
            bonus_for: bonus,
        };
        let mut cache = ControlCache::new();
        let r = run_pairs(
            &mut m,
            &mut cache,
            &a,
            Scenario::Cross,
            None,
            &block(1, 2),
            600,
        )
        .unwrap();
        assert_eq!(r.len(), 2);
        // Both seats gain 30, so the margin does not move.
        assert!(r.iter().all(|p| p.delta_num() == 0));
    }

    #[test]
    fn ranking_uses_fitness_then_wilson_lower_bound() {
        let mk = |name: &str, id, fitness, lo| CandidateSummary {
            name: name.into(),
            id,
            n: 1,
            better: 1,
            same: 0,
            worse: 0,
            mean_delta: 0.0,
            wilson: (lo, 1.0),
            sign: sign_test(1, 0),
            tuned_win_rate: 0.0,
            control_win_rate: 0.0,
            waste_increase: 0.0,
            fitness,
            per_scenario: vec![],
        };
        let mut v = vec![
            mk("c", 3, 5.0, 0.4),
            mk("a", 1, 9.0, 0.1),
            mk("b", 2, 5.0, 0.6),
        ];
        rank(&mut v);
        assert_eq!(
            v.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn fitness_combines_delta_wins_and_waste() {
        let g = |margin, win, wasted| GameScore {
            margin_num: margin,
            opponents: 1,
            decided_win: win,
            wasted,
        };
        let pairs = [
            PairResult {
                scenario: Scenario::ArmMirror,
                seed: 1,
                tuned: g(30, true, 500),
                control: g(0, false, 100),
            },
            PairResult {
                scenario: Scenario::ArmMirror,
                seed: 2,
                tuned: g(10, false, 300),
                control: g(10, false, 100),
            },
        ];
        let off = summarise("x", 1, &pairs, &StatsConfig::default());
        // Mean delta 15, win rates 0.5 vs 0: 15 + 20 * 0.5 = 25.
        assert_eq!((off.better, off.same, off.worse), (1, 1, 0));
        assert_eq!(off.mean_delta, 15.0);
        assert_eq!(off.decided_delta(), 0.5);
        assert_eq!(off.fitness, 25.0);
        // Waste on: mean 400 vs 100, an increase of 300, costing 90.
        let cfg = StatsConfig {
            waste_scoring: true,
            ..StatsConfig::default()
        };
        let on = summarise("x", 1, &pairs, &cfg);
        assert_eq!(on.waste_increase, 300.0);
        assert!((on.fitness - (25.0 - 90.0)).abs() < 1e-9);
        // Ties count as failures in the Wilson interval: 1 better of 2.
        assert_eq!(on.wilson, wilson(1, 2, DEFAULT_Z));
        // No pairs at all: everything is zero and the interval is [0, 1].
        let empty = summarise("e", 2, &[], &StatsConfig::default());
        assert_eq!((empty.n, empty.fitness, empty.wilson), (0, 0.0, (0.0, 1.0)));
        assert_eq!(empty.sign.p, 1.0);
    }

    #[test]
    fn candidates_are_validated_and_ids_are_canonical() {
        assert!(matches!(
            Candidate::new("x", &[("army.nope", "1")]),
            Err(ArenaError::BadOverride { .. })
        ));
        assert!(matches!(
            Candidate::new("x", &[("army.wave_size", "many")]),
            Err(ArenaError::BadOverride { .. })
        ));
        let a = Candidate::new(
            "a",
            &[("army.wave_size", "6"), ("cheat.income_factor", "1.250")],
        )
        .unwrap();
        let b = Candidate::new(
            "b",
            &[("cheat.income_factor", "1.25"), ("army.wave_size", "6")],
        )
        .unwrap();
        let c = Candidate::new(
            "c",
            &[("army.wave_size", "7"), ("cheat.income_factor", "1.25")],
        )
        .unwrap();
        assert_eq!(a.id(), b.id(), "order and spelling do not matter");
        assert_ne!(a.id(), c.id());
        // The id is pinned: it is part of the results-log key.
        assert_eq!(
            Candidate::new("e", &[]).unwrap().id(),
            0xcbf2_9ce4_8422_2325
        );
    }

    #[test]
    fn mismatched_outcomes_are_errors() {
        struct Bad;
        impl Match for Bad {
            fn play(&mut self, _seed: u64, _sides: &Sides) -> Outcome {
                Outcome {
                    ended: Ended::Timeout,
                    seats: vec![],
                }
            }
        }
        let a = Candidate::new("a", &[]).unwrap();
        let r = run_pairs(
            &mut Bad,
            &mut ControlCache::new(),
            &a,
            Scenario::ArmMirror,
            None,
            &block(1, 1),
            60,
        );
        assert_eq!(
            r,
            Err(ArenaError::OutcomeSeatMismatch {
                expected: 2,
                got: 0
            })
        );
        let r = run_pairs(
            &mut Bad,
            &mut ControlCache::new(),
            &a,
            Scenario::ArmMirror,
            None,
            &block(1, 1),
            0,
        );
        assert_eq!(r, Err(ArenaError::ZeroLength));
        assert_eq!(StatsConfig::with_z(f64::NAN), Err(ArenaError::BadZ));
        assert_eq!(StatsConfig::with_z(1.645).map(|c| c.z), Ok(1.645));
    }
}
