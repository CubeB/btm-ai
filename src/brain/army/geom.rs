//! Integer geometry for the army passes (SPEC §7: no floats in order paths).
//!
//! Bearings are a fixed table of cosines and sines in thousandths, so every
//! peer computes the same points.

use crate::view::{GameView, Pos, Tile, TILE_SIZE};

/// cos and sin × 1000 of k × 22.5°, for k = 0..16, counter-clockwise from +x.
pub const BEARINGS16: [(i64, i64); 16] = [
    (1000, 0),
    (924, 383),
    (707, 707),
    (383, 924),
    (0, 1000),
    (-383, 924),
    (-707, 707),
    (-924, 383),
    (-1000, 0),
    (-924, -383),
    (-707, -707),
    (-383, -924),
    (0, -1000),
    (383, -924),
    (707, -707),
    (924, -383),
];

/// A position from wide coordinates, saturating.
pub fn pos(x: i64, z: i64) -> Pos {
    let c = |v: i64| i32::try_from(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0);
    Pos::new(c(x), c(z))
}

/// The point `r` from `p` on bearing `k` of 16.
pub fn on_bearing(p: Pos, k: usize, r: i64) -> Pos {
    let (c, s) = BEARINGS16.get(k % 16).copied().unwrap_or((1000, 0));
    pos(i64::from(p.x) + c * r / 1000, i64::from(p.z) + s * r / 1000)
}

/// The centroid of some points, `None` for none.
pub fn centroid(points: impl IntoIterator<Item = Pos>) -> Option<Pos> {
    let (mut x, mut z, mut n) = (0i64, 0i64, 0i64);
    for p in points {
        x += i64::from(p.x);
        z += i64::from(p.z);
        n += 1;
    }
    (n > 0).then(|| pos(x / n, z / n))
}

/// Clamps a point to `margin` inside the map.
pub fn inside(view: &dyn GameView, p: Pos, margin: i64) -> Pos {
    let m = view.map();
    let (w, h) = (i64::from(m.width()), i64::from(m.height()));
    let cx = i64::from(p.x).clamp(margin.min(w / 2), (w - margin).max(w / 2));
    let cz = i64::from(p.z).clamp(margin.min(h / 2), (h - margin).max(h / 2));
    pos(cx, cz)
}

/// The point reached from `from` by going `forward` towards `to` and
/// `across` to the left of that line (negative: to the right).
pub fn sidestep(from: Pos, to: Pos, across: i64, forward: i64) -> Pos {
    let len = from.dist(to);
    if len == 0 {
        return from;
    }
    let dx = i64::from(to.x) - i64::from(from.x);
    let dz = i64::from(to.z) - i64::from(from.z);
    pos(
        i64::from(from.x) + (dx * forward - dz * across) / len,
        i64::from(from.z) + (dz * forward + dx * across) / len,
    )
}

/// How far `u` stands behind `p` as seen from `threat`: its offset from `p`
/// along the direction threat → p (negative: in front, nearer the threat).
pub fn behind(p: Pos, threat: Pos, u: Pos) -> i64 {
    let len = threat.dist(p);
    if len == 0 {
        return 0;
    }
    let vx = i64::from(p.x) - i64::from(threat.x);
    let vz = i64::from(p.z) - i64::from(threat.z);
    let ux = i64::from(u.x) - i64::from(p.x);
    let uz = i64::from(u.z) - i64::from(p.z);
    (ux * vx + uz * vz) / len
}

/// Whether `q` is within `r` of the segment `a`-`b`.
pub fn near_segment(q: Pos, a: Pos, b: Pos, r: i64) -> bool {
    let (ax, az) = (i64::from(a.x), i64::from(a.z));
    let (dx, dz) = (i64::from(b.x) - ax, i64::from(b.z) - az);
    let (qx, qz) = (i64::from(q.x) - ax, i64::from(q.z) - az);
    let len2 = dx * dx + dz * dz;
    if len2 == 0 {
        return q.within(a, r);
    }
    let t = (qx * dx + qz * dz).clamp(0, len2);
    // The closest point is a + d × t / len2; compare squared distances
    // scaled by len2² to stay exact.
    let ex = i128::from(qx) * i128::from(len2) - i128::from(dx) * i128::from(t);
    let ez = i128::from(qz) * i128::from(len2) - i128::from(dz) * i128::from(t);
    let lhs = ex * ex + ez * ez;
    let rr = i128::from(r) * i128::from(len2);
    lhs <= rr * rr
}

