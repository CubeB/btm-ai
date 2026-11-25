//! A tile grid map: heights, water, metal, impassable cells, starts and vents.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::SimError;
use crate::view::{MapInfo, MoveClass, Pos, Tile};

/// A rectangle of tiles, `x0..x1` by `z0..z1` (end exclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRect {
    pub x0: i32,
    pub z0: i32,
    pub x1: i32,
    pub z1: i32,
}

impl TileRect {
    pub const fn new(x0: i32, z0: i32, x1: i32, z1: i32) -> Self {
        Self { x0, z0, x1, z1 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    info: MapInfo,
    height: Vec<i32>,
    metal: Vec<i32>,
    blocked: Vec<bool>,
    starts: Vec<Pos>,
    vents: Vec<Pos>,
}

impl Map {
    pub fn builder(width_tiles: i32, height_tiles: i32) -> MapBuilder {
        let n = usize::try_from((width_tiles.max(0)) * (height_tiles.max(0))).unwrap_or(0);
        MapBuilder {
            map: Map {
                info: MapInfo {
                    width_tiles,
                    height_tiles,
                    sea_level: 0,
                    surface_metal: 0,
                },
                height: vec![10; n],
                metal: vec![0; n],
                blocked: vec![false; n],
                starts: Vec::new(),
                vents: Vec::new(),
            },
        }
    }

    /// A 64 × 64 tile (1024 × 1024) two-player map, point-symmetric: four
    /// metal spots per side, one in the middle, a lake per side, a ridge of
    /// impassable cells per side and a vent per side.
    pub fn two_player() -> Map {
        let mut b = Map::builder(64, 64);
        let mirror = |r: TileRect| TileRect::new(64 - r.x1, 64 - r.z1, 64 - r.x0, 64 - r.z0);
        for spot in [(6, 13), (13, 6), (16, 16), (4, 24)] {
            let r = TileRect::new(spot.0, spot.1, spot.0 + 2, spot.1 + 2);
            b = b.metal(r, 500).metal(mirror(r), 500);
        }
        b = b.metal(TileRect::new(31, 31, 33, 33), 800);
        let lake = TileRect::new(40, 6, 54, 18);
        b = b.water(lake, 40).water(mirror(lake), 40);
        let ridge = TileRect::new(22, 30, 25, 40);
        b = b.block(ridge).block(mirror(ridge));
        b.start(Pos::new(160, 160))
            .start(Pos::new(864, 864))
            .vent(Pos::new(328, 72))
            .vent(Pos::new(696, 952))
            .build()
            .unwrap_or_else(|_| Map::builder(1, 1).map)
    }

    pub fn info(&self) -> MapInfo {
        self.info
    }

    pub fn starts(&self) -> &[Pos] {
        &self.starts
    }

    pub fn vents(&self) -> &[Pos] {
        &self.vents
    }

    fn idx(&self, t: Tile) -> Option<usize> {
        if self.info.contains_tile(t) {
            usize::try_from(t.z * self.info.width_tiles + t.x).ok()
        } else {
            None
        }
    }

    pub fn height(&self, t: Tile) -> i32 {
        self.idx(t).map_or(0, |i| self.height[i])
    }

    pub fn metal(&self, t: Tile) -> i32 {
        self.idx(t).map_or(0, |i| self.metal[i])
    }

    pub fn depth(&self, t: Tile) -> i32 {
        (self.info.sea_level - self.height(t)).max(0)
    }

    /// Largest height step to an on-map 4-neighbour.
    pub fn slope(&self, t: Tile) -> i32 {
        let h = self.height(t);
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .map(|&(dx, dz)| Tile::new(t.x + dx, t.z + dz))
            .filter(|n| self.info.contains_tile(*n))
            .map(|n| (self.height(n) - h).abs())
            .max()
            .unwrap_or(0)
    }

    pub fn standable(&self, t: Tile, mc: MoveClass) -> bool {
        match self.idx(t) {
            Some(i) if !self.blocked[i] => mc.accepts(self.slope(t), self.depth(t)),
            _ => false,
        }
    }

    pub fn metal_spots(&self) -> Vec<(Tile, i32)> {
        let w = self.info.width_tiles;
        let mut out = Vec::new();
        for (i, &m) in self.metal.iter().enumerate() {
            if m > self.info.surface_metal {
                let i = i32::try_from(i).unwrap_or(0);
                out.push((Tile::new(i % w, i / w), m));
            }
        }
        out
    }

    /// An 8-way A* path for a movement class from `from` towards `goal`.
    /// Returns waypoints (tile centres, the last replaced by `goal` when it
    /// is reached exactly), first waypoint first. If the goal cannot be
    /// reached the path ends at the explored tile nearest to it.
    pub fn path(&self, from: Pos, goal: Pos, mc: MoveClass) -> Vec<Pos> {
        let (Some(start), Some(end)) = (self.idx(from.tile()), self.idx(goal.tile())) else {
            return Vec::new();
        };
        if start == end {
            return vec![goal];
        }
        let w = self.info.width_tiles;
        let n = self.height.len();
        let tile_of = |i: usize| {
            let i = i32::try_from(i).unwrap_or(0);
            Tile::new(i % w, i / w)
        };
        let goal_tile = goal.tile();
        let h = |t: Tile| -> u32 {
            let dx = (t.x - goal_tile.x).unsigned_abs();
            let dz = (t.z - goal_tile.z).unsigned_abs();
            10 * dx.max(dz) + 4 * dx.min(dz)
        };
        let mut g = vec![u32::MAX; n];
        let mut came = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        g[start] = 0;
        open.push(Reverse((h(tile_of(start)), start)));
        let mut best = (h(tile_of(start)), start);
        while let Some(Reverse((f, cur))) = open.pop() {
            if cur == end {
                best = (0, cur);
                break;
            }
            let ct = tile_of(cur);
            let hc = h(ct);
            if f > g[cur].saturating_add(hc) {
                continue; // stale heap entry
            }
            if hc < best.0 || (hc == best.0 && cur < best.1) {
                best = (hc, cur);
            }
            for (dx, dz) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let nt = Tile::new(ct.x + dx, ct.z + dz);
                let Some(ni) = self.idx(nt) else { continue };
                if !self.standable(nt, mc) {
                    continue;
                }
                let diag = dx != 0 && dz != 0;
                if diag
                    && !(self.standable(Tile::new(ct.x + dx, ct.z), mc)
                        && self.standable(Tile::new(ct.x, ct.z + dz), mc))
                {
                    continue;
                }
                let ng = g[cur].saturating_add(if diag { 14 } else { 10 });
                if ng < g[ni] {
                    g[ni] = ng;
                    came[ni] = cur;
                    open.push(Reverse((ng.saturating_add(h(nt)), ni)));
                }
            }
        }
        let mut out = Vec::new();
        let mut at = best.1;
        while at != start && at != usize::MAX {
            out.push(tile_of(at).center());
            at = came[at];
        }
        out.reverse();
        if best.1 == end {
            if let Some(last) = out.last_mut() {
                *last = goal;
            }
        }
        out
    }
}

/// Builds a [`Map`]. Tiles start at height 10 above a sea level of 0.
#[derive(Clone, Debug)]
pub struct MapBuilder {
    map: Map,
}

impl MapBuilder {
    fn each(mut self, r: TileRect, f: impl Fn(&mut Map, usize)) -> Self {
        for z in r.z0..r.z1 {
            for x in r.x0..r.x1 {
                if let Some(i) = self.map.idx(Tile::new(x, z)) {
                    f(&mut self.map, i);
                }
            }
        }
        self
    }

