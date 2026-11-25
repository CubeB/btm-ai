//! Protecting the base and its expansions: rebuild records, the attack watch
//! and fortification plans (SPEC §4.8), outpost tower plans (§4.7) and the
//! memory of raided extractor sites (§4.2 rule 2).

use std::collections::{BTreeMap, BTreeSet};

use super::super::{Ctx, Role, World};
use super::facts::{dist, secs, Census};
use super::geom::{self, Rect};
use super::siting::{self, S};
use crate::knobs::Knobs;
use crate::view::{DefId, GameView, Pos, Tick, UnitId, TILE_SIZE};

/// Losses within this of a record merge into it; a rebuild is skipped while
/// something of ours stands this close (SPEC §4.8).
const MERGE: i64 = 32;
/// Hits kept per tower (SPEC §4.8).
const EPISODES: usize = 6;
/// Outposts are further than this from home (SPEC §4.7).
pub const OUTPOST_DISTANCE: i64 = 900;

/// A lost armed building to rebuild (SPEC §4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub def: DefId,
    pub pos: Pos,
    pub losses: u32,
    pub rebuilds: u32,
    pub first: Tick,
    pub last_loss: Tick,
}

/// One attack episode on a tower (SPEC §4.8 attack watch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Episode {
    last: Tick,
    dir: (i64, i64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Watch {
    health: i32,
    episodes: Vec<Episode>,
}

/// A fortification piece: a wall slot, or an AA tower near a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub def: DefId,
    pub at: Pos,
    /// A wall slot taken exactly; otherwise a plain search round `at`.
    pub exact: bool,
    /// From the every-tower trigger.
    pub every_tower: bool,
}

/// An outpost tower plan (SPEC §4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outpost {
    pub anchor: Pos,
    /// Uncovered extractors the tower would cover.
    pub covered: u32,
    pub raided: bool,
}

/// The rebuild plan's pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rebuild {
    pub index: usize,
    pub def: DefId,
    pub pos: Pos,
}

/// Defence memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Defence {
    pub records: Vec<Record>,
    seen: BTreeSet<UnitId>,
    /// Raided extractor sites with the tick they are forgotten.
    pub raided: Vec<(Pos, Tick)>,
    watch: BTreeMap<UnitId, Watch>,
}

impl Defence {
    /// Takes in this pass's new building losses (records, raids) and forgets
    /// what has lapsed.
    pub fn update(&mut self, view: &dyn GameView, k: &Knobs, w: &World, tick: Tick) {
        let c = Census::new(w);
        for l in &w.own.building_losses {
            if !self.seen.insert(l.id) {
                continue;
            }
            let Some(d) = view.unit_def(l.def) else {
                continue;
            };
            if c.is(l.def, Role::Mex) {
                self.raided
                    .push((l.pos, tick.saturating_add(secs(k.def.raid_memory_s))));
            }
            if !k.def.rebuild || !d.armed() {
                continue;
            }
            match self.records.iter().position(|r| r.pos.within(l.pos, MERGE)) {
                Some(i) => {
                    let forget = self.records.get(i).is_some_and(|r| {
                        r.rebuilds >= u32::try_from(k.def.rebuild_max).unwrap_or(0)
                    });
                    if forget {
                        self.records.remove(i);
                    } else if let Some(r) = self.records.get_mut(i) {
                        r.losses += 1;
                        r.last_loss = l.tick;
                    }
                }
                None => self.records.push(Record {
                    def: l.def,
                    pos: l.pos,
                    losses: 1,
                    rebuilds: 0,
                    first: l.tick,
                    last_loss: l.tick,
                }),
            }
        }
        let live: BTreeSet<UnitId> = w.own.building_losses.iter().map(|l| l.id).collect();
        self.seen.retain(|id| live.contains(id));
        let memory = secs(k.def.rebuild_memory_s);
        self.records
            .retain(|r| tick.saturating_sub(r.first) < memory);
        self.raided.retain(|&(_, until)| tick < until);
    }

    /// The first eligible rebuild record (SPEC §4.8).
    pub fn rebuild(&self, s: &S<'_>) -> Option<Rebuild> {
        if !s.knobs.def.rebuild {
            return None;
        }
        let delay = secs(s.knobs.def.rebuild_delay_s);
        self.records.iter().enumerate().find_map(|(index, r)| {
            let ok = s.tick.saturating_sub(r.last_loss) >= delay
                && !occupied(s, r.pos, MERGE)
                && !siting::under_guns(s.view, s.knobs, s.w, r.pos)
                && !s.failed(r.pos)
                && s.view.map().contains(r.pos);
            ok.then_some(Rebuild {
                index,
                def: r.def,
                pos: r.pos,
            })
        })
    }

