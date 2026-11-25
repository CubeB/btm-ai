//! Metal makers (SPEC §4.9): the per-tick on/off switch, the experimental
//! budget mode and the maker ceiling.

use super::super::{Ctx, Staging};
use super::facts::Facts;
use crate::knobs::Knobs;
use crate::view::{Command, Orders, UnitId, TICKS_PER_SECOND};

/// Maker state across passes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Makers {
    /// Consecutive planning passes with metal short and energy rich.
    starved_passes: i32,
}

/// One extra maker runs in budget mode at this energy fill, permille.
const BUDGET_EXTRA_FILL: i64 = 950;

impl Makers {
    /// Counts the starved passes; call once per planning pass.
    pub fn plan_pass(&mut self, f: &Facts) {
        if f.metal_short && f.energy_rich {
            self.starved_passes = self.starved_passes.saturating_add(1);
        } else {
            self.starved_passes = 0;
        }
    }

    /// The maker ceiling (SPEC §4.9).
    pub fn ceiling(&self, k: &Knobs, f: &Facts) -> i32 {
        let mut c = k.eco.makers;
        if self.starved_passes >= k.eco.makers_starved_passes {
            c = c.max(k.eco.makers_starved);
        }
        if k.eco.makers_surplus_max > 0 && f.energy_surplus > super::list::SURPLUS_MAKER_ENERGY {
            c = c.max(k.eco.makers_surplus_max);
        }
        c
    }

    /// Switches makers, every tick (SPEC §4.9). The dispatcher lets through
    /// at most one command per maker per second and drops one that matches
    /// the maker's state.
    pub fn switch(&mut self, ctx: &Ctx<'_>, out: &mut Staging) {
        let w = &*ctx.world;
        let k = ctx.knobs;
        let makers: Vec<(UnitId, bool, i64, i64)> = w
            .own
            .units
            .iter()
            .filter(|s| s.building && !s.is_frame() && !s.unit.scripted)
            .filter_map(|s| {
                let d = ctx.def(s.unit.def)?;
                (d.on_off && d.metal_make.milli() > 0).then_some((
                    s.id(),
                    s.unit.on,
                    d.metal_make.milli(),
                    d.energy_use.milli(),
                ))
            })
            .collect();
        if makers.is_empty() {
            return;
        }
        let e = &w.economy.raw.energy;
        let fill = e.fill_permille();
        let off = w.threat.base_under_attack()
            || e.stalled
            || fill < i64::from(k.eco.maker_off_below_pct) * 10;
        if k.eco.maker_budget && !off {
            if !ctx.tick.is_multiple_of(TICKS_PER_SECOND) {
                return;
            }
            // Rank by metal per energy (cross-multiplied), then id.
            let mut ranked = makers.clone();
            ranked.sort_by(|a, b| {
                (i128::from(b.2) * i128::from(a.3))
                    .cmp(&(i128::from(a.2) * i128::from(b.3)))
                    .then(a.0.cmp(&b.0))
            });
            let running: i64 = makers.iter().filter(|m| m.1).map(|m| m.3).sum();
            // What the makers could use: the surplus with their own use added back.
            let mut budget = (e.income - e.demand).milli() + running;
            let mut extra = i32::from(fill >= BUDGET_EXTRA_FILL);
            for (id, on, _, use_) in ranked {
                let want = if budget >= use_ {
                    budget -= use_;
                    true
                } else if extra > 0 {
                    extra -= 1;
                    true
                } else {
                    false
                };
                if want != on && ctx.in_flight(id).is_none() {
                    out.now(id, Command::SetOn(want));
                }
            }
            return;
        }
        let on_above = fill > i64::from(k.eco.maker_on_above_pct) * 10;
        for (id, on, _, _) in makers {
            let want = if off {
                false
            } else if on_above {
                true
            } else {
                on
            };
            if want != on && ctx.in_flight(id).is_none() {
                out.now(id, Command::SetOn(want));
            }
        }
    }
}
