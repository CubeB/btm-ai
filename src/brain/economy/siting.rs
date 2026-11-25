//! Site choice: extractor sites (SPEC §4.5) and generic siting (§4.6).
//!
//! Everything here reads; the only state it touches is the heart-wait clock
//! and the seat's random stream, which breaks exact ties only (SPEC §7).
//!
//! Clearance lanes: every building is surrounded by a lane, 2 tiles normally
//! and 3 round factories. A candidate footprint that reaches into a lane
//! (fewer tiles from the other footprint than the lane is wide) is
//! discarded when either building is a factory, and is otherwise only a
//! *crowded* fallback. Pending build orders of our own builders count as
//! buildings, so two builders never pick the same spot.

use std::collections::BTreeMap;

use super::super::{Role, World};
use super::facts::{dist, extractor_near_world, our_side, secs, Census, Facts, EXTRACTOR_TAKEN};
use super::geom::{self, Rect};
use crate::knobs::Knobs;
use crate::rng::AiRng;
use crate::view::{
    DefId, FeatureId, FogLeaks, GameView, Pos, Tick, Tile, UnitDef, UnitId, TILE_SIZE,
};

/// What a site search may read.
#[derive(Clone, Copy)]
pub struct S<'x> {
    pub view: &'x dyn GameView,
    pub leaks: &'x dyn FogLeaks,
    pub knobs: &'x Knobs,
    pub w: &'x World,
    pub facts: &'x Facts,
    /// Pending builds of our own builders (queues, in flight, staged).
    pub reserved: &'x [(DefId, Pos)],
    pub vents: &'x [Pos],
    pub tick: Tick,
}

impl S<'_> {
    pub fn census(&self) -> Census<'_> {
        Census::new(self.w)
    }

    pub fn def(&self, d: DefId) -> Option<&UnitDef> {
        self.view.unit_def(d)
    }

    pub fn failed(&self, p: Pos) -> bool {
        self.w
            .eco
            .failed_cells
            .get(&p.vis_cell())
            .is_some_and(|&until| self.tick < until)
    }
}

/// The builder a job is planned for, with its context (SPEC §4.2 rule 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Builder {
    pub id: UnitId,
    pub def: DefId,
    pub pos: Pos,
    pub commander: bool,
    pub flies: bool,
    /// A ship (construction ship).
    pub naval: bool,
    pub at_base: bool,
    /// Home when at base, else the builder's own position.
    pub anchor: Pos,
    /// Work-ticks per tick.
    pub worker: i64,
    /// Plans on the commander's own layer.
    pub cmd_layer: bool,
}

impl Builder {
    /// Reachable from home on the builder's layer (flyers reach everything).
    pub fn reachable(&self, w: &World, p: Pos) -> bool {
        if self.flies {
            true
        } else if self.naval {
            w.reach.naval_reachable(p)
        } else if self.cmd_layer {
            w.reach.commander_reachable(p)
        } else {
            w.reach.ground_reachable(p)
        }
    }

    /// On the builder's side of reachability (SPEC §4.2 rule 3).
    pub fn on_side(&self, w: &World, p: Pos) -> bool {
        self.flies || self.reachable(w, p) == self.at_base
    }
}

/// How well a candidate fits the lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    Clean,
    Crowded,
}

/// What occupies an extractor placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    Legal,
    /// Blocked only by reclaimable features: reclaim the first, then build.
    Wreck(FeatureId),
    /// Blocked by something that is neither a building nor a feature (a
    /// unit standing on it).
    Unit,
    Illegal,
}

/// A chosen site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    pub at: Pos,
    /// A reclaimable wreck to clear first.
    pub wreck: Option<FeatureId>,
}

impl Site {
    pub fn at(at: Pos) -> Site {
        Site { at, wreck: None }
    }
}

/// Range an armed ground contact that is not a building threatens (SPEC §4.6).
pub const MOBILE_GUN_RANGE: i64 = 400;

