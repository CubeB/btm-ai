//! The statistics of SPEC.md section 8.3: a Wilson score interval and an exact
//! two-sided binomial sign test.
//!
//! The arena is offline, so `f64` is fine for the results. The sign test is
//! nonetheless computed with exact integers while they fit (up to 120 decided
//! pairs) and falls back to a log-space sum beyond that.

/// The default normal quantile: 1.96, a 95% interval.
pub const DEFAULT_Z: f64 = 1.96;

/// The Wilson score interval for `successes` out of `n` trials, clamped to
/// `[0, 1]`.
///
/// With p = successes / n:
///
/// - centre = (p + z^2 / 2n) / (1 + z^2 / n)
/// - half-width = z * sqrt((p(1 - p) + z^2 / 4n) / n) / (1 + z^2 / n)
///
/// `n = 0` gives `(0, 1)`, and so does a `z` that is negative or not finite.
/// `successes` above `n` counts as `n`.
pub fn wilson(successes: u64, n: u64, z: f64) -> (f64, f64) {
    if n == 0 || !z.is_finite() || z < 0.0 {
        return (0.0, 1.0);
    }
    let nf = n as f64;
    let p = successes.min(n) as f64 / nf;
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let centre = (p + z2 / (2.0 * nf)) / denom;
    let half = z * ((p * (1.0 - p) + z2 / (4.0 * nf)) / nf).sqrt() / denom;
    (
        (centre - half).clamp(0.0, 1.0),
        (centre + half).clamp(0.0, 1.0),
    )
}

/// The result of an exact two-sided sign test at p = 0.5.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignTest {
    /// Pairs the tuned seat did better in.
    pub better: u64,
    /// Pairs it did worse in.
    pub worse: u64,
    /// `better + worse`: ties are excluded.
    pub n: u64,
    /// The two-sided p-value, `min(1, 2 * sum_{i<=k} C(n,i) / 2^n)` with
    /// `k = min(better, worse)`.
    pub p: f64,
    /// The same p-value as the exact fraction `numerator / 2^n`, when `n` is
    /// at most 120 (otherwise `None`).
    pub exact: Option<(u128, u32)>,
}

/// Largest `n` for which the exact integer form is kept: `2^121` fits in a
/// `u128`.
const EXACT_MAX_N: u64 = 120;

/// The exact two-sided binomial sign test over `better` versus `worse`.
/// Ties are not passed in: they are excluded, as the spec says.
pub fn sign_test(better: u64, worse: u64) -> SignTest {
    let n = better.saturating_add(worse);
    let k = better.min(worse);
    if n <= EXACT_MAX_N {
        let n32 = n as u32;
        let mut choose: u128 = 1; // C(n, 0)
        let mut sum: u128 = 0;
        for i in 0..=k {
            sum += choose;
            // C(n, i + 1) = C(n, i) * (n - i) / (i + 1); the division is exact.
            choose = choose * u128::from(n - i) / u128::from(i + 1);
        }
        let full = 1u128 << n32;
        let numerator = sum.saturating_mul(2).min(full);
        SignTest {
            better,
            worse,
            n,
            p: numerator as f64 / full as f64,
            exact: Some((numerator, n32)),
        }
    } else {
        SignTest {
            better,
            worse,
            n,
            p: sign_p_log(n, k),
            exact: None,
        }
    }
}

