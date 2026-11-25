//! Seed schedule and side swapping (SPEC.md section 8.2).
//!
//! Phases draw their seeds from **disjoint consecutive blocks** that start at
//! a first seed. Within a block, each seed is one pair: a control game with no
//! overrides and a tuned game on the same seed. Which seat is tuned follows
//! the seed's parity, so side advantages cancel over a block.

use super::{ArenaError, Sides};
use crate::personality::Faction;

/// A phase of a tuning run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Random candidates on short games.
    Screening,
    /// The best few on fresh seeds and longer games.
    Confirmation,
    /// A four-player check (reported, does not gate).
    FourPlayer,
    /// Fixed named candidates on seeds never used before.
    Evaluate,
}

impl Phase {
    /// The display name.
    pub fn name(self) -> &'static str {
        match self {
            Phase::Screening => "screening",
            Phase::Confirmation => "confirmation",
            Phase::FourPlayer => "four-player",
            Phase::Evaluate => "evaluate",
        }
    }
}

/// A run of consecutive seeds, `first..first + count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedBlock {
    /// The phase the block belongs to.
    pub phase: Phase,
    /// The first seed.
    pub first: u64,
    /// How many seeds (pairs).
    pub count: u64,
}

impl SeedBlock {
    /// The seeds of the block, in order.
    pub fn seeds(&self) -> impl Iterator<Item = u64> {
        self.first..self.first.saturating_add(self.count)
    }

    /// One past the last seed.
    pub fn end(&self) -> u64 {
        self.first.saturating_add(self.count)
    }
}

/// Hands out disjoint, consecutive seed blocks.
#[derive(Debug, Clone)]
pub struct SeedSchedule {
    next: u64,
    blocks: Vec<SeedBlock>,
}

impl SeedSchedule {
    /// A schedule whose first block starts at `first_seed`.
    pub fn new(first_seed: u64) -> SeedSchedule {
        SeedSchedule {
            next: first_seed,
            blocks: Vec::new(),
        }
    }

    /// Takes the next `count` seeds for `phase`. The block starts right after
    /// the previous one, so no seed is ever handed out twice.
    pub fn allocate(&mut self, phase: Phase, count: u64) -> Result<SeedBlock, ArenaError> {
        let end = self
            .next
            .checked_add(count)
            .ok_or(ArenaError::SeedOverflow)?;
        let block = SeedBlock {
            phase,
            first: self.next,
            count,
        };
        self.next = end;
        self.blocks.push(block);
        Ok(block)
    }

    /// Every block handed out so far, in order.
    pub fn blocks(&self) -> &[SeedBlock] {
        &self.blocks
    }

    /// The seed the next block would start at.
    pub fn next_seed(&self) -> u64 {
        self.next
    }
}

/// A game scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scenario {
    /// ARM against ARM.
    ArmMirror,
    /// CORE against CORE.
    CoreMirror,
    /// ARM (seat 0) against CORE (seat 1).
    Cross,
    /// Four players on a four-player map: ARM, CORE, ARM, CORE.
    FourPlayer,
}

impl Scenario {
    /// The three two-player scenarios of the tuning phases.
    pub const TWO_PLAYER: [Scenario; 3] =
        [Scenario::ArmMirror, Scenario::CoreMirror, Scenario::Cross];

    /// The display name.
    pub fn name(self) -> &'static str {
        match self {
            Scenario::ArmMirror => "ARM mirror",
            Scenario::CoreMirror => "CORE mirror",
            Scenario::Cross => "ARM-vs-CORE cross",
            Scenario::FourPlayer => "four-player",
        }
    }

    /// The factions by seat, before any side swapping.
    pub fn factions(self) -> Vec<Faction> {
        match self {
            Scenario::ArmMirror => vec![Faction::Arm; 2],
            Scenario::CoreMirror => vec![Faction::Core; 2],
            Scenario::Cross => vec![Faction::Arm, Faction::Core],
            Scenario::FourPlayer => vec![Faction::Arm, Faction::Core, Faction::Arm, Faction::Core],
        }
    }
}

/// The seat scored for a seed: seat 0 on odd seeds and seat 1 on even seeds
/// in two-player scenarios. In the four-player check (where the spec gives no
/// rule) the seat rotates with `seed mod 4`.
pub fn scored_seat(scenario: Scenario, seed: u64) -> usize {
    match scenario {
        Scenario::FourPlayer => (seed % 4) as usize,
        _ => usize::from(seed.is_multiple_of(2)),
    }
}