/// "Under enemy guns" (SPEC §4.6): remembered contacts only, never aircraft,
/// needs `build.notice_harassment`.
pub fn under_guns(view: &dyn GameView, k: &Knobs, w: &World, p: Pos) -> bool {
    if !k.build.notice_harassment {
        return false;
    }
    let margin = dist(k.build.enemy_gun_margin);
    w.contacts.iter().any(|c| {
        if !c.armed || c.air || c.frame {
            return false;
        }
        let r = if c.building && k.build.enemy_gun_range_from_weapon {
            view.unit_def(c.def)
                .map_or(MOBILE_GUN_RANGE, |d| i64::from(d.max_range()) + margin)
        } else {
            MOBILE_GUN_RANGE
        };
        c.pos.within(p, r)
    })
}

/// A remembered armed ground (non-air) enemy within `r` of `p`.
pub fn armed_ground_within(w: &World, p: Pos, r: i64) -> bool {
    w.contacts.near(p, r).any(|c| c.armed && !c.air && !c.frame)
}

/// The grid pitch for a type: larger footprint side + 2 tiles, in world units.
pub fn pitch(d: &UnitDef) -> i32 {
    (d.footprint.0.max(d.footprint.1).max(1) + 2) * TILE_SIZE
}

fn in_map(s: &S<'_>, r: &Rect) -> bool {
    r.x0 >= 0 && r.z0 >= 0 && r.x1 <= s.w.map.width_tiles && r.z1 <= s.w.map.height_tiles
}

/// The lane test against every building (of any owner, SPEC §1.4) and our
/// pending builds. `only_factories`: report only factory lanes (extractors).
pub fn lanes(s: &S<'_>, d: &UnitDef, rect: &Rect, only_factories: bool) -> Option<Fit> {
    let me_factory = d.is_factory();
    let span = d.footprint.0.max(d.footprint.1).max(1);
    // Centre to centre: our half, a lane of 3 and the largest footprint (8).
    let radius = (span / 2 + 3 + 8) * TILE_SIZE * 3 / 2;
    let mut crowded = false;
    let mut check = |od: &UnitDef, at: Pos| -> bool {
        let or = Rect::of(od.footprint, at);
        let factory = me_factory || od.is_factory();
        let lane = if factory { 3 } else { 2 };
        let g = rect.gap(&or);
        if g < 0 {
            return false;
        }
        if g < lane {
            if factory {
                return false;
            }
            crowded = true;
        }
        true
    };
    for b in s.leaks.leak_buildings_near(rect.center(), radius) {
        let Some(od) = s.view.unit_def(b.def) else {
            continue;
        };
        if only_factories && !od.is_factory() && !me_factory {
            continue;
        }
        if !check(od, b.pos) {
            return None;
        }
    }
    for &(rd, rp) in s.reserved {
        let Some(od) = s.view.unit_def(rd) else {
            continue;
        };
        if only_factories && !od.is_factory() {
            // A pending extractor build still may not be overlapped.
            if Rect::of(od.footprint, rp).overlaps(rect) {
                return None;
            }
            continue;
        }
        if !check(od, rp) {
            return None;
        }
    }
    Some(if crowded { Fit::Crowded } else { Fit::Clean })
}

/// The generic placement test (SPEC §4.6): in the map, no metal patch tile
/// under the footprint, no vent buried (unless it is a geothermal plant),
/// legal, and the lanes.
pub fn fit(s: &S<'_>, def: DefId, d: &UnitDef, at: Pos) -> Option<Fit> {
    let rect = Rect::of(d.footprint, at);
    if !in_map(s, &rect) {
        return None;
    }
    if rect.tiles().any(|t| s.w.map.deposit_at(t).is_some()) {
        return None;
    }
    let geo = s.w.roles.is(def, Role::Geothermal);
    if !geo && s.vents.iter().any(|&v| rect.contains(v)) {
        return None;
    }
    if !s.leaks.leak_can_place(def, at) {
        return None;
    }
    lanes(s, d, &rect, false)
}

/// What occupies an extractor or rebuild placement.
pub fn spot(s: &S<'_>, def: DefId, d: &UnitDef, at: Pos) -> Spot {
    if s.leaks.leak_can_place(def, at) {
        return Spot::Legal;
    }
    if !s.view.can_place_terrain(def, at) {
        return Spot::Illegal;
    }
    let rect = Rect::of(d.footprint, at);
    let span = d.footprint.0.max(d.footprint.1).max(1);
    let radius = (span + 8) * TILE_SIZE;
    let building = s.leaks.leak_buildings_near(at, radius).iter().any(|b| {
        s.view
            .unit_def(b.def)
            .is_some_and(|bd| Rect::of(bd.footprint, b.pos).overlaps(&rect))
    });
    if building {
        return Spot::Illegal;
    }
    let features: Vec<_> = s
        .leaks
        .leak_features_near(at, radius)
        .into_iter()
        .filter(|f| Rect::of(f.footprint, f.pos).overlaps(&rect))
        .collect();
    match features.first() {
        None => Spot::Unit,
        Some(f) if features.iter().all(|f| f.reclaimable) => Spot::Wreck(f.id),
        Some(_) => Spot::Illegal,
    }
}

