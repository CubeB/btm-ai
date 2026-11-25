//! Value types: ids, time, positions and resource amounts (SPEC conventions, §7).

use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// Simulation ticks per second (SPEC conventions: "N s" means N × 30 ticks).
pub const TICKS_PER_SECOND: u32 = 30;
/// Side of a heightmap tile in world units (SPEC conventions).
pub const TILE_SIZE: i32 = 16;
/// Side of a fog/vision cell in world units, 2 × 2 tiles (SPEC conventions).
pub const VIS_CELL_SIZE: i32 = 32;

/// A simulation tick count. Wall-clock time is never used (SPEC §2.2, §7).
pub type Tick = u32;

/// Converts seconds of game time to ticks (SPEC conventions).
pub const fn secs(s: u32) -> Tick {
    s * TICKS_PER_SECOND
}

/// A unit's id. Ids are never reused; iteration is in id order (SPEC §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(pub u32);

/// A feature's id (wreck, rock, tree). Iteration is in id order (SPEC §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeatureId(pub u32);

/// Index of a unit definition in [`crate::view::GameView::unit_defs`] (SPEC §1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefId(pub u16);

/// A player (seat). Iteration is in player id order (SPEC §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u8);

/// An alliance team. Players with the same team are allies (SPEC §1.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TeamId(pub u8);

/// A flat x/z position in integer world units (SPEC conventions, §7).
///
/// The game's fixed-point scalar is truncated to whole world units at the
/// view boundary: no AI decision in the spec needs sub-unit precision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub x: i32,
    pub z: i32,
}

impl Pos {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// Squared flat distance (exact).
    pub fn dist2(self, o: Pos) -> i64 {
        let dx = i64::from(self.x) - i64::from(o.x);
        let dz = i64::from(self.z) - i64::from(o.z);
        dx * dx + dz * dz
    }

    /// Flat distance, rounded down (integer square root).
    pub fn dist(self, o: Pos) -> i64 {
        // dist2 is never negative.
        self.dist2(o).unsigned_abs().isqrt() as i64
    }

    /// True when `o` is within `r` (inclusive) of this position.
    pub fn within(self, o: Pos, r: i64) -> bool {
        self.dist2(o) <= r.saturating_mul(r)
    }

    /// The point `d` units from here towards `to` (beyond it if `d` is
    /// longer than the gap; away from it if `d` is negative). Returns `self`
    /// when the two points coincide.
    pub fn toward(self, to: Pos, d: i64) -> Pos {
        let len = self.dist(to);
        if len == 0 {
            return self;
        }
        let dx = i64::from(to.x) - i64::from(self.x);
        let dz = i64::from(to.z) - i64::from(self.z);
        Pos::new(
            sat_i32(i64::from(self.x) + dx * d / len),
            sat_i32(i64::from(self.z) + dz * d / len),
        )
    }

    /// The heightmap tile containing this position.
    pub fn tile(self) -> Tile {
        Tile::new(self.x.div_euclid(TILE_SIZE), self.z.div_euclid(TILE_SIZE))
    }

    /// The vision cell containing this position.
    pub fn vis_cell(self) -> VisCell {
        VisCell {
            x: self.x.div_euclid(VIS_CELL_SIZE),
            z: self.z.div_euclid(VIS_CELL_SIZE),
        }
    }
}

fn sat_i32(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// A heightmap tile coordinate, 16 world units square (SPEC conventions).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tile {
    pub x: i32,
    pub z: i32,
}

impl Tile {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    /// The world position of the tile's top-left corner.
    pub fn corner(self) -> Pos {
        Pos::new(self.x * TILE_SIZE, self.z * TILE_SIZE)
    }

    /// The world position of the tile's centre.
    pub fn center(self) -> Pos {
        Pos::new(
            self.x * TILE_SIZE + TILE_SIZE / 2,
            self.z * TILE_SIZE + TILE_SIZE / 2,
        )
    }
}

/// A vision cell coordinate, 32 world units square (SPEC §1.2, §3.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisCell {
    pub x: i32,
    pub z: i32,
}

impl VisCell {
    /// The world position of the cell's centre.
    pub fn center(self) -> Pos {
        Pos::new(
            self.x * VIS_CELL_SIZE + VIS_CELL_SIZE / 2,
            self.z * VIS_CELL_SIZE + VIS_CELL_SIZE / 2,
        )
    }
}

/// A quantity of metal or energy, or a rate per second, in milli-units
/// (SPEC §1.2 economy queries, §7 integer arithmetic).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Amount(pub i64);

impl Amount {
    pub const ZERO: Amount = Amount(0);

    /// Whole units, e.g. `Amount::units(50)` is 50 metal.
    pub const fn units(n: i64) -> Self {
        Amount(n * 1000)
    }

    /// Milli-units.
    pub const fn milli(self) -> i64 {
        self.0
    }

    /// Whole units, rounded towards zero.
    pub const fn whole(self) -> i64 {
        self.0 / 1000
    }
}

impl Add for Amount {
    type Output = Amount;
    fn add(self, o: Amount) -> Amount {
        Amount(self.0.saturating_add(o.0))
    }
}

impl Sub for Amount {
    type Output = Amount;
    fn sub(self, o: Amount) -> Amount {
        Amount(self.0.saturating_sub(o.0))
    }
}

impl AddAssign for Amount {
    fn add_assign(&mut self, o: Amount) {
        *self = *self + o;
    }
}

impl SubAssign for Amount {
    fn sub_assign(&mut self, o: Amount) {
        *self = *self - o;
    }
}

impl Neg for Amount {
    type Output = Amount;
    fn neg(self) -> Amount {
        Amount(self.0.saturating_neg())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_integer_and_exact() {
        let a = Pos::new(0, 0);
        let b = Pos::new(30, 40);
        assert_eq!(a.dist2(b), 2500);
        assert_eq!(a.dist(b), 50);
        assert!(a.within(b, 50));
        assert!(!a.within(b, 49));
        assert_eq!(a.toward(b, 10), Pos::new(6, 8));
        assert_eq!(a.toward(b, -10), Pos::new(-6, -8));
        assert_eq!(a.toward(a, 10), a);
    }

    #[test]
    fn grid_conversions() {
        assert_eq!(Pos::new(-1, 33).tile(), Tile::new(-1, 2));
        assert_eq!(Pos::new(63, 64).vis_cell(), VisCell { x: 1, z: 2 });
        assert_eq!(Tile::new(2, 3).center(), Pos::new(40, 56));
        assert_eq!(secs(2), 60);
        assert_eq!(Amount::units(3) - Amount(500), Amount(2500));
    }
}