    /// Notes that a rebuild was issued.
    pub fn rebuilt(&mut self, index: usize) {
        if let Some(r) = self.records.get_mut(index) {
            r.rebuilds += 1;
        }
    }

    /// The attack watch (SPEC §4.8), every 15 ticks.
    pub fn watch(&mut self, ctx: &Ctx<'_>) {
        let k = ctx.knobs;
        let w = &*ctx.world;
        if !k.fort.where_attacked || w.roles.get(Role::Wall).is_none() {
            self.watch.clear();
            return;
        }
        let tick = ctx.tick;
        let radius = dist(k.fort.attacker_radius);
        let gap = secs(k.fort.attack_gap_s);
        let memory = secs(k.fort.attack_memory_s);
        let mut alive = BTreeSet::new();
        for s in &w.own.units {
            if !s.building || s.is_frame() || !ctx.def(s.unit.def).is_some_and(|d| d.armed()) {
                continue;
            }
            alive.insert(s.id());
            let at = s.unit.pos;
            let entry = self.watch.entry(s.id()).or_insert(Watch {
                health: s.unit.health,
                episodes: Vec::new(),
            });
            if s.unit.health < entry.health {
                let mut sum = (0i64, 0i64);
                for c in w.contacts.near(at, radius) {
                    if c.armed_ground_mobile() {
                        let d = geom::dir(at, c.pos);
                        sum.0 += d.0;
                        sum.1 += d.1;
                    }
                }
                if sum != (0, 0) {
                    match entry.episodes.last_mut() {
                        Some(e) if tick.saturating_sub(e.last) < gap => {
                            e.last = tick;
                            e.dir.0 += sum.0;
                            e.dir.1 += sum.1;
                        }
                        _ => entry.episodes.push(Episode {
                            last: tick,
                            dir: sum,
                        }),
                    }
                    if entry.episodes.len() > EPISODES {
                        entry.episodes.remove(0);
                    }
                }
            }
            entry.health = s.unit.health;
            entry
                .episodes
                .retain(|e| tick.saturating_sub(e.last) < memory);
        }
        self.watch.retain(|id, _| alive.contains(id));
    }

    /// Towers owed walls, with the direction of the latest episode (SPEC §4.8).
    fn owed(&self, k: &Knobs) -> Vec<(UnitId, (i64, i64))> {
        let repeat = usize::try_from(k.fort.repeat_attacks).unwrap_or(usize::MAX);
        self.watch
            .iter()
            .filter_map(|(&id, wt)| {
                let latest = wt.episodes.last()?;
                let near = wt
                    .episodes
                    .iter()
                    .filter(|e| geom::within_45(e.dir, latest.dir))
                    .count();
                (wt.episodes.len() >= repeat && near >= 2).then(|| (id, geom::norm(latest.dir)))
            })
            .collect()
    }

    /// The next fortification piece (SPEC §4.8), one per call.
    pub fn fortification(&self, s: &S<'_>, adv_lab: bool) -> Option<Piece> {
        let k = s.knobs;
        let c = s.census();
        let wall = c.def(Role::Wall)?;
        let wall_def = s.def(wall)?;
        let home = s.facts.home?;
        let every = k.fort.every_tower || (k.fort.at_tier_two && adv_lab);
        let reactive = usize::try_from(if k.fort.wrap {
            k.fort.wrap_walls
        } else {
            k.fort.reactive_walls
        })
        .unwrap_or(0);
        // 1. Towers owed walls on the attacked side.
        if k.fort.where_attacked {
            for (id, dir) in self.owed(k) {
                let Some(t) = s.w.own.get(id) else { continue };
                let Some(td) = s.def(t.unit.def) else {
                    continue;
                };
                if let Some(at) = wall_slot(
                    s,
                    wall,
                    wall_def.footprint,
                    t.unit.pos,
                    td.footprint,
                    dir,
                    reactive,
                ) {
                    return Some(Piece {
                        def: wall,
                        at,
                        exact: true,
                        every_tower: false,
                    });
                }
            }
        }
        // 2. Rebuilt sites: walls facing outward, an AA tower behind twice-lost sites.
        if k.fort.rebuilt {
            for r in &self.records {
                let dir = geom::dir(home, r.pos);
                let dir = if dir == (0, 0) { (geom::UNIT, 0) } else { dir };
                let fp = s.def(r.def).map_or((2, 2), |d| d.footprint);
                if let Some(at) = wall_slot(s, wall, wall_def.footprint, r.pos, fp, dir, reactive) {
                    return Some(Piece {
                        def: wall,
                        at,
                        exact: true,
                        every_tower: false,
                    });
                }
                if k.fort.aa_twice_lost && r.losses >= 2 {
                    if let Some(p) = aa_behind(s, r.pos, dir) {
                        return Some(p);
                    }
                }
            }
        }
        // 3. Every finished light tower: walls facing the enemy base, then AA.
        if every {
            let llt = c.def(Role::LightLaserTower)?;
            let per = usize::try_from(k.fort.walls_per_tower).unwrap_or(0);
            for t in
                s.w.own
                    .units
                    .iter()
                    .filter(|u| u.unit.def == llt && !u.is_frame())
            {
                let towards = s.facts.enemy_base.map(|e| geom::dir(t.unit.pos, e));
                let dir = towards
                    .filter(|d| *d != (0, 0))
                    .unwrap_or_else(|| siting::facing(s));
                let fp = s.def(llt).map_or((2, 2), |d| d.footprint);
                if let Some(at) = wall_slot(s, wall, wall_def.footprint, t.unit.pos, fp, dir, per) {
                    return Some(Piece {
                        def: wall,
                        at,
                        exact: true,
                        every_tower: true,
                    });
                }
                if k.fort.aa_tower {
                    if let Some(mut p) = aa_behind(s, t.unit.pos, dir) {
                        p.every_tower = true;
                        return Some(p);
                    }
                }
            }
        }
        None
    }