/// Plain ring search (SPEC §4.6): the nearest ring with any good site within
/// `first` (widening to `fallback`), then a random pick within that ring.
/// Crowded sites are used only if no clean site exists.
pub fn plain(
    rng: &mut AiRng,
    anchor: Pos,
    pitch: i32,
    first: i64,
    fallback: i64,
    mut good: impl FnMut(Pos) -> Option<Fit>,
) -> Option<Pos> {
    let pitch = pitch.max(TILE_SIZE);
    let max_ring = i32::try_from(first.max(fallback) / i64::from(pitch)).unwrap_or(0);
    let mut crowded: Option<Vec<Pos>> = None;
    for r in 0..=max_ring {
        let mut clean = Vec::new();
        let mut tight = Vec::new();
        for (dx, dz) in geom::ring(r) {
            let p = Pos::new(anchor.x + dx * pitch, anchor.z + dz * pitch);
            match good(p) {
                Some(Fit::Clean) => clean.push(p),
                Some(Fit::Crowded) => tight.push(p),
                None => {}
            }
        }
        if !clean.is_empty() {
            return rng.pick(&clean).copied();
        }
        if crowded.is_none() && !tight.is_empty() {
            crowded = Some(tight);
        }
    }
    crowded.and_then(|c| rng.pick(&c).copied())
}

/// A score: compared lexicographically, highest wins.
pub type Score = [i64; 3];

/// Scored ring search (SPEC §4.6): every ring out to `first` (else
/// `fallback`), highest score wins, exact ties at random; clean before
/// crowded.
pub fn scored(
    rng: &mut AiRng,
    anchor: Pos,
    pitch: i32,
    first: i64,
    fallback: i64,
    mut good: impl FnMut(Pos, i32) -> Option<(Fit, Score)>,
) -> Option<Pos> {
    let pitch = pitch.max(TILE_SIZE);
    let mut search = |radius: i64| -> Option<Pos> {
        let max_ring = i32::try_from(radius / i64::from(pitch)).unwrap_or(0);
        let mut best: Option<(Fit, Score, Vec<Pos>)> = None;
        for r in 0..=max_ring {
            for (dx, dz) in geom::ring(r) {
                let p = Pos::new(anchor.x + dx * pitch, anchor.z + dz * pitch);
                let Some((fit, score)) = good(p, r) else {
                    continue;
                };
                match best.as_mut() {
                    None => best = Some((fit, score, vec![p])),
                    Some(b) => {
                        // Clean (smaller) beats crowded, then the higher score.
                        let better = fit < b.0 || (fit == b.0 && score > b.1);
                        if better {
                            *b = (fit, score, vec![p]);
                        } else if fit == b.0 && score == b.1 {
                            b.2.push(p);
                        }
                    }
                }
            }
        }
        best.and_then(|(_, _, ps)| rng.pick(&ps).copied())
    };
    search(first).or_else(|| {
        if fallback > first {
            search(fallback)
        } else {
            None
        }
    })
}