/// Builds the tuned game's sides for one seed.
///
/// - **Mirrors and four-player:** only the scored seat gets `overrides`.
/// - **Cross:** both seats get them and the scored seat is the parity seat.
/// - **Cross restricted to a faction** (`restrict`): the cross sides alternate
///   with the seed's parity so that the faction sits in the scored seat, and
///   only that seat is tuned.
///
/// A restriction on any other scenario is an error.
pub fn tuned_sides(
    scenario: Scenario,
    seed: u64,
    restrict: Option<Faction>,
    length_s: u32,
    overrides: &[(String, String)],
) -> Result<Sides, ArenaError> {
    let scored = scored_seat(scenario, seed);
    let mut factions = scenario.factions();
    let mut tuned = vec![false; factions.len()];
    match (scenario, restrict) {
        (Scenario::Cross, None) => tuned.fill(true),
        (Scenario::Cross, Some(f)) => {
            let other = match f {
                Faction::Arm => Faction::Core,
                Faction::Core => Faction::Arm,
            };
            factions = vec![other; 2];
            if let Some(slot) = factions.get_mut(scored) {
                *slot = f;
            }
            if let Some(t) = tuned.get_mut(scored) {
                *t = true;
            }
        }
        (_, None) => {
            if let Some(t) = tuned.get_mut(scored) {
                *t = true;
            }
        }
        (_, Some(_)) => return Err(ArenaError::RestrictionNeedsCross),
    }
    Ok(Sides::new(factions, &tuned, scored, length_s, overrides))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn blocks_are_consecutive_and_disjoint() {
        let mut s = SeedSchedule::new(101);
        let a = s.allocate(Phase::Screening, 16).unwrap();
        let b = s.allocate(Phase::Confirmation, 8).unwrap();
        let c = s.allocate(Phase::Evaluate, 32).unwrap();
        assert_eq!((a.first, a.end()), (101, 117));
        assert_eq!((b.first, b.end()), (117, 125));
        assert_eq!((c.first, c.end()), (125, 157));
        let mut seen = BTreeSet::new();
        for blk in s.blocks() {
            for seed in blk.seeds() {
                assert!(seen.insert(seed), "seed {seed} reused");
            }
        }
        assert_eq!(seen.len(), 56);
        assert_eq!(s.next_seed(), 157);
        assert_eq!(
            s.allocate(Phase::Evaluate, u64::MAX),
            Err(ArenaError::SeedOverflow)
        );
        assert_eq!(s.blocks().len(), 3, "a failed allocation takes nothing");
    }

    #[test]
    fn mirror_candidate_takes_seat_0_on_odd_seeds() {
        for seed in 100..110u64 {
            let sides = tuned_sides(
                Scenario::ArmMirror,
                seed,
                None,
                600,
                &[("def.towers".into(), "0".into())],
            )
            .unwrap();
            let want = usize::from(seed.is_multiple_of(2));
            assert_eq!(sides.scored, want, "seed {seed}");
            assert!(sides.seats[want].tuned);
            assert!(!sides.seats[1 - want].tuned);
            assert!(sides.seats[1 - want].overrides.is_empty());
            assert_eq!(sides.seats[want].overrides.len(), 1);
        }
        assert_eq!(scored_seat(Scenario::ArmMirror, 101), 0);
        assert_eq!(scored_seat(Scenario::CoreMirror, 102), 1);
    }

    #[test]
    fn cross_tunes_both_seats_and_scores_the_parity_seat() {
        let ov = [("def.towers".to_string(), "0".to_string())];
        let odd = tuned_sides(Scenario::Cross, 7, None, 600, &ov).unwrap();
        let even = tuned_sides(Scenario::Cross, 8, None, 600, &ov).unwrap();
        for s in [&odd, &even] {
            assert!(s.seats.iter().all(|x| x.tuned && x.overrides == ov));
            assert_eq!(
                s.seats.iter().map(|x| x.faction).collect::<Vec<_>>(),
                [Faction::Arm, Faction::Core]
            );
        }
        assert_eq!((odd.scored, even.scored), (0, 1));
    }

    #[test]
    fn restricted_cross_alternates_sides_and_tunes_one_seat() {
        let ov = [("def.towers".to_string(), "0".to_string())];
        for restrict in [Faction::Arm, Faction::Core] {
            for seed in 1..=6u64 {
                let s = tuned_sides(Scenario::Cross, seed, Some(restrict), 600, &ov).unwrap();
                assert_eq!(s.seats[s.scored].faction, restrict);
                assert!(s.seats[s.scored].tuned);
                assert_eq!(s.seats.iter().filter(|x| x.tuned).count(), 1);
                assert_ne!(s.seats[1 - s.scored].faction, restrict);
            }
        }
        // ARM sits in seat 0 on odd seeds and seat 1 on even seeds.
        let a = tuned_sides(Scenario::Cross, 1, Some(Faction::Arm), 1, &[]).unwrap();
        let b = tuned_sides(Scenario::Cross, 2, Some(Faction::Arm), 1, &[]).unwrap();
        assert_eq!((a.scored, b.scored), (0, 1));
        assert_eq!(
            tuned_sides(Scenario::ArmMirror, 1, Some(Faction::Arm), 1, &[]),
            Err(ArenaError::RestrictionNeedsCross)
        );
    }

    #[test]
    fn four_player_is_arm_core_arm_core() {
        let s = tuned_sides(Scenario::FourPlayer, 6, None, 900, &[]).unwrap();
        assert_eq!(s.seats.len(), 4);
        assert_eq!(s.scored, 2);
        assert_eq!(
            s.seats.iter().map(|x| x.faction).collect::<Vec<_>>(),
            [Faction::Arm, Faction::Core, Faction::Arm, Faction::Core]
        );
        assert_eq!(s.seats.iter().filter(|x| x.tuned).count(), 1);
    }
}