    /// The outpost tower plan (SPEC §4.7).
    pub fn outpost(&self, s: &S<'_>) -> Option<Outpost> {
        let k = s.knobs;
        let c = s.census();
        let llt = c.def(Role::LightLaserTower)?;
        let mex = c.def(Role::Mex)?;
        if k.def.outpost_towers <= 0 {
            return None;
        }
        let home = s.facts.home?;
        let range = i64::from(s.def(llt)?.max_range());
        let towers: Vec<Pos> =
            s.w.own
                .units
                .iter()
                .filter(|u| u.unit.def == llt)
                .map(|u| u.unit.pos)
                .collect();
        let outposts = towers
            .iter()
            .filter(|p| !p.within(home, OUTPOST_DISTANCE))
            .count();
        let base = i64::from(k.def.outpost_towers);
        let step = i64::from(k.def.outpost_income_step);
        let grow = if step > 0 { s.facts.income / step } else { 0 };
        let cap = base.max(i64::from(k.def.outpost_max)).min(base + grow);
        if i64::try_from(outposts).unwrap_or(i64::MAX) >= cap {
            return None;
        }
        let covered = |p: Pos| towers.iter().any(|t| t.within(p, range));
        let uncovered: Vec<Pos> =
            s.w.own
                .units
                .iter()
                .filter(|u| u.unit.def == mex && !u.is_frame())
                .map(|u| u.unit.pos)
                .filter(|p| !p.within(home, OUTPOST_DISTANCE) && !covered(*p))
                .collect();
        let raids: Vec<Pos> = self
            .raided
            .iter()
            .map(|&(p, _)| p)
            .filter(|p| !p.within(home, OUTPOST_DISTANCE) && !covered(*p))
            .collect();
        let bonus = i64::from(k.def.outpost_min_mex);
        let mut best: Option<(i64, Outpost)> = None;
        for (seed, is_raid) in uncovered
            .iter()
            .map(|&p| (p, false))
            .chain(raids.iter().map(|&p| (p, true)))
        {
            let near: Vec<Pos> = uncovered
                .iter()
                .copied()
                .filter(|m| m.within(seed, range))
                .collect();
            let raided = raids.iter().any(|r| r.within(seed, range));
            let score = i64::try_from(near.len()).unwrap_or(0) + if raided { bonus } else { 0 };
            if best.as_ref().is_some_and(|(b, _)| score <= *b) {
                continue;
            }
            let anchor = if is_raid || near.is_empty() {
                seed
            } else {
                let n = i64::try_from(near.len()).unwrap_or(1).max(1);
                let (sx, sz) = near.iter().fold((0i64, 0i64), |a, p| {
                    (a.0 + i64::from(p.x), a.1 + i64::from(p.z))
                });
                Pos::new(
                    i32::try_from(sx / n).unwrap_or(seed.x),
                    i32::try_from(sz / n).unwrap_or(seed.z),
                )
            };
            best = Some((
                score,
                Outpost {
                    anchor,
                    covered: u32::try_from(near.len()).unwrap_or(0),
                    raided,
                },
            ));
        }
        best.filter(|(score, _)| *score >= bonus.max(1))
            .map(|(_, o)| o)
    }
}