/// The threat facing (SPEC §4.6): the age-weighted mean direction of
/// buildings lost in the last 60 s (`def.face_losses`), else the enemy base,
/// else the map centre, else +x; overridden by the strongest threat sector
/// (`def.face_threat_sector`). Scaled to [`geom::UNIT`].
pub fn facing(s: &S<'_>) -> (i64, i64) {
    let Some(home) = s.facts.home else {
        return (geom::UNIT, 0);
    };
    let k = s.knobs;
    if k.def.face_threat_sector {
        if let Some(i) = s.w.threat.sectors.strongest() {
            if let Some(&d) = geom::SECTOR_DIR.get(i) {
                return d;
            }
        }
    }
    if k.def.face_losses {
        let window = i64::from(secs(LOSS_FACING_S));
        let mut sum = (0i64, 0i64);
        for l in &s.w.own.building_losses {
            let age = i64::from(s.tick.saturating_sub(l.tick));
            if age >= window {
                continue;
            }
            let d = geom::dir(home, l.pos);
            sum.0 += d.0 * (window - age);
            sum.1 += d.1 * (window - age);
        }
        let d = geom::norm(sum);
        if d != (0, 0) {
            return d;
        }
    }
    if let Some(e) = s.facts.enemy_base {
        let d = geom::dir(home, e);
        if d != (0, 0) {
            return d;
        }
    }
    let centre = Pos::new(
        s.w.map.width_tiles * TILE_SIZE / 2,
        s.w.map.height_tiles * TILE_SIZE / 2,
    );
    let d = geom::dir(home, centre);
    if d != (0, 0) {
        d
    } else {
        (geom::UNIT, 0)
    }
}

/// Losses older than this do not steer the facing (SPEC §4.6).
const LOSS_FACING_S: i32 = 60;

/// Own units and pending builds of `def`: (position, finished).
fn same_type(s: &S<'_>, def: DefId) -> Vec<Pos> {
    s.w.own
        .units
        .iter()
        .filter(|u| u.unit.def == def)
        .map(|u| u.unit.pos)
        .chain(
            s.reserved
                .iter()
                .filter(|(d, _)| *d == def)
                .map(|&(_, p)| p),
        )
        .collect()
}

/// The defence site for a light or AA tower at base (SPEC §4.6).
pub fn defence_site(s: &S<'_>, rng: &mut AiRng, def: DefId, d: &UnitDef, aa: bool) -> Option<Pos> {
    let home = s.facts.home?;
    let k = s.knobs;
    let range = i64::from(d.max_range());
    let towers = same_type(s, def);
    let spread = |p: Pos| -> i64 {
        towers
            .iter()
            .map(|t| t.dist(p))
            .min()
            .map_or(range, |m| m.min(range))
    };
    let pitch = pitch(d);
    let first = dist(k.build.site_radius);
    let fallback = dist(k.build.site_fallback_radius);
    if aa {
        // Own buildings no AA tower covers yet.
        let uncovered: Vec<Pos> =
            s.w.own
                .units
                .iter()
                .filter(|u| u.building && u.unit.def != def)
                .map(|u| u.unit.pos)
                .filter(|p| !towers.iter().any(|t| t.within(*p, range)))
                .collect();
        return scored(rng, home, pitch, first, fallback, |p, _| {
            if !s.w.reach.ground_reachable(p) || s.failed(p) || under_guns(s.view, k, s.w, p) {
                return None;
            }
            let f = fit(s, def, d, p)?;
            let covers = uncovered.iter().filter(|u| u.within(p, range)).count();
            Some((
                f,
                [i64::try_from(covers).unwrap_or(0), spread(p), -p.dist(home)],
            ))
        });
    }
    let base = dist(k.def.distance);
    let count = i64::try_from(towers.len()).unwrap_or(i64::MAX);
    let post = if k.def.perimeter_first && count < i64::from(k.def.perimeter_count) {
        let outer =
            s.w.own
                .units
                .iter()
                .filter(|u| u.building)
                .map(|u| u.unit.pos.dist(home))
                .max()
                .unwrap_or(0);
        base.max((outer + dist(k.def.perimeter_margin)).min(dist(k.army.defend_radius)))
    } else {
        base
    };
    let face = facing(s);
    let radius = first.max(post + i64::from(pitch));
    scored(rng, home, pitch, radius, fallback.max(radius), |p, _| {
        if !s.w.reach.ground_reachable(p) || s.failed(p) || under_guns(s.view, k, s.w, p) {
            return None;
        }
        let f = fit(s, def, d, p)?;
        let fwd = geom::forward(home, face, p);
        Some((f, [spread(p), fwd.min(post), -p.dist(home)]))
    })
}

