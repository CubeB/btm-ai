//! Which manager owns which unit, and the hand-over rules (SPEC §3.1, §3.2).
//!
//! The planner, army and air are not peers that negotiate (SPEC §3.1). Each
//! own unit therefore has exactly one **default owner**, fixed by its class
//! ([`default_owner`]), and the dispatcher drops every staged order from a
//! manager that does not own the unit. The rules:
//!
//! 1. **Default owner by class.** Factories, builders (the commander apart)
//!    and other structures belong to the economy; combat units and mobile
//!    anti-air to the army; hulls to the navy; scouts to the scouts; armed
//!    aircraft to the air pass; transports to the transport pass.
//! 2. **The commander is shared.** Both the economy (building, §4) and the
//!    army (defending alone, d-gun, fleeing, §5.4, §5.7) order it. When both
//!    stage orders in one tick, the higher [`super::Priority`] wins, ties to
//!    the earlier pipeline step, so the army does not overwrite a planner job
//!    unless it outranks it (SPEC §3.1 "commander tasked").
//! 3. **Loans.** A manager may borrow a unit with [`Ownership::lend`] only
//!    along a hand-over listed in [`HANDOVERS`], and only while the unit is
//!    with its default owner. From the next dispatch on, orders to it are
//!    accepted only from the borrower. The borrower ends the loan with
//!    [`Ownership::give_back`]; the default owner may [`Ownership::recall`]
//!    it at any time. A loan also ends when the unit dies, or when its class
//!    changes so that its default owner is no longer the lender.
//! 4. **Held units.** Units held by a mission script, and unfinished frames,
//!    are counted but never ordered: every order to them is dropped.
//!
//! Loans are kept in a `BTreeMap`, so every walk is in unit id order (§7).

use std::collections::BTreeMap;
use std::fmt;

use super::own::UnitClass;
use crate::view::{Tick, UnitId};

/// A manager slot: the source of a staged order and the owner of a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// Economy and build (SPEC §4): metal makers and the build planner.
    Economy,
    /// Army (SPEC §5.1-5.7, §5.11-5.13): the phase machine, waves, defence,
    /// builder safety and per-unit micro.
    Army,
    /// Navy (SPEC §5.12 "Navy"): hulls.
    Navy,
    /// Scouting (SPEC §5.10).
    Scouts,
    /// Transports and ferries (SPEC §5.9).
    Transport,
    /// Air (SPEC §5.8): armed aircraft.
    Air,
}

impl Source {
    /// Every slot.
    pub const ALL: [Source; 6] = [
        Source::Economy,
        Source::Army,
        Source::Navy,
        Source::Scouts,
        Source::Transport,
        Source::Air,
    ];

    /// The micro limiter applies to the army and air passes (SPEC §2.3). The
    /// navy is part of the army pass (SPEC §3.1 step 10).
    pub fn is_rate_limited(self) -> bool {
        matches!(self, Source::Army | Source::Navy | Source::Air)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Who may order a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Owner {
    /// One manager.
    One(Source),
    /// The commander: economy and army, resolved by priority.
    Shared,
    /// Nobody: scripted units and frames.
    Held,
}

/// Why a unit was lent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LoanReason {
    /// The combat unit borrowed as the scout when there is no scout (§5.10).
    BorrowedScout,
    /// The hull borrowed as the naval scout (`scout.naval`, §5.10).
    NavalScout,
    /// Booked as a ferry passenger (§5.9); other passes leave it alone.
    FerryPassenger,
}

/// The hand-overs a loan may make: (lender, borrower, reason).
pub const HANDOVERS: &[(Source, Source, LoanReason)] = &[
    (Source::Army, Source::Scouts, LoanReason::BorrowedScout),
    (Source::Navy, Source::Scouts, LoanReason::NavalScout),
    (Source::Army, Source::Transport, LoanReason::FerryPassenger),
    (
        Source::Economy,
        Source::Transport,
        LoanReason::FerryPassenger,
    ),
];

/// An active loan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loan {
    pub from: Source,
    pub to: Source,
    pub reason: LoanReason,
    pub since: Tick,
}

