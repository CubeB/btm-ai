//! Map intel computed once at load from public data (SPEC §3.4), the metal
//! deposits of SPEC §4.5, and the extractor-site claims builders make.

use std::collections::BTreeMap;

use crate::view::{Command, DefId, GameView, OwnUnit, Pos, Tick, Tile, UnitId, TILE_SIZE};

/// The map's character by water fraction (SPEC §3.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapCharacter {
    /// Water fraction below 0.12.
    #[default]
    Land,
    /// Below 0.40.
    Mixed,
    /// Otherwise.
    Water,
}

/// A tile where a shipyard's 8 × 8 footprint fits (SPEC §3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShipyardSite {
    /// The footprint's top-left tile.
    pub tile: Tile,
    /// A ring 6 tiles wider is also at least 30 deep.
    pub open: bool,
}

impl ShipyardSite {
    /// The footprint's centre in world units.
    pub fn center(&self) -> Pos {
        Pos::new(
            self.tile.x * TILE_SIZE + SHIPYARD_FP * TILE_SIZE / 2,
            self.tile.z * TILE_SIZE + SHIPYARD_FP * TILE_SIZE / 2,
        )
    }
}

const SHIPYARD_FP: i32 = 8;
const SHIPYARD_RING: i32 = 6;
const SHIPYARD_MIN_DEPTH: i32 = 30;
const SHIPYARD_MAX_DEPTH: i32 = 255;

/// A metal deposit: patch tiles touching at an edge or a corner (SPEC §4.5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deposit {
    /// Index in [`MapIntel::deposits`].
    pub id: usize,
    /// Its patch tiles, in scan order (row, then column).
    pub tiles: Vec<Tile>,
    /// Sum of the tiles' metal values.
    pub metal: i64,
    /// Metal-weighted centre of mass, in world units.
    pub center: Pos,
}

/// The map intel of SPEC §3.4 plus the deposits of §4.5.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapIntel {
    pub width_tiles: i32,
    pub height_tiles: i32,
    /// Share of tiles below sea level, in permille.
    pub water_permille: i64,
    pub character: MapCharacter,
    /// Per tile (scan order): 0 on dry land, else 1 + the index of its
    /// 4-connected water body.
    water_body: Vec<u32>,
    /// Number of water bodies.
    pub water_bodies: u32,
    /// Every shipyard site, in scan order.
    pub shipyard_sites: Vec<ShipyardSite>,
    /// The declared start positions, in slot order.
    pub starts: Vec<Pos>,
    /// Metal deposits, in scan order of their first tile.
    pub deposits: Vec<Deposit>,
    /// Per tile: 0, or 1 + the index of its deposit.
    deposit_of: Vec<u32>,
}

impl MapIntel {
    /// Analyses the map. Runs once, on the first tick.
    pub fn analyse(view: &dyn GameView) -> MapIntel {
        let info = view.map();
        let (w, h) = (info.width_tiles.max(0), info.height_tiles.max(0));
        let n = usize::try_from(w).unwrap_or(0) * usize::try_from(h).unwrap_or(0);
        let mut intel = MapIntel {
            width_tiles: w,
            height_tiles: h,
            starts: view.start_positions(),
            ..MapIntel::default()
        };
        if n == 0 {
            return intel;
        }
        let depth: Vec<i32> = (0..h)
            .flat_map(|z| (0..w).map(move |x| Tile::new(x, z)))
            .map(|t| view.water_depth(t))
            .collect();

        // Water fraction and character.
        let wet = depth.iter().filter(|&&d| d > 0).count() as i64;
        intel.water_permille = wet * 1000 / n as i64;
        intel.character = if intel.water_permille < 120 {
            MapCharacter::Land
        } else if intel.water_permille < 400 {
            MapCharacter::Mixed
        } else {
            MapCharacter::Water
        };

        // Water bodies: 4-connected submerged tiles.
        let (labels, count) = label(w, h, |i| depth.get(i).is_some_and(|&d| d > 0), false);
        intel.water_body = labels;
        intel.water_bodies = count;

        // Shipyard sites.
        let ok = Prefix::new(w, h, |i| {
            depth
                .get(i)
                .is_some_and(|&d| (SHIPYARD_MIN_DEPTH..=SHIPYARD_MAX_DEPTH).contains(&d))
        });
        let deep = Prefix::new(w, h, |i| {
            depth.get(i).is_some_and(|&d| d >= SHIPYARD_MIN_DEPTH)
        });
        for z in 0..=(h - SHIPYARD_FP) {
            for x in 0..=(w - SHIPYARD_FP) {
                if !ok.all(x, z, SHIPYARD_FP, SHIPYARD_FP) {
                    continue;
                }
                let r = SHIPYARD_RING;
                let open = deep.all(x - r, z - r, SHIPYARD_FP + 2 * r, SHIPYARD_FP + 2 * r);
                intel.shipyard_sites.push(ShipyardSite {
                    tile: Tile::new(x, z),
                    open,
                });
            }
        }

        // Deposits: patch tiles, 8-connected.
        let mut patch = vec![0i32; n];
        for (t, m) in view.metal_spots() {
            if let Some(i) = intel.index(t) {
                if let Some(p) = patch.get_mut(i) {
                    *p = m.max(1);
                }
            }
        }
        let (dl, dcount) = label(w, h, |i| patch.get(i).is_some_and(|&m| m > 0), true);
        let mut deposits: Vec<Deposit> = (0..dcount)
            .map(|id| Deposit {
                id: id as usize,
                tiles: Vec::new(),
                metal: 0,
                center: Pos::default(),
            })
            .collect();
        let mut moments: Vec<(i64, i64)> = vec![(0, 0); deposits.len()];
        for z in 0..h {
            for x in 0..w {
                let t = Tile::new(x, z);
                let Some(i) = intel.index(t) else { continue };
                let l = dl.get(i).copied().unwrap_or(0);
                if l == 0 {
                    continue;
                }
                let d = (l - 1) as usize;
                let m = i64::from(patch.get(i).copied().unwrap_or(0));
                if let (Some(dep), Some(mo)) = (deposits.get_mut(d), moments.get_mut(d)) {
                    dep.tiles.push(t);
                    dep.metal += m;
                    let c = t.center();
                    mo.0 += m * i64::from(c.x);
                    mo.1 += m * i64::from(c.z);
                }
            }
        }
        for (dep, mo) in deposits.iter_mut().zip(&moments) {
            if dep.metal > 0 {
                dep.center = Pos::new(
                    i32::try_from(mo.0 / dep.metal).unwrap_or(0),
                    i32::try_from(mo.1 / dep.metal).unwrap_or(0),
                );
            }
        }
        intel.deposits = deposits;
        intel.deposit_of = dl;
        intel
    }