/// Something of ours stands within `r` of `p`, or a build is pending there.
pub fn occupied(s: &S<'_>, p: Pos, r: i64) -> bool {
    s.w.own
        .units
        .iter()
        .any(|u| u.building && u.unit.pos.within(p, r))
        || s.reserved.iter().any(|(_, q)| q.within(p, r))
}

/// The first open wall slot of the `n` that face `dir` most squarely round a
/// footprint (wrap mode), or along a line in front of it (line mode).
fn wall_slot(
    s: &S<'_>,
    wall: DefId,
    wfp: (i32, i32),
    centre: Pos,
    fp: (i32, i32),
    dir: (i64, i64),
    n: usize,
) -> Option<Pos> {
    let k = s.knobs;
    let slots: Vec<Pos> = if k.fort.wrap {
        let mut ring = wrap_slots(centre, fp, wfp, k.fort.wrap_gap_tiles.max(0));
        ring.sort_by_key(|&p| (-geom::forward(centre, dir, p), p.x, p.z));
        ring.truncate(n);
        ring
    } else {
        let front = geom::along(centre, dir, dist(k.fort.line_distance));
        let perp = (-dir.1, dir.0);
        let step = i64::from(wfp.0.max(1) * TILE_SIZE);
        (0..n)
            .map(|i| {
                // Middle first: 0, +1, −1, +2, −2, ...
                let i = i64::try_from(i).unwrap_or(0);
                let off = if i % 2 == 1 { (i + 1) / 2 } else { -(i / 2) };
                geom::along(front, perp, off * step)
            })
            .collect()
    };
    let wd = s.def(wall)?;
    for at in slots {
        if s.w
            .own
            .units
            .iter()
            .any(|u| u.unit.def == wall && Rect::of(wfp, u.unit.pos) == Rect::of(wfp, at))
            || s.reserved
                .iter()
                .any(|&(d, p)| d == wall && Rect::of(wfp, p) == Rect::of(wfp, at))
        {
            continue;
        }
        if s.failed(at)
            || siting::under_guns(s.view, s.knobs, s.w, at)
            || !s.view.map().contains(at)
            || !s.leaks.leak_can_place(wall, at)
            || Rect::of(wd.footprint, at)
                .tiles()
                .any(|t| s.w.map.deposit_at(t).is_some())
        {
            continue;
        }
        return Some(at);
    }
    None
}

/// Wall-sized slots hugging a footprint at a gap, corners included.
fn wrap_slots(centre: Pos, fp: (i32, i32), wfp: (i32, i32), gap: i32) -> Vec<Pos> {
    let r = Rect::of(fp, centre);
    let (ww, wz) = (wfp.0.max(1), wfp.1.max(1));
    let (x0, z0) = (r.x0 - gap - ww, r.z0 - gap - wz);
    let (x1, z1) = (r.x1 + gap, r.z1 + gap);
    let mut out = Vec::new();
    let mut z = z0;
    while z <= z1 {
        let mut x = x0;
        while x <= x1 {
            if x == x0 || x >= x1 || z == z0 || z >= z1 {
                out.push(Pos::new(
                    x * TILE_SIZE + ww * TILE_SIZE / 2,
                    z * TILE_SIZE + wz * TILE_SIZE / 2,
                ));
            }
            x += ww;
        }
        z += wz;
    }
    out
}

/// An AA tower `fort.aa_back` behind a point (towards home), unless one
/// already covers it within `fort.aa_cover_radius`.
fn aa_behind(s: &S<'_>, p: Pos, outward: (i64, i64)) -> Option<Piece> {
    let k = s.knobs;
    let aa = s.census().def(Role::AntiAirTower)?;
    let at = geom::along(p, outward, -dist(k.fort.aa_back));
    let cover = dist(k.fort.aa_cover_radius);
    let covered =
        s.w.own
            .units
            .iter()
            .any(|u| u.unit.def == aa && u.unit.pos.within(at, cover))
            || s.reserved
                .iter()
                .any(|&(d, q)| d == aa && q.within(at, cover));
    (!covered).then_some(Piece {
        def: aa,
        at,
        exact: false,
        every_tower: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_slots_ring_a_tower() {
        // A 2 × 2 tower with 2 × 2 walls at gap 0: eight slots.
        let slots = wrap_slots(Pos::new(32, 32), (2, 2), (2, 2), 0);
        assert_eq!(slots.len(), 8);
        assert!(slots.contains(&Pos::new(0, 0)));
        assert!(slots.contains(&Pos::new(64, 64)));
        assert!(!slots.contains(&Pos::new(32, 32)));
    }
}
