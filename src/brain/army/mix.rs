//! The counter lean (SPEC §5.3): which of our unit types answer the enemy
//! mix, published as share points in `world.army.lean` for factory
//! production (§4.4).
//!
//! The role histogram itself is kept by the core (`world.enemy.roles`). The
//! choice is a pure function of the histogram, the static/mobile metal split
//! and the knobs ([`choose`]), so it is unit-tested on its own.

use super::{at_least, secs, ArmyManager};
use crate::brain::contacts::ContactRole;
use crate::brain::roles::Role;
use crate::brain::Ctx;
use crate::knobs::Fixed;
use crate::view::{Amount, DefId, Tick};

/// The three answers per factory, in table order (SPEC §5.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lean {
    /// Kbot lab: rocket kbot, artillery kbot, raider kbot.
    pub lab: [i32; 3],
    /// Vehicle plant: missile truck, medium tank, light tank.
    pub plant: [i32; 3],
}

/// Answer columns: 0 against air or raiders, 1 against static defence or
/// tanks, 2 against rockets/artillery or builders.
pub const ANSWER_ROLES: [(Role, Role); 3] = [
    (Role::RocketKbot, Role::MissileTruck),
    (Role::ArtilleryKbot, Role::MediumTank),
    (Role::RaiderKbot, Role::LightTank),
];

/// The answer column for an enemy role, if it has one.
pub fn answer_of(role: ContactRole) -> Option<usize> {
    match role {
        ContactRole::Air | ContactRole::Raider => Some(0),
        ContactRole::StaticDefence | ContactRole::Tank => Some(1),
        ContactRole::RocketArtillery | ContactRole::Builder => Some(2),
        ContactRole::Other => None,
    }
}

/// The triggered lean: each answer whose enemy role has count > 0 and a
/// share of the histogram ≥ `trigger` gets exactly `lean` points (0 or
/// `lean`, never more).
pub fn triggered(roles: &[u32; 7], trigger: Fixed, lean: i32) -> Lean {
    let total: i64 = roles.iter().map(|&r| i64::from(r)).sum();
    let mut out = Lean::default();
    for role in ContactRole::ALL {
        let n = i64::from(roles.get(role.index()).copied().unwrap_or(0));
        if n == 0 || !at_least(n, trigger, total) {
            continue;
        }
        if let Some(col) = answer_of(role) {
            if let (Some(a), Some(b)) = (out.lab.get_mut(col), out.plant.get_mut(col)) {
                *a = lean;
                *b = lean;
            }
        }
    }
    out
}

/// The static/mobile rule (kbot lab only, every pass): artillery gets
/// `bonus` if enemy armed ground buildings are ≥ `trigger` of the armed
/// ground metal, rocket kbots get `bonus` if mobile units are.
pub fn static_mobile(statics: Amount, mobiles: Amount, trigger: Fixed, bonus: i32) -> [i32; 3] {
    let total = (statics + mobiles).milli();
    let mut out = [0; 3];
    if total <= 0 {
        return out;
    }
    if at_least(statics.milli(), trigger, total) {
        out[1] = bonus;
    }
    if at_least(mobiles.milli(), trigger, total) {
        out[0] = bonus;
    }
    out
}

/// The whole choice: the held triggered lean plus this pass's static/mobile
/// bonus.
pub fn choose(held: Lean, statics: Amount, mobiles: Amount, trigger: Fixed, bonus: i32) -> Lean {
    let sm = static_mobile(statics, mobiles, trigger, bonus);
    let mut out = held;
    for (a, b) in out.lab.iter_mut().zip(sm) {
        *a += b;
    }
    out
}

/// What the lean pass remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MixState {
    /// The triggered lean, held between recomputations.
    pub(crate) held: Lean,
    /// When it was last recomputed.
    pub(crate) at: Option<Tick>,
}

