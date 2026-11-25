//! The AI's private random stream (SPEC.md section 7).
//!
//! Each computer seat owns one [`AiRng`]. There is no global state: the
//! generator is a plain value that the seat carries, so two seats (or two
//! peers) can never disturb each other's sequence.
//!
//! The generator the spec requires is the Park-Miller "minimal standard" linear
//! congruential generator (multiplier 48271, modulus 2^31 - 1), and a draw
//! "below n" is `raw mod n`, never a library distribution. Its seed is the one
//! value the seat drew from the simulation's own random stream when it was
//! created: use [`AiRng::from_sim_draw`] for that.
//!
//! Where no simulation stream exists (the arena, tests, tools), [`SplitMix64`]
//! can derive such a draw from a match seed and a player id: see
//! [`AiRng::from_match`].

/// The Park-Miller modulus, 2^31 - 1 (a Mersenne prime).
const MODULUS: u64 = 2_147_483_647;
/// The Park-Miller multiplier (the revised "minimal standard" constant).
const MULTIPLIER: u64 = 48_271;

/// A seat's private generator: Park-Miller, multiplier 48271, modulus 2^31 - 1.
///
/// The state is always in `1..=2^31 - 2`. Raw draws have the same range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRng {
    state: u32,
}

impl AiRng {
    /// The largest raw value, `2^31 - 2`.
    pub const MAX_RAW: u32 = (MODULUS - 1) as u32;

    /// Seeds the generator from the value the seat drew from the simulation's
    /// random stream. Any value is accepted: it is reduced modulo 2^31 - 1,
    /// and the one forbidden state, zero, becomes 1.
    pub fn from_sim_draw(draw: u64) -> AiRng {
        let reduced = (draw % MODULUS) as u32;
        AiRng {
            state: if reduced == 0 { 1 } else { reduced },
        }
    }

    /// Derives a generator from a match seed and a player id, for use when no
    /// simulation stream is available. The same inputs always give the same
    /// sequence; different player ids give unrelated sequences.
    pub fn from_match(match_seed: u64, player_id: u32) -> AiRng {
        let mut mixer = SplitMix64::new(match_seed);
        // Fold the player id in after one warm-up draw so that nearby seeds
        // and nearby ids do not give nearby states.
        let _ = mixer.next_u64();
        let mut seeded = SplitMix64::new(mixer.next_u64() ^ u64::from(player_id));
        AiRng::from_sim_draw(seeded.next_u64())
    }

    /// The next raw value, in `1..=2^31 - 2`.
    pub fn next_raw(&mut self) -> u32 {
        let next = (u64::from(self.state) * MULTIPLIER) % MODULUS;
        // `next` is below 2^31 - 1, so it always fits.
        self.state = next as u32;
        self.state
    }

    /// A draw below `n`: `raw mod n`, as the spec requires. Returns 0 when
    /// `n` is 0 or 1 (and still consumes one draw, so the stream position
    /// never depends on `n`).
    pub fn below(&mut self, n: u32) -> u32 {
        let raw = self.next_raw();
        if n == 0 {
            0
        } else {
            raw % n
        }
    }

    /// Picks one item to break an exact tie. `None` for an empty slice (no
    /// draw is consumed then).
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let n = u32::try_from(items.len()).unwrap_or(u32::MAX);
        items.get(self.below(n) as usize)
    }

    /// Fisher-Yates shuffle from the back, drawing `below(i + 1)` for each
    /// position `i` from the last to the second.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let bound = u32::try_from(i + 1).unwrap_or(u32::MAX);
            let j = self.below(bound) as usize;
            items.swap(i, j);
        }
    }
}

/// SplitMix64, a small 64-bit mixer, used only to turn seeds into seeds
/// (match seed plus player id into an [`AiRng`] seed; arena seed schedules).
/// It is not the seat's decision stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Starts the stream at `seed`.
    pub fn new(seed: u64) -> SplitMix64 {
        SplitMix64 { state: seed }
    }

    /// The next 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn park_miller_matches_the_published_check_value() {
        // The standard check for the 48271 minimal-standard generator: from
        // seed 1, the 10000th value is 399268537.
        let mut rng = AiRng::from_sim_draw(1);
        let mut last = 0;
        for _ in 0..10_000 {
            last = rng.next_raw();
        }
        assert_eq!(last, 399_268_537);
    }

    #[test]
    fn sequence_is_stable() {
        let mut rng = AiRng::from_sim_draw(1);
        let first: Vec<u32> = (0..5).map(|_| rng.next_raw()).collect();
        assert_eq!(
            first,
            [
                48_271,
                182_605_794,
                1_291_394_886,
                1_914_720_637,
                2_078_669_041
            ]
        );
        let mut rng = AiRng::from_match(12345, 3);
        let seq: Vec<u32> = (0..4).map(|_| rng.next_raw()).collect();
        let mut again = AiRng::from_match(12345, 3);
        let seq2: Vec<u32> = (0..4).map(|_| again.next_raw()).collect();
        assert_eq!(seq, seq2);
        // Pinned values: a change here is a determinism break.
        assert_eq!(seq, PINNED_FROM_MATCH);
    }

    const PINNED_FROM_MATCH: [u32; 4] = [1_747_266_522, 2_029_531_184, 1_443_290_371, 405_022_567];

    #[test]
    fn seeding_edge_cases() {
        assert_eq!(AiRng::from_sim_draw(0), AiRng::from_sim_draw(1));
        assert_eq!(AiRng::from_sim_draw(MODULUS), AiRng::from_sim_draw(1));
        assert_eq!(AiRng::from_sim_draw(MODULUS + 5), AiRng::from_sim_draw(5));
    }

    #[test]
    fn players_get_distinct_streams() {
        let a: Vec<u32> = {
            let mut r = AiRng::from_match(7, 0);
            (0..8).map(|_| r.next_raw()).collect()
        };
        let b: Vec<u32> = {
            let mut r = AiRng::from_match(7, 1);
            (0..8).map(|_| r.next_raw()).collect()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn raw_stays_in_range() {
        let mut rng = AiRng::from_match(99, 2);
        for _ in 0..10_000 {
            let v = rng.next_raw();
            assert!((1..=AiRng::MAX_RAW).contains(&v));
        }
    }

    #[test]
    fn below_is_raw_mod_n() {
        let mut a = AiRng::from_sim_draw(1);
        // 48271 mod 10, and 182605794 mod 10.
        assert_eq!(a.below(10), 1);
        assert_eq!(a.below(10), 4);
        assert_eq!(a.below(0), 0);
        assert_eq!(a.below(1), 0);
    }

    #[test]
    fn pick_and_shuffle_are_deterministic() {
        let items = [10, 20, 30, 40];
        let mut a = AiRng::from_sim_draw(42);
        let mut b = AiRng::from_sim_draw(42);
        assert_eq!(a.pick(&items), b.pick(&items));
        let empty: [u8; 0] = [];
        assert_eq!(a.pick(&empty), None);

        let mut x = [1, 2, 3, 4, 5, 6];
        let mut y = [1, 2, 3, 4, 5, 6];
        a.shuffle(&mut x);
        b.shuffle(&mut y);
        assert_eq!(x, y);
        let mut sorted = x;
        sorted.sort_unstable();
        assert_eq!(sorted, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn splitmix64_reference_values() {
        // Reference outputs for seed 0 (from the algorithm's published
        // description).
        let mut s = SplitMix64::new(0);
        assert_eq!(s.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(s.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }
}
