//! Times `Ai::tick` over one AI-against-AI game in the sim.
//!
//! ```text
//! cargo run --release --example tick_time -- [land|water] [MINUTES] [SEED] [PERSONALITY]
//! ```
//!
//! Defaults: the land map, 20 minutes, seed 1, Balanced on both seats. The
//! game stops early once it is decided. Wall-clock times on this machine;
//! the sim's own step is not counted.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use btm_ai::personality::{resolve, Difficulty, Faction, Registry};
use btm_ai::sim::{Game, Map, Pilot, SimConfig};
use btm_ai::view::{OrderRecorder, TICKS_PER_SECOND};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tick_time: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: &str| args.get(i).cloned().unwrap_or_else(|| d.to_string());
    let map = match arg(0, "land").as_str() {
        "land" => Map::two_player(),
        "water" => Map::water_split(),
        other => return Err(format!("unknown map {other:?} (land or water)")),
    };
    let minutes: u32 = arg(1, "20").parse().map_err(|e| format!("MINUTES: {e}"))?;
    let seed: u64 = arg(2, "1").parse().map_err(|e| format!("SEED: {e}"))?;
    let name = arg(3, "Balanced");
    let reg = Registry::builtin();
    let p = reg
        .get(&name)
        .ok_or_else(|| format!("no personality {name:?}"))?;
    let pilot = |f: Faction| {
        resolve(p, Difficulty::Standard, f, &[])
            .map(|r| Pilot::Ai(Box::new(r)))
            .map_err(|e| e.to_string())
    };
    let pilots = vec![pilot(Faction::Arm)?, pilot(Faction::Core)?];
    let mut g = Game::new(map, SimConfig::two_player(seed), pilots).map_err(|e| e.to_string())?;
    let cap = minutes.saturating_mul(60).saturating_mul(TICKS_PER_SECOND);

    let seats = g.ais.len();
    let mut total = vec![Duration::ZERO; seats];
    let mut worst = vec![(Duration::ZERO, 0); seats];
    let mut ticks = vec![0u64; seats];
    while g.sim.tick() < cap && !g.sim.decided() {
        for (i, ai) in g.ais.iter_mut().enumerate() {
            if !g.sim.player_alive(ai.player()) {
                continue;
            }
            let mut rec = OrderRecorder::new();
            let v = g.sim.view(ai.player());
            let t0 = Instant::now();
            ai.tick(&v, &v, &mut rec).map_err(|e| e.to_string())?;
            let dt = t0.elapsed();
            if let (Some(t), Some(w), Some(n)) =
                (total.get_mut(i), worst.get_mut(i), ticks.get_mut(i))
            {
                *t += dt;
                *n += 1;
                if dt > w.0 {
                    *w = (dt, g.sim.tick());
                }
            }
            g.sim.submit(ai.player(), rec.take());
        }
        g.sim.step();
    }
    println!(
        "{name} vs {name}, seed {seed}: ended at tick {} ({:.1} min), decided {}",
        g.sim.tick(),
        f64::from(g.sim.tick()) / f64::from(60 * TICKS_PER_SECOND),
        g.sim.decided()
    );
    for i in 0..seats {
        let n = ticks.get(i).copied().unwrap_or(0).max(1);
        let mean = total.get(i).copied().unwrap_or_default() / u32::try_from(n).unwrap_or(u32::MAX);
        let (w, at) = worst.get(i).copied().unwrap_or_default();
        println!(
            "seat {i}: {n} ticks, mean {:.1} us, max {:.0} us at tick {at}",
            mean.as_secs_f64() * 1e6,
            w.as_secs_f64() * 1e6
        );
    }
    Ok(())
}
