//! Paired games in the sim: personality B against personality A (SPEC §8).
//!
//! ```text
//! cargo run --release --example arena -- [A] [B] [N] [SECONDS] [land|water] [FIRST_SEED]
//! ```
//!
//! Defaults: Balanced, Rush, 16 pairs, 480 s games, the land map, seed 1.
//! Half the pairs are ARM mirrors and half CORE mirrors, on one block of
//! N/2 consecutive seeds. Each pair is a control game (A against A) and a
//! tuned game (B in the scored seat against A) on the same seed; the
//! report is the §8 one, with Wilson intervals and the sign test.
//!
//! The sim is not the game: these results say how the personalities fare
//! in the sim, not how strong they are in the real engine.

use std::process::ExitCode;
use std::time::Instant;

use btm_ai::arena::{
    rank, render_report, run_pairs, summarise, Candidate, ControlCache, Phase, ReportInfo,
    Scenario, SeedSchedule, StatsConfig,
};
use btm_ai::personality::{Difficulty, Registry};
use btm_ai::sim::{Map, SimMatch};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("arena: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: &str| args.get(i).cloned().unwrap_or_else(|| d.to_string());
    let a = arg(0, "Balanced");
    let b = arg(1, "Rush");
    let n: u64 = arg(2, "16").parse().map_err(|e| format!("N: {e}"))?;
    let length_s: u32 = arg(3, "480").parse().map_err(|e| format!("SECONDS: {e}"))?;
    let map = match arg(4, "land").as_str() {
        "land" => Map::two_player(),
        "water" => Map::water_split(),
        other => return Err(format!("unknown map {other:?} (land or water)")),
    };
    let first: u64 = arg(5, "1")
        .parse()
        .map_err(|e| format!("FIRST_SEED: {e}"))?;
    if n < 2 || n % 2 == 1 {
        return Err("N must be even and at least 2".into());
    }

    let reg = Registry::builtin();
    let find = |name: &str| {
        reg.get(name).cloned().ok_or_else(|| {
            let names: Vec<&str> = reg.all().iter().map(|p| p.name.as_str()).collect();
            format!("no personality {name:?}; built-ins: {}", names.join(", "))
        })
    };
    let (pa, pb) = (find(&a)?, find(&b)?);
    let mut m = SimMatch::new(map, pa, pb, Difficulty::Standard).map_err(|e| e.to_string())?;
    let cand = Candidate::new(&b, &[]).map_err(|e| e.to_string())?;
    let cfg = StatsConfig::default();
    let mut cache = ControlCache::new();
    let mut sched = SeedSchedule::new(first);
    let scenarios = [Scenario::ArmMirror, Scenario::CoreMirror];

    let started = Instant::now();
    let mut pairs = Vec::new();
    let per = n / 2;
    let block = sched
        .allocate(Phase::Evaluate, per)
        .map_err(|e| e.to_string())?;
    for scenario in scenarios {
        pairs.extend(
            run_pairs(&mut m, &mut cache, &cand, scenario, None, &block, length_s)
                .map_err(|e| e.to_string())?,
        );
    }
    let mut ranked = vec![summarise(&cand.name, cand.id(), &pairs, &cfg)];
    rank(&mut ranked);
    let info = ReportInfo {
        phase: Some(Phase::Evaluate),
        seeds: Some((block.first, block.count)),
        length_s,
        scenarios: scenarios.to_vec(),
        restrict: None,
        cfg,
    };
    println!("{b} (tuned) against {a} (control), {n} pairs in the sim");
    print!("{}", render_report(&info, &ranked));
    println!();
    println!("pair  scenario      seed  tuned margin  control margin  delta");
    for (i, p) in pairs.iter().enumerate() {
        println!(
            "{:>4}  {:<12} {:>5}  {:>12.1}  {:>14.1}  {:>5.1}",
            i + 1,
            p.scenario.name(),
            p.seed,
            p.tuned.margin(),
            p.control.margin(),
            p.delta()
        );
    }
    for (seed, why) in &m.failures {
        println!("seed {seed}: game failed: {why}");
    }
    println!(
        "{} games in {:.1} s. Sim results do not measure strength in the real game.",
        m.games,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
