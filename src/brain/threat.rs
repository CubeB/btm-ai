//! The threat picture round home (SPEC §1.3 step 4, §5.4): intruders, the
//! 16-sector threat memory and the radar early warning.
//!
//! The core loop supplies the geometry ([`sector_of`]) and the sector
//! arithmetic ([`SectorMemory`]). The army manager fills every field of
//! [`ThreatPicture`] each tick (its phase step), and the economy manager
//! reads it (tower facing §4.6, urgency §4.2).

use crate::view::{Amount, Pos, Tick, UnitId};

/// Number of fixed threat sectors round home (SPEC §5.4).
pub const SECTORS: usize = 16;

/// The sector of `p` seen from `home`: 16 sectors of 22.5°, sector 0 starting
/// at bearing −180° and the index increasing with angle (SPEC §5.4).
///
/// The bearing is `atan2(dz, dx)`. It is computed exactly in integers, with no
/// arctangent at all: every sector boundary is a multiple of 22.5°, so the
/// vector is turned by quarter turns into the first quadrant and then
/// compared against tan 22.5° = √2 − 1 by squaring, which is exact and
/// identical on every peer (SPEC §7). `p == home` is sector 0.
pub fn sector_of(home: Pos, p: Pos) -> usize {
    let mut x = i64::from(p.x) - i64::from(home.x);
    let mut z = i64::from(p.z) - i64::from(home.z);
    if x == 0 && z == 0 {
        return 0;
    }
    // Rotate by −90° until the vector lies in [0°, 90°): x > 0, z >= 0.
    let mut quarter = 0usize;
    while !(x > 0 && z >= 0) && quarter < 4 {
        let (nx, nz) = (z, -x);
        x = nx;
        z = nz;
        quarter += 1;
    }
    // Here 0 <= θ < 90° with tan θ = z / x.
    let sum = x + z;
    let within = if sum * sum < 2 * x * x {
        0 // θ < 22.5°
    } else if z < x {
        1 // θ < 45°
    } else if sum * sum > 2 * z * z {
        2 // θ < 67.5°
    } else {
        3
    };
    // Bin of the bearing in [0°, 360°), then shifted so that −180° is 0.
    let bin = quarter * 4 + within;
    (bin + SECTORS / 2) % SECTORS
}

/// The decaying 16-sector threat memory (`army.sectors`, SPEC §5.4).
///
/// Values are whole metal. The army manager adds a contact's metal once when
/// it first becomes an intruder, and calls [`SectorMemory::decay`] every tick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectorMemory {
    threat: [i64; SECTORS],
    last_halved: Option<Tick>,
}

impl SectorMemory {
    /// Adds an intruder's metal (rounded, at least 1) to its sector.
    pub fn add(&mut self, sector: usize, metal: Amount) {
        let whole = ((metal.milli() + 500) / 1000).max(1);
        if let Some(s) = self.threat.get_mut(sector % SECTORS) {
            *s = s.saturating_add(whole);
        }
    }

    /// Halves every sector once each `halflife` ticks (`army.sector_halflife_s`).
    /// The clock starts at the first call. A zero half-life never decays.
    pub fn decay(&mut self, now: Tick, halflife: Tick) {
        let last = *self.last_halved.get_or_insert(now);
        if halflife > 0 && now.saturating_sub(last) >= halflife {
            for s in &mut self.threat {
                *s /= 2;
            }
            self.last_halved = Some(now);
        }
    }

    /// One sector's threat.
    pub fn get(&self, sector: usize) -> i64 {
        self.threat.get(sector).copied().unwrap_or(0)
    }

    /// The sum over all sectors (drives the wave recall, SPEC §5.1).
    pub fn total(&self) -> i64 {
        self.threat.iter().sum()
    }

    /// The strongest sector, ties to the lowest index; `None` while empty.
    pub fn strongest(&self) -> Option<usize> {
        let mut best: Option<(usize, i64)> = None;
        for (i, &v) in self.threat.iter().enumerate() {
            if v > 0 && best.is_none_or(|(_, b)| v > b) {
                best = Some((i, v));
            }
        }
        best.map(|(i, _)| i)
    }
}