/// The radar post (SPEC §4.6): home + facing × `def.radar_distance`; the
/// nearest ring with a good site, then the highest ground.
pub fn radar_site(s: &S<'_>, rng: &mut AiRng, b: &Builder, def: DefId, d: &UnitDef) -> Option<Pos> {
    let home = s.facts.home?;
    let k = s.knobs;
    let post = geom::along(home, facing(s), dist(k.def.radar_distance));
    let pitch = pitch(d);
    let first = dist(k.build.site_radius);
    let fallback = dist(k.build.site_fallback_radius);
    let max_ring = i32::try_from(fallback.max(first) / i64::from(pitch)).unwrap_or(0);
    for r in 0..=max_ring {
        let mut best: Option<(i32, Vec<Pos>)> = None;
        for (dx, dz) in geom::ring(r) {
            let p = Pos::new(post.x + dx * pitch, post.z + dz * pitch);
            if !b.on_side(s.w, p) || s.failed(p) || under_guns(s.view, k, s.w, p) {
                continue;
            }
            if fit(s, def, d, p).is_none() {
                continue;
            }
            let h = s.view.height(p.tile());
            match best.as_mut() {
                Some(bb) if h < bb.0 => {}
                Some(bb) if h == bb.0 => bb.1.push(p),
                _ => best = Some((h, vec![p])),
            }
        }
        if let Some((_, ps)) = best {
            return rng.pick(&ps).copied();
        }
    }
    None
}

/// Energy rows (SPEC §4.6): adjacency to a same-type unit one pitch along a
/// row or column (diagonal half), then −ring + (1 − cos(angle to the threat
/// direction)) / 4, then nearest to the builder.
pub fn energy_rows(
    s: &S<'_>,
    rng: &mut AiRng,
    b: &Builder,
    def: DefId,
    d: &UnitDef,
) -> Option<Pos> {
    let k = s.knobs;
    let anchor = b.anchor;
    let pitch = pitch(d);
    let pt = pitch / TILE_SIZE;
    let face = facing(s);
    let mates: Vec<Tile> = same_type(s, def)
        .into_iter()
        .map(|p| crate::view::footprint_origin(d.footprint, p))
        .collect();
    scored(
        rng,
        anchor,
        pitch,
        dist(k.build.site_radius),
        dist(k.build.site_fallback_radius),
        |p, ring| {
            if !b.on_side(s.w, p) || s.failed(p) || under_guns(s.view, k, s.w, p) {
                return None;
            }
            let f = fit(s, def, d, p)?;
            let o = crate::view::footprint_origin(d.footprint, p);
            let mut adj = 0;
            for m in &mates {
                let (ax, az) = ((m.x - o.x).abs(), (m.z - o.z).abs());
                if (ax == pt && az == 0) || (ax == 0 && az == pt) {
                    adj = 2;
                    break;
                }
                if ax == pt && az == pt {
                    adj = 1;
                }
            }
            let cos = geom::cos_permille(geom::dir(anchor, p), face);
            let away = -i64::from(ring) * 1000 + (1000 - cos) / 4;
            Some((f, [adj, away, -p.dist(b.pos)]))
        },
    )
}

/// A plain ring search round `anchor`, reachable for the builder unless
/// `water` (SPEC §4.6 "anything else" and "water structures").
pub fn ring_site(
    s: &S<'_>,
    rng: &mut AiRng,
    b: &Builder,
    def: DefId,
    d: &UnitDef,
    anchor: Pos,
    water: bool,
) -> Option<Pos> {
    let k = s.knobs;
    plain(
        rng,
        anchor,
        pitch(d),
        dist(k.build.site_radius),
        dist(k.build.site_fallback_radius),
        |p| {
            if (!water && !b.on_side(s.w, p)) || s.failed(p) || under_guns(s.view, k, s.w, p) {
                return None;
            }
            fit(s, def, d, p)
        },
    )
}

/// The nearest explored, free, reachable vent (SPEC §4.6 geothermal).
pub fn geothermal_site(s: &S<'_>, b: &Builder, def: DefId) -> Option<Pos> {
    s.vents
        .iter()
        .copied()
        .filter(|&v| s.view.cell_explored(v.vis_cell()))
        .filter(|&v| !s.leaks.leak_vent_occupied(v))
        .filter(|&v| b.on_side(s.w, v) && !s.failed(v))
        .filter(|&v| !under_guns(s.view, s.knobs, s.w, v))
        .filter(|&v| s.leaks.leak_can_place(def, v))
        .min_by_key(|v| (v.dist2(b.anchor), v.x, v.z))
}

