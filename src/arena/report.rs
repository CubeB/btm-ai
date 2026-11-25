//! The plain-text report of SPEC.md section 8.3.
//!
//! One row per candidate, ranked by fitness and then by the Wilson lower
//! bound (the caller passes summaries already ranked, see [`super::rank`]):
//! n, better / same / worse, mean delta, the Wilson interval to 2 places, the
//! sign-test p to 3 places, the decided-win rates and their delta, fitness,
//! and then the mean delta per scenario.

use std::fmt::Write;

use super::schedule::{Phase, Scenario};
use super::{CandidateSummary, StatsConfig};
use crate::personality::Faction;

/// The run's settings, printed at the head of the report.
#[derive(Debug, Clone)]
pub struct ReportInfo {
    /// The phase, if the report covers one.
    pub phase: Option<Phase>,
    /// The first seed and the number of seeds of the block played.
    pub seeds: Option<(u64, u64)>,
    /// The game-time cap in seconds.
    pub length_s: u32,
    /// The scenarios played.
    pub scenarios: Vec<Scenario>,
    /// The faction the cross was restricted to, if any.
    pub restrict: Option<Faction>,
    /// The scoring configuration.
    pub cfg: StatsConfig,
}

/// A label for a normal quantile: `95%` for 1.96 and so on, else `z=...`.
fn confidence_label(z: f64) -> String {
    for (zz, pct) in [(1.282, 80), (1.645, 90), (1.96, 95), (2.576, 99)] {
        if (z - zz).abs() < 0.001 {
            return format!("{pct}%");
        }
    }
    format!("z={z}")
}