    fn index(&self, t: Tile) -> Option<usize> {
        tile_index(self.width_tiles, self.height_tiles, t)
    }

    /// The water body of a tile (`None` on dry land or off the map).
    pub fn water_body(&self, t: Tile) -> Option<u32> {
        let l = *self.water_body.get(self.index(t)?)?;
        (l > 0).then(|| l - 1)
    }

    /// The deposit a tile belongs to.
    pub fn deposit_at(&self, t: Tile) -> Option<&Deposit> {
        let l = *self.deposit_of.get(self.index(t)?)?;
        if l == 0 {
            None
        } else {
            self.deposits.get((l - 1) as usize)
        }
    }

    /// The shipyard site nearest `p` (ties in scan order).
    pub fn shipyard_site_near(&self, p: Pos) -> Option<&ShipyardSite> {
        self.shipyard_sites
            .iter()
            .min_by_key(|s| s.center().dist2(p))
    }
}

/// Scan-order index of a tile.
pub(crate) fn tile_index(w: i32, h: i32, t: Tile) -> Option<usize> {
    if t.x < 0 || t.z < 0 || t.x >= w || t.z >= h {
        return None;
    }
    usize::try_from(t.z * w + t.x).ok()
}

/// Labels connected regions of `inside` tiles, 4- or 8-connected. Labels are
/// 1-based in scan order of each region's first tile; 0 is outside.
pub(crate) fn label(
    w: i32,
    h: i32,
    inside: impl Fn(usize) -> bool,
    eight: bool,
) -> (Vec<u32>, u32) {
    let n = usize::try_from(w).unwrap_or(0) * usize::try_from(h).unwrap_or(0);
    let mut labels = vec![0u32; n];
    let mut next = 0u32;
    let mut stack: Vec<(i32, i32)> = Vec::new();
    const FOUR: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    const EIGHT: [(i32, i32); 8] = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (1, -1),
        (-1, 1),
        (-1, -1),
    ];
    let dirs: &[(i32, i32)] = if eight { &EIGHT } else { &FOUR };
    for z in 0..h {
        for x in 0..w {
            let Some(i) = tile_index(w, h, Tile::new(x, z)) else {
                continue;
            };
            if labels.get(i).copied() != Some(0) || !inside(i) {
                continue;
            }
            next += 1;
            if let Some(l) = labels.get_mut(i) {
                *l = next;
            }
            stack.push((x, z));
            while let Some((cx, cz)) = stack.pop() {
                for &(dx, dz) in dirs {
                    let (nx, nz) = (cx + dx, cz + dz);
                    let Some(j) = tile_index(w, h, Tile::new(nx, nz)) else {
                        continue;
                    };
                    if labels.get(j).copied() == Some(0) && inside(j) {
                        if let Some(l) = labels.get_mut(j) {
                            *l = next;
                        }
                        stack.push((nx, nz));
                    }
                }
            }
        }
    }
    (labels, next)
}

/// A 2-D prefix count of tiles satisfying a test, for O(1) "all tiles in
/// this rectangle" queries. Off-map tiles never satisfy it.
pub(crate) struct Prefix {
    w: i32,
    h: i32,
    sums: Vec<u32>,
}