/// Why a hand-over was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnershipError {
    /// Not one of our units this tick.
    UnknownUnit(UnitId),
    /// The unit is held, shared, or not with the named lender.
    NotLender { unit: UnitId, lender: Source },
    /// The unit is already on loan.
    AlreadyLent(UnitId),
    /// The (lender, borrower, reason) triple is not in [`HANDOVERS`].
    NotAllowed {
        from: Source,
        to: Source,
        reason: LoanReason,
    },
    /// Only the borrower or the lender may end a loan.
    NotParty { unit: UnitId, by: Source },
    /// The unit is not on loan.
    NotLent(UnitId),
}

impl fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnershipError::UnknownUnit(u) => write!(f, "unit {} is not ours", u.0),
            OwnershipError::NotLender { unit, lender } => {
                write!(f, "unit {} is not with {lender:?}", unit.0)
            }
            OwnershipError::AlreadyLent(u) => write!(f, "unit {} is already lent", u.0),
            OwnershipError::NotAllowed { from, to, reason } => {
                write!(f, "no hand-over {from:?} -> {to:?} for {reason:?}")
            }
            OwnershipError::NotParty { unit, by } => {
                write!(f, "{by:?} is not a party to the loan of unit {}", unit.0)
            }
            OwnershipError::NotLent(u) => write!(f, "unit {} is not lent", u.0),
        }
    }
}

impl std::error::Error for OwnershipError {}

/// The default owner of a unit of this class (rule 1).
pub fn default_owner(class: UnitClass, commander: bool, held: bool) -> Owner {
    if held {
        return Owner::Held;
    }
    if commander {
        return Owner::Shared;
    }
    Owner::One(match class {
        UnitClass::Factory | UnitClass::Builder | UnitClass::Structure | UnitClass::Other => {
            Source::Economy
        }
        UnitClass::Combat | UnitClass::AntiAir => Source::Army,
        UnitClass::Hull => Source::Navy,
        UnitClass::Scout => Source::Scouts,
        UnitClass::Transport => Source::Transport,
        UnitClass::Aircraft => Source::Air,
    })
}

/// Unit ownership for the seat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ownership {
    defaults: BTreeMap<UnitId, Owner>,
    loans: BTreeMap<UnitId, Loan>,
    tick: Tick,
}

impl Ownership {
    /// Refreshes default owners from this tick's classification and ends
    /// loans whose unit died or changed hands (rule 3). `units` is
    /// `(id, default owner)` in id order.
    pub(crate) fn sync(&mut self, units: &[(UnitId, Owner)], tick: Tick) {
        self.tick = tick;
        self.defaults = units.iter().copied().collect();
        let defaults = &self.defaults;
        self.loans
            .retain(|id, loan| defaults.get(id) == Some(&Owner::One(loan.from)));
    }

    /// Who may order `unit` now.
    pub fn owner(&self, unit: UnitId) -> Option<Owner> {
        if let Some(l) = self.loans.get(&unit) {
            return Some(Owner::One(l.to));
        }
        self.defaults.get(&unit).copied()
    }

    /// Whether `source` may order `unit` (shared units: economy or army).
    pub fn may_order(&self, unit: UnitId, source: Source) -> bool {
        match self.owner(unit) {
            Some(Owner::One(s)) => s == source,
            Some(Owner::Shared) => matches!(source, Source::Economy | Source::Army),
            Some(Owner::Held) | None => false,
        }
    }

    /// Whether `unit` is with `source`, counting loans (for building lists).
    pub fn is_with(&self, unit: UnitId, source: Source) -> bool {
        self.owner(unit) == Some(Owner::One(source))
    }

    /// The active loan of `unit`, if any.
    pub fn loan(&self, unit: UnitId) -> Option<&Loan> {
        self.loans.get(&unit)
    }