/// Renders the report. Always ends with a newline.
pub fn render_report(info: &ReportInfo, ranked: &[CandidateSummary]) -> String {
    let mut s = String::new();
    let conf = confidence_label(info.cfg.z);
    // `write!` to a `String` cannot fail, so its result is ignored below.
    let _ = writeln!(s, "Tuning arena report");
    let _ = writeln!(
        s,
        "Method: Wilson {conf} interval (z = {}) on the fraction of better games, ties count as failures;",
        info.cfg.z
    );
    let _ = writeln!(
        s,
        "        exact two-sided sign test over better vs worse, ties excluded."
    );
    let _ = writeln!(
        s,
        "        better/worse: delta beyond +/-{} against the control game.",
        info.cfg.margin_threshold
    );
    let _ = writeln!(
        s,
        "        fitness = mean delta + {} x (tuned decided-win rate - control rate){}.",
        info.cfg.win_weight,
        if info.cfg.waste_scoring {
            format!(" - {} x any increase in waste", info.cfg.waste_weight)
        } else {
            String::new()
        }
    );
    if let Some(phase) = info.phase {
        let _ = writeln!(s, "Phase: {}", phase.name());
    }
    if let Some((first, count)) = info.seeds {
        let _ = writeln!(
            s,
            "Seeds: {first}..{} ({count} per scenario)",
            first.saturating_add(count)
        );
    }
    let scenarios: Vec<&str> = info.scenarios.iter().map(|x| x.name()).collect();
    let _ = writeln!(
        s,
        "Length: {} s. Scenarios: {}",
        info.length_s,
        scenarios.join(", ")
    );
    if let Some(f) = info.restrict {
        let _ = writeln!(s, "Cross restricted to: {}", f.name());
    }
    let _ = writeln!(s);

    let wilson_head = format!("Wilson {conf}");
    let _ = writeln!(
        s,
        "{:>4}  {:<22} {:<16}  {:>4}  {:>13}  {:>10}  {:<14}  {:>6}  {:>13}  {:>7}  {:>8}",
        "Rank",
        "Candidate",
        "Id",
        "n",
        "Better/same/worse",
        "Mean delta",
        wilson_head,
        "Sign p",
        "Decided t/c",
        "D delta",
        "Fitness"
    );
    for (i, c) in ranked.iter().enumerate() {
        let wilson = format!("[{:.2}, {:.2}]", c.wilson.0, c.wilson.1);
        let bsw = format!("{} / {} / {}", c.better, c.same, c.worse);
        let decided = format!("{:.2} vs {:.2}", c.tuned_win_rate, c.control_win_rate);
        let _ = writeln!(
            s,
            "{:>4}  {:<22} {:016x}  {:>4}  {:>13}  {:>+10.1}  {:<14}  {:>6.3}  {:>13}  {:>+7.2}  {:>+8.1}",
            i + 1,
            c.name,
            c.id,
            c.n,
            bsw,
            c.mean_delta,
            wilson,
            c.sign.p,
            decided,
            c.decided_delta(),
            c.fitness
        );
    }
    if ranked.iter().any(|c| !c.per_scenario.is_empty()) {
        let _ = writeln!(s);
        let _ = writeln!(s, "Mean delta per scenario:");
        for c in ranked {
            let parts: Vec<String> = c
                .per_scenario
                .iter()
                .map(|(sc, count, mean)| format!("{} {:+.1} (n={count})", sc.name(), mean))
                .collect();
            let _ = writeln!(s, "  {}: {}", c.name, parts.join("; "));
        }
    }
    s
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::arena::stats::{sign_test, wilson};

    fn summary(name: &str, id: u64) -> CandidateSummary {
        CandidateSummary {
            name: name.into(),
            id,
            n: 32,
            better: 25,
            same: 2,
            worse: 5,
            mean_delta: 74.5,
            wilson: wilson(25, 32, 1.96),
            sign: sign_test(25, 5),
            tuned_win_rate: 0.5,
            control_win_rate: 0.09,
            waste_increase: 0.0,
            fitness: 74.5 + 20.0 * 0.41,
            per_scenario: vec![(Scenario::ArmMirror, 16, 80.5), (Scenario::Cross, 16, -3.0)],
        }
    }

    #[test]
    fn report_has_the_spec_fields() {
        let info = ReportInfo {
            phase: Some(Phase::Confirmation),
            seeds: Some((101, 16)),
            length_s: 1800,
            scenarios: vec![Scenario::ArmMirror, Scenario::Cross],
            restrict: Some(Faction::Arm),
            cfg: StatsConfig::default(),
        };
        let text = render_report(&info, &[summary("arm-lab-zero", 0xabc)]);
        assert!(text.contains("Wilson 95% interval (z = 1.96)"));
        assert!(text.contains("Phase: confirmation"));
        assert!(text.contains("Seeds: 101..117 (16 per scenario)"));
        assert!(text.contains("Length: 1800 s. Scenarios: ARM mirror, ARM-vs-CORE cross"));
        assert!(text.contains("Cross restricted to: ARM"));
        let row = text
            .lines()
            .find(|l| l.contains("arm-lab-zero") && l.contains("0000000000000abc"));
        let row = row.unwrap();
        assert!(row.contains("25 / 2 / 5"), "{row}");
        assert!(row.contains("+74.5"), "{row}");
        assert!(row.contains("[0.61, 0.89]"), "{row}");
        assert!(row.contains("0.000"), "{row}");
        assert!(row.contains("0.50 vs 0.09"), "{row}");
        assert!(row.contains("+0.41"), "{row}");
        assert!(
            text.contains("arm-lab-zero: ARM mirror +80.5 (n=16); ARM-vs-CORE cross -3.0 (n=16)")
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn rows_are_in_the_order_given_and_numbered() {
        let info = ReportInfo {
            phase: None,
            seeds: None,
            length_s: 600,
            scenarios: vec![],
            restrict: None,
            cfg: StatsConfig::with_z(1.645).unwrap(),
        };
        let text = render_report(&info, &[summary("first", 1), summary("second", 2)]);
        assert!(text.contains("Wilson 90%"));
        let rows: Vec<&str> = text
            .lines()
            .filter(|l| l.contains("first") || l.contains("second"))
            .collect();
        assert!(rows[0].trim_start().starts_with("1  first"));
        assert!(rows[1].trim_start().starts_with("2  second"));
        let odd = StatsConfig::with_z(1.5).unwrap();
        assert_eq!(confidence_label(odd.z), "z=1.5");
    }

    #[test]
    fn empty_report_still_has_a_header() {
        let info = ReportInfo {
            phase: None,
            seeds: None,
            length_s: 1,
            scenarios: vec![],
            restrict: None,
            cfg: StatsConfig::default(),
        };
        let text = render_report(&info, &[]);
        assert!(text.contains("Rank"));
        assert!(!text.contains("Mean delta per scenario"));
    }
}
