//! The influence grid (SPEC §3.3): vision-cell resolution, rebuilt from the
//! contact memory every `cadence.threat_grid_ticks`.
//!
//! Four layers, all integers: ground threat (summed dps in milli-hp per
//! second), anti-air cover (a count), economic value (metal in milli-units)
//! and staleness (rebuilds since the cell was last visible, starting at
//! 100000 for "never seen"; always 0 with full vision). Scans and ties are
//! in grid scan order (row, then column), never hash order (SPEC §7).

use super::contacts::ContactMemory;
use crate::knobs::Fixed;
use crate::view::{GameView, Pos, VisCell, VIS_CELL_SIZE};

/// Staleness of a cell never seen.
pub const NEVER_SEEN: u32 = 100_000;
/// Staleness is capped here in the scout score.
pub const STALENESS_CAP: u32 = 3000;

/// The grid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InfluenceGrid {
    /// Width in cells.
    pub w: i32,
    /// Height in cells.
    pub h: i32,
    threat: Vec<i64>,
    aa: Vec<u32>,
    value: Vec<i64>,
    stale: Vec<u32>,
    /// Number of rebuilds so far.
    pub rebuilds: u32,
}

impl InfluenceGrid {
    fn index(&self, c: VisCell) -> Option<usize> {
        if c.x < 0 || c.z < 0 || c.x >= self.w || c.z >= self.h {
            return None;
        }
        usize::try_from(c.z * self.w + c.x).ok()
    }

    fn cell(&self, i: usize) -> VisCell {
        let w = self.w.max(1);
        let i = i32::try_from(i).unwrap_or(0);
        VisCell { x: i % w, z: i / w }
    }

    /// Rebuilds every layer from the memory (SPEC §3.3).
    pub fn rebuild(&mut self, view: &dyn GameView, memory: &ContactMemory) {
        let info = view.map();
        let w = (info.width() + VIS_CELL_SIZE - 1) / VIS_CELL_SIZE;
        let h = (info.height() + VIS_CELL_SIZE - 1) / VIS_CELL_SIZE;
        let n = usize::try_from(w.max(0) * h.max(0)).unwrap_or(0);
        if self.w != w || self.h != h || self.stale.len() != n {
            self.w = w;
            self.h = h;
            self.stale = vec![NEVER_SEEN; n];
        }
        self.threat = vec![0; n];
        self.aa = vec![0; n];
        self.value = vec![0; n];
        self.rebuilds += 1;

        for c in memory.iter() {
            let Some(d) = view.unit_def(c.def) else {
                continue;
            };
            let dps: i64 = d.weapons.iter().map(|w| w.dps_milli()).sum();
            if dps > 0 {
                let r = d.max_range();
                self.disc(c.pos, r, |g, i| {
                    if let Some(t) = g.threat.get_mut(i) {
                        *t = t.saturating_add(dps);
                    }
                });
            }
            let aa_range = d
                .weapons
                .iter()
                .filter(|w| w.anti_air)
                .map(|w| w.range)
                .max();
            if let Some(r) = aa_range {
                self.disc(c.pos, r, |g, i| {
                    if let Some(a) = g.aa.get_mut(i) {
                        *a += 1;
                    }
                });
            }
            if c.building {
                if let Some(v) = self
                    .index(c.pos.vis_cell())
                    .and_then(|i| self.value.get_mut(i))
                {
                    *v = v.saturating_add(c.metal.milli());
                }
            }
        }

        let full = view.full_vision();
        for i in 0..n {
            let c = self.cell(i);
            if let Some(s) = self.stale.get_mut(i) {
                *s = if full || view.cell_visible(c) {
                    0
                } else {
                    s.saturating_add(1)
                };
            }
        }
    }

    /// Calls `f` for every cell whose centre is within `r` of `p`.
    fn disc(&mut self, p: Pos, r: i32, mut f: impl FnMut(&mut Self, usize)) {
        let r = i64::from(r.max(0));
        let c = p.vis_cell();
        let span = i32::try_from(r / i64::from(VIS_CELL_SIZE) + 1).unwrap_or(0);
        for z in (c.z - span).max(0)..=(c.z + span).min(self.h - 1) {
            for x in (c.x - span).max(0)..=(c.x + span).min(self.w - 1) {
                let cell = VisCell { x, z };
                if cell.center().within(p, r) {
                    if let Some(i) = self.index(cell) {
                        f(self, i);
                    }
                }
            }
        }
    }