/// The radar early warning (SPEC §5.4), held for 10 s after it is raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadarWarning {
    /// Centroid of the closing units.
    pub toward: Pos,
    /// The warning lapses after this tick.
    pub until: Tick,
}

/// What threatens our base. **Filled by the army manager** each tick in its
/// phase step; every field defaults to "nothing known".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreatPicture {
    /// Remembered armed contacts within `army.defend_radius` of home or 400 of
    /// a besieged factory, in id order (SPEC §5.4). Default: empty.
    pub intruders: Vec<UnitId>,
    /// The intruder nearest home that is seen now, standing and not a frame.
    pub target_intruder: Option<UnitId>,
    /// Metal of the seen, standing, non-frame intruders.
    pub intruder_metal: Amount,
    /// Count of the seen, standing, non-frame intruders.
    pub intruder_count: u32,
    /// The 16-sector threat memory (`army.sectors`).
    pub sectors: SectorMemory,
    /// The radar early warning, if raised.
    pub radar_warning: Option<RadarWarning>,
    /// An outpost under raid this pass (SPEC §5.4 outpost raids), if any.
    pub raided_outpost: Option<Pos>,
}

impl ThreatPicture {
    /// "An enemy near base" (urgency, SPEC §4.2 tower worth).
    pub fn base_under_attack(&self) -> bool {
        !self.intruders.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sectors_follow_bearing_from_minus_180() {
        let h = Pos::new(1000, 1000);
        // Bearing 180° (−x) is the boundary into sector 0.
        assert_eq!(sector_of(h, Pos::new(900, 1000)), 0);
        // Just below −180° + 22.5°.
        assert_eq!(sector_of(h, Pos::new(900, 970)), 0);
        // Bearing −90° (−z) is 90° past −180°: sector 4.
        assert_eq!(sector_of(h, Pos::new(1000, 900)), 4);
        // Bearing 0° (+x): sector 8; bearing 90° (+z): sector 12.
        assert_eq!(sector_of(h, Pos::new(1100, 1000)), 8);
        assert_eq!(sector_of(h, Pos::new(1000, 1100)), 12);
        // 30° is in the second bin after 0°.
        assert_eq!(sector_of(h, Pos::new(1087, 1050)), 9);
        // 170° is in the last sector.
        assert_eq!(sector_of(h, Pos::new(902, 1017)), 15);
        assert_eq!(sector_of(h, h), 0);
        // Every sector is reachable round a circle.
        let mut seen = [false; SECTORS];
        for deg in 0..360 {
            let (x, z) = unit_circle(deg);
            seen[sector_of(h, Pos::new(1000 + x, 1000 + z))] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    fn unit_circle(deg: i32) -> (i32, i32) {
        // A coarse integer circle from a table of sines at 1° steps is not
        // needed: walk the square instead, which covers every bearing.
        let t = deg * 8 / 360; // octant
        let f = (deg * 8 % 360) * 1000 / 360;
        match t {
            0 => (1000, f),
            1 => (1000 - f, 1000),
            2 => (-f, 1000),
            3 => (-1000, 1000 - f),
            4 => (-1000, -f),
            5 => (-1000 + f, -1000),
            6 => (f, -1000),
            _ => (1000, -1000 + f),
        }
    }

    #[test]
    fn sector_memory_adds_halves_and_ranks() {
        let mut m = SectorMemory::default();
        assert_eq!(m.strongest(), None);
        m.add(3, Amount(400)); // rounds to 0, at least 1
        m.add(5, Amount::units(100));
        m.add(2, Amount::units(100));
        assert_eq!(m.get(3), 1);
        assert_eq!(m.strongest(), Some(2));
        assert_eq!(m.total(), 201);
        m.decay(0, 1800);
        m.decay(1799, 1800);
        assert_eq!(m.total(), 201);
        m.decay(1800, 1800);
        assert_eq!(m.get(5), 50);
        assert_eq!(m.get(3), 0);
    }
}