    pub fn ground(self, r: TileRect, h: i32) -> Self {
        self.each(r, |m, i| m.height[i] = h)
    }

    /// Water `depth` deep (ground at sea level − depth).
    pub fn water(self, r: TileRect, depth: i32) -> Self {
        self.each(r, |m, i| m.height[i] = m.info.sea_level - depth)
    }

    /// Impassable for every movement class (cliffs, rocks).
    pub fn block(self, r: TileRect) -> Self {
        self.each(r, |m, i| m.blocked[i] = true)
    }

    /// Metal per tile, milli-metal per second at full extraction.
    pub fn metal(self, r: TileRect, value: i32) -> Self {
        self.each(r, |m, i| m.metal[i] = value)
    }

    pub fn surface_metal(mut self, v: i32) -> Self {
        self.map.info.surface_metal = v;
        self
    }

    pub fn start(mut self, p: Pos) -> Self {
        self.map.starts.push(p);
        self
    }

    pub fn vent(mut self, p: Pos) -> Self {
        self.map.vents.push(p);
        self
    }

    pub fn build(self) -> Result<Map, SimError> {
        let m = self.map;
        if m.info.width_tiles <= 0 || m.info.height_tiles <= 0 {
            return Err(SimError::BadMap("empty map"));
        }
        if m.starts
            .iter()
            .chain(&m.vents)
            .any(|p| !m.info.contains(*p))
        {
            return Err(SimError::BadMap("start or vent off the map"));
        }
        Ok(m)
    }
}
