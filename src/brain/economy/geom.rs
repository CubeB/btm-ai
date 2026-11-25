//! Integer geometry for siting: footprint rectangles, clearance gaps, square
//! rings, and directions scaled to 1000 (SPEC §4.6, §7: no floats).

use crate::view::{footprint_origin, Pos, Tile, TILE_SIZE};

/// Length of a direction vector.
pub const UNIT: i64 = 1000;

/// A footprint in tiles: `x0..x1` by `z0..z1`, end exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: i32,
    pub z0: i32,
    pub x1: i32,
    pub z1: i32,
}

impl Rect {
    /// The footprint of `fp` tiles centred at `at`, snapped as the game does.
    pub fn of(fp: (i32, i32), at: Pos) -> Rect {
        let o = footprint_origin(fp, at);
        Rect {
            x0: o.x,
            z0: o.z,
            x1: o.x + fp.0.max(1),
            z1: o.z + fp.1.max(1),
        }
    }

    /// Chebyshev separation in tiles; negative or zero when they touch or
    /// overlap (0 means edge to edge).
    pub fn gap(&self, o: &Rect) -> i32 {
        (o.x0 - self.x1)
            .max(self.x0 - o.x1)
            .max(o.z0 - self.z1)
            .max(self.z0 - o.z1)
    }

    /// The footprints share a tile.
    pub fn overlaps(&self, o: &Rect) -> bool {
        self.gap(o) < 0
    }

    /// Every tile, in scan order.
    pub fn tiles(&self) -> impl Iterator<Item = Tile> + '_ {
        (self.z0..self.z1).flat_map(move |z| (self.x0..self.x1).map(move |x| Tile::new(x, z)))
    }

    /// Whether a world point lies inside.
    pub fn contains(&self, p: Pos) -> bool {
        let t = p.tile();
        t.x >= self.x0 && t.x < self.x1 && t.z >= self.z0 && t.z < self.z1
    }

    /// The world centre of the rectangle.
    pub fn center(&self) -> Pos {
        Pos::new(
            (self.x0 + self.x1) * TILE_SIZE / 2,
            (self.z0 + self.z1) * TILE_SIZE / 2,
        )
    }
}

/// The square ring `r` round the origin, ordered by row, then column
/// (SPEC §4.5, §4.6 ring order). Ring 0 is the origin itself.
pub fn ring(r: i32) -> Vec<(i32, i32)> {
    if r <= 0 {
        return vec![(0, 0)];
    }
    let mut out = Vec::with_capacity(usize::try_from(8 * r).unwrap_or(0));
    for dz in -r..=r {
        for dx in -r..=r {
            if dx.abs() == r || dz.abs() == r {
                out.push((dx, dz));
            }
        }
    }
    out
}

/// The direction from `from` to `to`, scaled to length [`UNIT`]; zero when
/// the points coincide.
pub fn dir(from: Pos, to: Pos) -> (i64, i64) {
    let dx = i64::from(to.x) - i64::from(from.x);
    let dz = i64::from(to.z) - i64::from(from.z);
    norm((dx, dz))
}

/// A vector scaled to length [`UNIT`]; zero stays zero.
pub fn norm(v: (i64, i64)) -> (i64, i64) {
    let len = (v.0 * v.0 + v.1 * v.1).unsigned_abs().isqrt() as i64;
    if len == 0 {
        (0, 0)
    } else {
        (v.0 * UNIT / len, v.1 * UNIT / len)
    }
}

/// `p` moved `d` world units along a [`UNIT`]-length direction.
pub fn along(p: Pos, dir: (i64, i64), d: i64) -> Pos {
    Pos::new(
        sat(i64::from(p.x) + dir.0 * d / UNIT),
        sat(i64::from(p.z) + dir.1 * d / UNIT),
    )
}

/// Signed distance of `p − origin` along a [`UNIT`]-length direction.
pub fn forward(origin: Pos, dir: (i64, i64), p: Pos) -> i64 {
    let dx = i64::from(p.x) - i64::from(origin.x);
    let dz = i64::from(p.z) - i64::from(origin.z);
    (dx * dir.0 + dz * dir.1) / UNIT
}

/// The cosine of the angle between two vectors, in permille (0 if either is
/// zero).
pub fn cos_permille(a: (i64, i64), b: (i64, i64)) -> i64 {
    let la = (a.0 * a.0 + a.1 * a.1).unsigned_abs().isqrt() as i64;
    let lb = (b.0 * b.0 + b.1 * b.1).unsigned_abs().isqrt() as i64;
    if la == 0 || lb == 0 {
        return 0;
    }
    let dot = i128::from(a.0) * i128::from(b.0) + i128::from(a.1) * i128::from(b.1);
    i64::try_from(dot * 1000 / (i128::from(la) * i128::from(lb))).unwrap_or(0)
}

/// Whether two vectors are within 45° of each other (exact, by squaring).
pub fn within_45(a: (i64, i64), b: (i64, i64)) -> bool {
    let dot = i128::from(a.0) * i128::from(b.0) + i128::from(a.1) * i128::from(b.1);
    let la = i128::from(a.0) * i128::from(a.0) + i128::from(a.1) * i128::from(a.1);
    let lb = i128::from(b.0) * i128::from(b.0) + i128::from(b.1) * i128::from(b.1);
    la > 0 && lb > 0 && dot > 0 && 2 * dot * dot >= la * lb
}

/// The centre direction of each of the 16 threat sectors (SPEC §5.4: sector
/// 0 starts at bearing −180°, 22.5° each), scaled to [`UNIT`]. The bearing is
/// `atan2(dz, dx)`, so the direction is `(cos, sin)` of the centre angle.
pub const SECTOR_DIR: [(i64, i64); 16] = [
    (-981, -195),
    (-831, -556),
    (-556, -831),
    (-195, -981),
    (195, -981),
    (556, -831),
    (831, -556),
    (981, -195),
    (981, 195),
    (831, 556),
    (556, 831),
    (195, 981),
    (-195, 981),
    (-556, 831),
    (-831, 556),
    (-981, 195),
];

fn sat(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::sector_of;

    #[test]
    fn rings_gaps_and_sectors() {
        assert_eq!(ring(0), vec![(0, 0)]);
        let r1 = ring(1);
        assert_eq!(r1.len(), 8);
        assert_eq!(r1[0], (-1, -1));
        assert_eq!(r1[4], (1, 0));
        let a = Rect::of((2, 2), Pos::new(16, 16));
        let b = Rect::of((2, 2), Pos::new(64, 16));
        assert_eq!((a.x0, a.x1), (0, 2));
        assert_eq!(a.gap(&b), 1);
        assert!(!a.overlaps(&b));
        assert!(a.overlaps(&a));
        // Every sector's centre direction falls in that sector.
        let home = Pos::new(5000, 5000);
        for (i, d) in SECTOR_DIR.iter().enumerate() {
            assert_eq!(sector_of(home, along(home, *d, 1000)), i);
        }
        assert!(within_45((1000, 0), (800, 700)));
        assert!(!within_45((1000, 0), (600, 900)));
        assert_eq!(cos_permille((1, 0), (0, 5)), 0);
        assert_eq!(dir(Pos::new(0, 0), Pos::new(30, 40)), (600, 800));
    }
}