impl Prefix {
    pub(crate) fn new(w: i32, h: i32, test: impl Fn(usize) -> bool) -> Prefix {
        let (pw, ph) = (
            usize::try_from(w + 1).unwrap_or(1),
            usize::try_from(h + 1).unwrap_or(1),
        );
        let mut sums = vec![0u32; pw * ph];
        for z in 0..h {
            for x in 0..w {
                let v = tile_index(w, h, Tile::new(x, z)).is_some_and(&test);
                let (ux, uz) = (x as usize, z as usize);
                let get =
                    |s: &Vec<u32>, a: usize, b: usize| s.get(b * pw + a).copied().unwrap_or(0);
                let val = u32::from(v) + get(&sums, ux, uz + 1) + get(&sums, ux + 1, uz)
                    - get(&sums, ux, uz);
                if let Some(s) = sums.get_mut((uz + 1) * pw + ux + 1) {
                    *s = val;
                }
            }
        }
        Prefix { w, h, sums }
    }

    /// Whether every tile of the `rw × rh` rectangle at (x, z) satisfies the
    /// test (false if any part is off the map).
    pub(crate) fn all(&self, x: i32, z: i32, rw: i32, rh: i32) -> bool {
        if x < 0 || z < 0 || rw <= 0 || rh <= 0 || x + rw > self.w || z + rh > self.h {
            return false;
        }
        let pw = (self.w + 1) as usize;
        let at = |a: i32, b: i32| {
            self.sums
                .get(b as usize * pw + a as usize)
                .copied()
                .unwrap_or(0)
        };
        let total = at(x + rw, z + rh) + at(x, z) - at(x + rw, z) - at(x, z + rh);
        total == (rw * rh) as u32
    }
}

/// One builder's claim on an extractor site (SPEC §4.5 "claimed").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    pub builder: UnitId,
    pub at: Pos,
    pub def: DefId,
    /// The tick the claim was noted (this tick for claims read off queues).
    pub tick: Tick,
}

/// Extractor-site claims: every extractor build order a builder holds, in
/// its queue or in flight, plus claims the economy notes while staging this
/// tick. Rebuilt every tick in builder id order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetalClaims {
    claims: Vec<Claim>,
}

impl MetalClaims {
    pub(crate) fn rebuild<'a>(
        &mut self,
        view: &dyn GameView,
        units: impl Iterator<Item = &'a OwnUnit>,
        in_flight: impl Iterator<Item = &'a crate::view::Order>,
        tick: Tick,
    ) {
        let is_mex = |d: DefId| view.unit_def(d).is_some_and(|d| d.extracts_metal > 0);
        let mut claims: BTreeMap<(UnitId, i32, i32), Claim> = BTreeMap::new();
        let mut add = |builder: UnitId, c: &Command| {
            if let Command::Build { def, at } = *c {
                if is_mex(def) {
                    claims.insert(
                        (builder, at.x, at.z),
                        Claim {
                            builder,
                            at,
                            def,
                            tick,
                        },
                    );
                }
            }
        };
        for u in units {
            for c in &u.commands {
                add(u.id, c);
            }
        }
        for o in in_flight {
            add(o.unit, &o.command);
        }
        self.claims = claims.into_values().collect();
    }

    /// Notes a claim staged this tick (call when staging an extractor build).
    pub fn note(&mut self, builder: UnitId, at: Pos, def: DefId, tick: Tick) {
        self.claims.push(Claim {
            builder,
            at,
            def,
            tick,
        });
    }

    /// Every claim, in builder id order (noted claims last).
    pub fn iter(&self) -> impl Iterator<Item = &Claim> + '_ {
        self.claims.iter()
    }

    /// Whether a builder other than `except` claims a site within `radius`
    /// of `at` (`build.claim_radius`).
    pub fn claimed_near(&self, at: Pos, radius: i64, except: Option<UnitId>) -> bool {
        self.claims
            .iter()
            .any(|c| Some(c.builder) != except && c.at.within(at, radius))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_prefix() {
        // 4 × 3 grid:
        // X X . X
        // . X . .
        // . . . X
        let cells = [1, 1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1];
        let inside = |i: usize| cells.get(i) == Some(&1);
        let (l4, n4) = label(4, 3, inside, false);
        assert_eq!(n4, 3);
        assert_eq!(l4, vec![1, 1, 0, 2, 0, 1, 0, 0, 0, 0, 0, 3]);
        let (_, n8) = label(4, 3, inside, true);
        assert_eq!(n8, 3);
        let p = Prefix::new(4, 3, inside);
        assert!(p.all(0, 0, 2, 1));
        assert!(!p.all(0, 0, 2, 2));
        assert!(p.all(3, 2, 1, 1));
        assert!(!p.all(3, 2, 2, 1));
        assert!(!p.all(-1, 0, 1, 1));
    }
}