/// A shipyard site (SPEC §4.6): nearest to home among the precomputed sites,
/// skipping failed ones, ones within `navy.yard_spacing` of our yards and
/// illegal ones; an open site is preferred if at most 256 further.
pub fn shipyard_site(s: &S<'_>, rng: &mut AiRng, def: DefId) -> Option<Pos> {
    let home = s.facts.home?;
    let c = s.census();
    let spacing = dist(s.knobs.navy.yard_spacing);
    let yards: Vec<Pos> = s
        .w
        .own
        .units
        .iter()
        .filter(|u| c.is(u.unit.def, Role::Shipyard) || c.is(u.unit.def, Role::AdvancedShipyard))
        .map(|u| u.unit.pos)
        .chain(
            s.reserved
                .iter()
                .filter(|(d, _)| *d == def)
                .map(|&(_, p)| p),
        )
        .collect();
    let ok: Vec<(Pos, bool, i64)> =
        s.w.map
            .shipyard_sites
            .iter()
            .map(|site| (site.center(), site.open))
            .filter(|&(p, _)| !s.failed(p) && !yards.iter().any(|y| y.within(p, spacing)))
            .filter(|&(p, _)| s.leaks.leak_can_place(def, p))
            .map(|(p, open)| (p, open, p.dist(home)))
            .collect();
    let nearest = ok.iter().map(|o| o.2).min()?;
    let open_best = ok.iter().filter(|o| o.1).map(|o| o.2).min();
    let (want_open, at) = match open_best {
        Some(o) if o <= nearest + OPEN_YARD_SLACK => (true, o),
        _ => (false, nearest),
    };
    let ties: Vec<Pos> = ok
        .iter()
        .filter(|o| o.2 == at && (!want_open || o.1))
        .map(|o| o.0)
        .collect();
    rng.pick(&ties).copied()
}

/// An open shipyard site may be this much further than the nearest.
const OPEN_YARD_SLACK: i64 = 256;
/// Torpedo launcher siting distances (SPEC §4.6).
const LAUNCHER_OUT: i64 = 200;
const LAUNCHER_SPACING: i64 = 400;

/// A torpedo launcher (SPEC §4.6): 200 out from the first yard without a
/// launcher within 400, towards the nearest remembered armed enemy unit,
/// else the enemy base, else away from home.
pub fn launcher_site(
    s: &S<'_>,
    rng: &mut AiRng,
    b: &Builder,
    def: DefId,
    d: &UnitDef,
) -> Option<Pos> {
    let home = s.facts.home?;
    let c = s.census();
    let launchers = same_type(s, def);
    let yard = s
        .w
        .own
        .units
        .iter()
        .filter(|u| !u.is_frame())
        .filter(|u| c.is(u.unit.def, Role::Shipyard) || c.is(u.unit.def, Role::AdvancedShipyard))
        .map(|u| u.unit.pos)
        .find(|y| !launchers.iter().any(|l| l.within(*y, LAUNCHER_SPACING)))?;
    let enemy =
        s.w.contacts
            .iter()
            .filter(|c| c.armed && !c.building)
            .min_by_key(|c| (c.pos.dist2(yard), c.id))
            .map(|c| c.pos)
            .or(s.facts.enemy_base);
    let dir = match enemy {
        Some(e) => geom::dir(yard, e),
        None => geom::dir(home, yard),
    };
    let anchor = geom::along(yard, dir, LAUNCHER_OUT);
    ring_site(s, rng, b, def, d, anchor, true)
}

/// Site filters for an extractor search (SPEC §4.5).
#[derive(Clone, Copy, Debug)]
pub struct MexQuery {
    pub anchor: Pos,
    pub radius: i64,
    /// Expansion: the "our side" rule.
    pub our_side: bool,
    /// Expansion: explored cells only.
    pub explored: bool,
    /// The commander's leash: (home, radius).
    pub leash: Option<(Pos, i64)>,
}

/// The result of an extractor search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MexFound {
    Site(Site),
    /// Nothing; `boxed` when only the our-side rule refused patches.
    None {
        boxed: bool,
    },
}

/// How long a deposit waits for a unit standing on its heart (SPEC §4.5).
const HEART_WAIT_S: i32 = 60;
/// Sites within this of a pending upgrade are refused (SPEC §4.5).
const UPGRADE_CLEAR: i64 = 64;