    /// Every loan, in unit id order.
    pub fn loans(&self) -> impl Iterator<Item = (UnitId, &Loan)> + '_ {
        self.loans.iter().map(|(&u, l)| (u, l))
    }

    /// Units lent for `reason`, in id order.
    pub fn lent_for(&self, reason: LoanReason) -> impl Iterator<Item = UnitId> + '_ {
        self.loans
            .iter()
            .filter(move |(_, l)| l.reason == reason)
            .map(|(&u, _)| u)
    }

    /// Lends `unit` from `from` to `to` (rule 3).
    pub fn lend(
        &mut self,
        unit: UnitId,
        from: Source,
        to: Source,
        reason: LoanReason,
    ) -> Result<(), OwnershipError> {
        let Some(&default) = self.defaults.get(&unit) else {
            return Err(OwnershipError::UnknownUnit(unit));
        };
        if !HANDOVERS.contains(&(from, to, reason)) {
            return Err(OwnershipError::NotAllowed { from, to, reason });
        }
        if self.loans.contains_key(&unit) {
            return Err(OwnershipError::AlreadyLent(unit));
        }
        if default != Owner::One(from) {
            return Err(OwnershipError::NotLender { unit, lender: from });
        }
        self.loans.insert(
            unit,
            Loan {
                from,
                to,
                reason,
                since: self.tick,
            },
        );
        Ok(())
    }

    /// The borrower returns `unit` to its default owner.
    pub fn give_back(&mut self, unit: UnitId, by: Source) -> Result<(), OwnershipError> {
        self.end(unit, by, |l| l.to)
    }

    /// The default owner takes `unit` back.
    pub fn recall(&mut self, unit: UnitId, by: Source) -> Result<(), OwnershipError> {
        self.end(unit, by, |l| l.from)
    }

    fn end(
        &mut self,
        unit: UnitId,
        by: Source,
        party: fn(&Loan) -> Source,
    ) -> Result<(), OwnershipError> {
        match self.loans.get(&unit) {
            None => Err(OwnershipError::NotLent(unit)),
            Some(l) if party(l) != by => Err(OwnershipError::NotParty { unit, by }),
            Some(_) => {
                self.loans.remove(&unit);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn loans_follow_the_handover_rules() {
        let mut o = Ownership::default();
        let (a, b, c) = (UnitId(1), UnitId(2), UnitId(3));
        o.sync(
            &[
                (a, Owner::One(Source::Army)),
                (b, Owner::Shared),
                (c, Owner::Held),
            ],
            5,
        );
        assert!(o.may_order(a, Source::Army));
        assert!(o.may_order(b, Source::Economy) && o.may_order(b, Source::Army));
        assert!(!o.may_order(b, Source::Air));
        assert!(!o.may_order(c, Source::Economy));
        assert_eq!(
            o.lend(a, Source::Army, Source::Air, LoanReason::BorrowedScout),
            Err(OwnershipError::NotAllowed {
                from: Source::Army,
                to: Source::Air,
                reason: LoanReason::BorrowedScout
            })
        );
        assert_eq!(
            o.lend(b, Source::Army, Source::Scouts, LoanReason::BorrowedScout),
            Err(OwnershipError::NotLender {
                unit: b,
                lender: Source::Army
            })
        );
        o.lend(a, Source::Army, Source::Scouts, LoanReason::BorrowedScout)
            .unwrap();
        assert!(o.may_order(a, Source::Scouts) && !o.may_order(a, Source::Army));
        assert_eq!(o.loan(a).map(|l| l.since), Some(5));
        assert_eq!(
            o.give_back(a, Source::Army),
            Err(OwnershipError::NotParty {
                unit: a,
                by: Source::Army
            })
        );
        o.recall(a, Source::Army).unwrap();
        assert!(o.may_order(a, Source::Army));
        // A loan ends when the unit dies.
        o.lend(
            a,
            Source::Army,
            Source::Transport,
            LoanReason::FerryPassenger,
        )
        .unwrap();
        o.sync(&[(b, Owner::Shared)], 6);
        assert!(o.loan(a).is_none());
    }
}