/// Dry land: no water over the tile.
pub fn dry(view: &dyn GameView, p: Pos) -> bool {
    view.water_depth(p.tile()) == 0
}

/// Dry throughout, sampled once per tile along the way.
pub fn dry_path(view: &dyn GameView, a: Pos, b: Pos) -> bool {
    let n = (a.dist(b) / i64::from(TILE_SIZE)).max(1);
    (0..=n).all(|i| dry(view, lerp(a, b, i, n)))
}

/// The point `i / n` of the way from `a` to `b`.
pub fn lerp(a: Pos, b: Pos, i: i64, n: i64) -> Pos {
    let n = n.max(1);
    pos(
        i64::from(a.x) + (i64::from(b.x) - i64::from(a.x)) * i / n,
        i64::from(a.z) + (i64::from(b.z) - i64::from(a.z)) * i / n,
    )
}

/// Ground height under a point.
pub fn ground(view: &dyn GameView, p: Pos) -> i64 {
    let t: Tile = p.tile();
    i64::from(view.height(t))
}

/// The blocked-shot line test (SPEC §5.12): the line between the two muzzle
/// points, `MUZZLE_HEIGHT` above the ground, sampled about once per tile
/// (between the sample bounds); blocked if the ground rises more than
/// `LINE_CLEARANCE` above it.
pub fn line_clear(view: &dyn GameView, a: Pos, b: Pos) -> bool {
    use super::{LINE_CLEARANCE, LINE_SAMPLES_MAX, LINE_SAMPLES_MIN, MUZZLE_HEIGHT};
    let n = (a.dist(b) / i64::from(TILE_SIZE)).clamp(LINE_SAMPLES_MIN, LINE_SAMPLES_MAX);
    let ha = ground(view, a) + MUZZLE_HEIGHT;
    let hb = ground(view, b) + MUZZLE_HEIGHT;
    (1..n).all(|i| {
        let line = ha + (hb - ha) * i / n;
        ground(view, lerp(a, b, i, n)) <= line + LINE_CLEARANCE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearings_and_steps_are_exact() {
        let o = Pos::new(1000, 1000);
        assert_eq!(on_bearing(o, 0, 100), Pos::new(1100, 1000));
        assert_eq!(on_bearing(o, 4, 100), Pos::new(1000, 1100));
        assert_eq!(on_bearing(o, 8, 100), Pos::new(900, 1000));
        assert_eq!(on_bearing(o, 2, 1000), Pos::new(1707, 1707));
        // Left of the +x line is +z.
        assert_eq!(
            sidestep(o, Pos::new(2000, 1000), 240, 120),
            Pos::new(1120, 1240)
        );
        assert_eq!(
            sidestep(o, Pos::new(2000, 1000), -240, 120),
            Pos::new(1120, 760)
        );
        assert_eq!(behind(o, Pos::new(0, 1000), Pos::new(1100, 1300)), 100);
        assert_eq!(behind(o, Pos::new(0, 1000), Pos::new(900, 1000)), -100);
        assert!(near_segment(
            Pos::new(50, 40),
            Pos::new(0, 0),
            Pos::new(100, 0),
            48
        ));
        assert!(!near_segment(
            Pos::new(50, 50),
            Pos::new(0, 0),
            Pos::new(100, 0),
            48
        ));
        assert!(!near_segment(
            Pos::new(160, 0),
            Pos::new(0, 0),
            Pos::new(100, 0),
            48
        ));
        assert_eq!(
            centroid([Pos::new(0, 0), Pos::new(10, 20)]),
            Some(Pos::new(5, 10))
        );
        assert_eq!(centroid([]), None);
    }
}