/// The extractor site search of SPEC §4.5 for `def`, deposits nearest the
/// anchor first.
pub fn extractor_site(
    s: &S<'_>,
    rng: &mut AiRng,
    heart: &mut BTreeMap<usize, Tick>,
    b: &Builder,
    def: DefId,
    q: MexQuery,
) -> MexFound {
    let Some(d) = s.def(def) else {
        return MexFound::None { boxed: false };
    };
    let k = s.knobs;
    let w = s.w;
    let uw = s.census().is(def, Role::UnderwaterMex);
    let fp = d.footprint;
    let guns = dist(k.build.mex_avoid_guns_radius);
    let claim = dist(k.build.claim_radius);
    let home = s.facts.home.unwrap_or(q.anchor);
    let mut deposits: Vec<_> = w
        .map
        .deposits
        .iter()
        .filter(|dep| dep.center.within(q.anchor, q.radius))
        .collect();
    deposits.sort_by_key(|dep| (dep.center.dist2(q.anchor), dep.id));
    let mut our_side_refused = false;
    for dep in deposits {
        let wet = s.view.water_depth(dep.center.tile()) > 0;
        if wet != uw {
            continue;
        }
        if extractor_near_world(s.view, w, dep.center, EXTRACTOR_TAKEN) {
            continue;
        }
        let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for t in &dep.tiles {
            x0 = x0.min(t.x);
            z0 = z0.min(t.z);
            x1 = x1.max(t.x);
            z1 = z1.max(t.z);
        }
        let mut best_any = 0i64;
        // (metal, −distance² to the deposit centre, −distance² to the anchor)
        let mut best: Option<((i64, i64, i64), Vec<Site>)> = None;
        for oz in (z0 - fp.1 + 1)..=z1 {
            for ox in (x0 - fp.0 + 1)..=x1 {
                let rect = Rect {
                    x0: ox,
                    z0: oz,
                    x1: ox + fp.0,
                    z1: oz + fp.1,
                };
                if !in_map(s, &rect) {
                    continue;
                }
                let metal: i64 = rect.tiles().map(|t| i64::from(s.view.metal(t))).sum();
                if metal <= 0 {
                    continue;
                }
                let at = Pos::new(
                    ox * TILE_SIZE + fp.0 * TILE_SIZE / 2,
                    oz * TILE_SIZE + fp.1 * TILE_SIZE / 2,
                );
                if lanes(s, d, &rect, true).is_none() {
                    continue;
                }
                let wreck = match spot(s, def, d, at) {
                    Spot::Illegal => continue,
                    Spot::Unit => {
                        best_any = best_any.max(metal);
                        continue;
                    }
                    Spot::Legal => None,
                    Spot::Wreck(f) => Some(f),
                };
                best_any = best_any.max(metal);
                // Site filters.
                let refused = armed_ground_within(w, at, guns)
                    || w.claims.claimed_near(at, claim, Some(b.id))
                    || w.eco
                        .pending_upgrades
                        .iter()
                        .any(|u| u.within(at, UPGRADE_CLEAR))
                    || s.failed(at)
                    || !b.on_side(w, at)
                    || q.leash.is_some_and(|(h, r)| !at.within(h, r))
                    || (q.explored && !s.view.cell_explored(at.vis_cell()));
                if refused {
                    continue;
                }
                if q.our_side && !our_side(at, home, s.facts.enemy_base) {
                    our_side_refused = true;
                    continue;
                }
                let key = (metal, -at.dist2(dep.center), -at.dist2(q.anchor));
                let site = Site { at, wreck };
                match best.as_mut() {
                    Some(bb) if key < bb.0 => {}
                    Some(bb) if key == bb.0 => bb.1.push(site),
                    _ => best = Some((key, vec![site])),
                }
            }
        }
        let Some((key, sites)) = best else {
            continue;
        };
        // The heart wait.
        if best_any > key.0 {
            let since = *heart.entry(dep.id).or_insert(s.tick);
            if s.tick.saturating_sub(since) < secs(HEART_WAIT_S) {
                continue;
            }
        } else {
            heart.remove(&dep.id);
        }
        if let Some(&site) = rng.pick(&sites) {
            return MexFound::Site(site);
        }
    }
    MexFound::None {
        boxed: our_side_refused,
    }
}