/// The same p-value summed in log space, for large `n`.
fn sign_p_log(n: u64, k: u64) -> f64 {
    let nf = n as f64;
    let ln2n = nf * std::f64::consts::LN_2;
    let mut ln_choose = 0.0_f64;
    let mut sum = 0.0_f64;
    for i in 0..=k {
        sum += (ln_choose - ln2n).exp();
        let i_f = i as f64;
        ln_choose += (nf - i_f).ln() - (i_f + 1.0).ln();
    }
    (2.0 * sum).min(1.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} vs {b}");
    }

    #[test]
    fn wilson_8_of_10() {
        // Hand computation: z^2 = 3.8416, denominator 1.38416, centre
        // 0.99208 / 1.38416 = 0.716738, half-width
        // 1.96 * sqrt(0.025604) / 1.38416 = 0.226587.
        let (lo, hi) = wilson(8, 10, DEFAULT_Z);
        close(lo, 0.49016, 5e-5);
        close(hi, 0.94332, 5e-5);
    }

    #[test]
    fn wilson_edges() {
        // 0 of n: the upper bound is exactly z^2 / (n + z^2).
        let (lo, hi) = wilson(0, 10, DEFAULT_Z);
        assert_eq!(lo, 0.0);
        close(hi, 3.8416 / 13.8416, 1e-12);
        // n of n: the lower bound is exactly n / (n + z^2).
        let (lo, hi) = wilson(10, 10, DEFAULT_Z);
        close(lo, 10.0 / 13.8416, 1e-12);
        assert_eq!(hi, 1.0);
        assert_eq!(wilson(0, 0, DEFAULT_Z), (0.0, 1.0));
        assert_eq!(wilson(5, 0, DEFAULT_Z), (0.0, 1.0));
        assert_eq!(wilson(3, 10, f64::NAN), (0.0, 1.0));
        assert_eq!(wilson(3, 10, -1.0), (0.0, 1.0));
        assert_eq!(wilson(11, 10, DEFAULT_Z), wilson(10, 10, DEFAULT_Z));
    }

    #[test]
    fn wilson_matches_the_spec_table_sizes() {
        // 25 better of 32 games, and 29 of 32 (SPEC.md 8.4).
        let (lo, hi) = wilson(25, 32, DEFAULT_Z);
        close(lo, 0.61245, 5e-5);
        close(hi, 0.88976, 5e-5);
        let (lo, hi) = wilson(29, 32, DEFAULT_Z);
        close(lo, 0.75782, 5e-5);
        close(hi, 0.96760, 5e-5);
    }

    #[test]
    fn wilson_z_is_configurable() {
        let (lo95, hi95) = wilson(8, 10, 1.96);
        let (lo90, hi90) = wilson(8, 10, 1.645);
        close(lo90, 0.54077, 5e-5);
        close(hi90, 0.93145, 5e-5);
        assert!(lo90 > lo95 && hi90 < hi95);
        assert_eq!(wilson(8, 10, 0.0), (0.8, 0.8));
    }

    #[test]
    fn sign_test_9_of_10() {
        // n = 10, k = 1: p = 2 * (1 + 10) / 1024 = 22 / 1024.
        let t = sign_test(9, 1);
        assert_eq!(t.n, 10);
        assert_eq!(t.exact, Some((22, 10)));
        close(t.p, 0.021_484_375, 1e-15);
        assert_eq!(sign_test(1, 9).exact, Some((22, 10)));
    }

    #[test]
    fn sign_test_known_values() {
        // A clean sweep of 8: 2 / 256.
        assert_eq!(sign_test(8, 0).exact, Some((2, 8)));
        // An even split caps at 1.
        assert_eq!(sign_test(5, 5).p, 1.0);
        // No decided pairs: p = 1.
        assert_eq!(sign_test(0, 0).p, 1.0);
        // The spec's own table (8.4), with ties already excluded.
        assert_eq!(format!("{:.3}", sign_test(21, 5).p), "0.002");
        assert_eq!(format!("{:.2}", sign_test(10, 12).p), "0.83");
        assert_eq!(sign_test(8, 7).p, 1.0);
        assert!(sign_test(25, 5).p < 0.001);
        // 25 vs 5: 2 * 174437 / 2^30.
        assert_eq!(sign_test(25, 5).exact, Some((348_874, 30)));
    }

    #[test]
    fn sign_test_is_exact_up_to_120_and_logspace_beyond() {
        let t = sign_test(60, 60);
        assert_eq!(t.exact, Some((1u128 << 120, 120)));
        assert_eq!(t.p, 1.0);
        assert!(sign_test(100, 21).exact.is_none());
        // The two methods agree where both apply.
        for (b, w) in [(60, 40), (75, 25), (50, 50), (90, 10)] {
            let exact = sign_test(b, w).p;
            let log = sign_p_log(b + w, b.min(w));
            close(exact, log, 1e-9 * exact.max(1e-300) + 1e-15);
        }
        let big = sign_test(300, 200);
        assert!(big.p > 0.0 && big.p < 1e-4);
        assert_eq!(sign_test(200, 200).p, 1.0);
    }
}