    /// Ground threat of a cell (summed dps, milli).
    pub fn threat_at(&self, c: VisCell) -> i64 {
        self.index(c)
            .and_then(|i| self.threat.get(i).copied())
            .unwrap_or(0)
    }

    /// Anti-air cover count of a cell.
    pub fn aa_at(&self, c: VisCell) -> u32 {
        self.index(c)
            .and_then(|i| self.aa.get(i).copied())
            .unwrap_or(0)
    }

    /// Remembered enemy building metal in a cell (milli).
    pub fn value_at(&self, c: VisCell) -> i64 {
        self.index(c)
            .and_then(|i| self.value.get(i).copied())
            .unwrap_or(0)
    }

    /// Staleness of a cell ([`NEVER_SEEN`] off the grid or before a rebuild).
    pub fn staleness_at(&self, c: VisCell) -> u32 {
        self.index(c)
            .and_then(|i| self.stale.get(i).copied())
            .unwrap_or(NEVER_SEEN)
    }

    /// Sum of ground threat over cells whose centre is within `r` of `p`.
    pub fn threat_in_radius(&self, p: Pos, r: i32) -> i64 {
        let r = i64::from(r.max(0));
        let c = p.vis_cell();
        let span = i32::try_from(r / i64::from(VIS_CELL_SIZE) + 1).unwrap_or(0);
        let mut sum = 0i64;
        for z in (c.z - span).max(0)..=(c.z + span).min(self.h - 1) {
            for x in (c.x - span).max(0)..=(c.x + span).min(self.w - 1) {
                let cell = VisCell { x, z };
                if cell.center().within(p, r) {
                    sum = sum.saturating_add(self.threat_at(cell));
                }
            }
        }
        sum
    }

    /// The best attack cell (SPEC §3.3): among cells holding enemy
    /// buildings, value − threat × `aversion`; the best within `radius` of
    /// `enemy_base` if any, else the best anywhere. Ties: scan order.
    pub fn best_attack_cell(
        &self,
        enemy_base: Option<Pos>,
        radius: i64,
        aversion: Fixed,
    ) -> Option<VisCell> {
        let mut near: Option<(i64, usize)> = None;
        let mut any: Option<(i64, usize)> = None;
        for (i, &v) in self.value.iter().enumerate() {
            if v <= 0 {
                continue;
            }
            let t = self.threat.get(i).copied().unwrap_or(0);
            let score = v.saturating_sub(aversion.apply(t));
            if any.is_none_or(|(b, _)| score > b) {
                any = Some((score, i));
            }
            let inside = enemy_base.is_some_and(|b| self.cell(i).center().within(b, radius));
            if inside && near.is_none_or(|(b, _)| score > b) {
                near = Some((score, i));
            }
        }
        near.or(any).map(|(_, i)| self.cell(i))
    }

    /// The best scout cell (SPEC §3.3): min(staleness, 3000) / max(1,
    /// distance from `from`), among cells passing `accept`. Ratios are
    /// compared exactly by cross-multiplication. Ties: scan order.
    pub fn best_scout_cell(
        &self,
        from: Pos,
        mut accept: impl FnMut(VisCell) -> bool,
    ) -> Option<VisCell> {
        let mut best: Option<(i64, i64, usize)> = None;
        for (i, &s) in self.stale.iter().enumerate() {
            let c = self.cell(i);
            let num = i64::from(s.min(STALENESS_CAP));
            let den = c.center().dist(from).max(1);
            let better = best.is_none_or(|(bn, bd, _)| {
                i128::from(num) * i128::from(bd) > i128::from(bn) * i128::from(den)
            });
            if better && accept(c) {
                best = Some((num, den, i));
            }
        }
        best.map(|(_, _, i)| self.cell(i))
    }
}
