//! The simulation's own random stream: xorshift64*, seeded, integer only.
//! (Distinct from the AI seat generator of SPEC §7, which the seats own.)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct XorShift {
    state: u64,
}

impl XorShift {
    pub fn new(seed: u64) -> Self {
        // A zero state would stick at zero; mix the seed and avoid it.
        let s = seed ^ 0x9E37_79B9_7F4A_7C15;
        Self {
            state: if s == 0 { 1 } else { s },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A draw in `0..n` (`raw mod n`, SPEC §7). Returns 0 when `n` is 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    pub fn state(&self) -> u64 {
        self.state
    }
}