impl ArmyManager {
    /// Recomputes the lean (at most every `mix.commit_s`) and publishes it.
    pub(crate) fn counter_lean(&mut self, ctx: &mut Ctx<'_>) {
        let k = &ctx.knobs.mix;
        if !k.counter {
            ctx.world.army.lean.clear();
            return;
        }
        let due = self
            .mix
            .at
            .is_none_or(|t| ctx.tick.saturating_sub(t) >= secs(k.commit_s));
        if due {
            self.mix.held = triggered(&ctx.world.enemy.roles, k.counter_trigger, k.lean);
            self.mix.at = Some(ctx.tick);
        }
        let (mut statics, mut mobiles) = (Amount::ZERO, Amount::ZERO);
        for c in ctx.world.contacts.iter().filter(|c| c.armed && !c.air) {
            if c.building {
                statics += c.metal;
            } else {
                mobiles += c.metal;
            }
        }
        let lean = choose(
            self.mix.held,
            statics,
            mobiles,
            k.counter_trigger,
            k.counter_bonus,
        );
        let roles = &ctx.world.roles;
        let mut map = std::collections::BTreeMap::new();
        for (col, &(lab, plant)) in ANSWER_ROLES.iter().enumerate() {
            for (role, pts) in [
                (lab, lean.lab.get(col).copied().unwrap_or(0)),
                (plant, lean.plant.get(col).copied().unwrap_or(0)),
            ] {
                if let (Some(def), true) = (roles.get(role), pts != 0) {
                    let e: &mut i32 = map.entry(def).or_insert(0);
                    *e += pts;
                }
            }
        }
        ctx.world.army.lean = map;
    }
}

/// The lean for one definition (0 when none).
pub fn lean_for(lean: &std::collections::BTreeMap<DefId, i32>, def: DefId) -> i32 {
    lean.get(&def).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(pairs: &[(ContactRole, u32)]) -> [u32; 7] {
        let mut h = [0u32; 7];
        for &(r, n) in pairs {
            h[r.index()] = n;
        }
        h
    }

    const TRIGGER: Fixed = Fixed::from_raw(3000);

    #[test]
    fn the_counter_follows_the_enemy_mix() {
        // Raiders dominate: rocket kbots and missile trucks.
        let h = hist(&[(ContactRole::Raider, 6), (ContactRole::Other, 4)]);
        let l = triggered(&h, TRIGGER, 2);
        assert_eq!(l.lab, [2, 0, 0]);
        assert_eq!(l.plant, [2, 0, 0]);

        // The enemy switches to tanks and towers: artillery and medium tanks.
        let h = hist(&[
            (ContactRole::Tank, 3),
            (ContactRole::StaticDefence, 3),
            (ContactRole::Raider, 1),
            (ContactRole::Other, 3),
        ]);
        let l = triggered(&h, TRIGGER, 2);
        assert_eq!(l.lab, [0, 2, 0], "both roles answer the same column once");
        assert_eq!(l.plant, [0, 2, 0]);

        // Rockets and builders: raider kbots and light tanks.
        let h = hist(&[(ContactRole::RocketArtillery, 2), (ContactRole::Builder, 1)]);
        assert_eq!(triggered(&h, TRIGGER, 2).lab, [0, 0, 2]);

        // Below the trigger share nothing leans; 3 of 10 is exactly 0.3.
        let h = hist(&[(ContactRole::Air, 2), (ContactRole::Other, 8)]);
        assert_eq!(triggered(&h, TRIGGER, 2), Lean::default());
        let h = hist(&[(ContactRole::Air, 3), (ContactRole::Other, 7)]);
        assert_eq!(triggered(&h, TRIGGER, 2).lab, [2, 0, 0]);
        assert_eq!(triggered(&[0; 7], TRIGGER, 2), Lean::default());
    }

    #[test]
    fn the_static_mobile_rule_adds_up_to_four() {
        let held = Lean {
            lab: [2, 2, 0],
            plant: [2, 2, 0],
        };
        // All metal in towers: artillery +2.
        let l = choose(held, Amount::units(500), Amount::ZERO, TRIGGER, 2);
        assert_eq!(l.lab, [2, 4, 0]);
        assert_eq!(l.plant, [2, 2, 0], "the vehicle plant has no such rule");
        // A mix over the trigger both ways: both get it.
        let l = choose(held, Amount::units(400), Amount::units(600), TRIGGER, 2);
        assert_eq!(l.lab, [4, 4, 0]);
        // Mobile only: rocket kbots.
        let l = choose(Lean::default(), Amount::ZERO, Amount::units(1), TRIGGER, 2);
        assert_eq!(l.lab, [2, 0, 0]);
        // Nothing known: nothing.
        let l = choose(Lean::default(), Amount::ZERO, Amount::ZERO, TRIGGER, 2);
        assert_eq!(l, Lean::default());
    }
}
