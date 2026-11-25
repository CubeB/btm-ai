//! Whole games in the sim through `sim::Game`: the AI against the scripted
//! opponents on the land and water maps, and against itself.
//!
//! These check that games end the way they should against the scripts, not
//! how strong the AI is: the sim is not the game.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::OnceLock;

use btm_ai::brain::DispatchStats;
use btm_ai::personality::{resolve, Difficulty, Faction, Registry};
use btm_ai::sim::{Event, Game, Map, Pilot, PlayerSetup, Script, Side, SimConfig};
use btm_ai::view::{Command, DefId, GameView, PlayerId, Tick, TICKS_PER_SECOND};

const MINUTE: Tick = 60 * TICKS_PER_SECOND;

fn ai(faction: Faction) -> Pilot {
    let reg = Registry::builtin();
    let r = resolve(
        reg.get("Balanced").unwrap(),
        Difficulty::Standard,
        faction,
        &[],
    )
    .unwrap();
    Pilot::Ai(Box::new(r))
}

fn against(map: Map, seed: u64, script: Script, minutes: Tick) -> Game {
    let pilots = vec![ai(Faction::Arm), Pilot::Script(script)];
    let mut g = Game::new(map, SimConfig::two_player(seed), pilots).unwrap();
    g.run_to(minutes * MINUTE).unwrap();
    g
}

fn won(g: &Game) -> bool {
    g.sim.decided() && g.sim.player_alive(PlayerId(0)) && !g.sim.player_alive(PlayerId(1))
}

#[test]
fn the_ai_beats_the_rush_script_on_land() {
    let g = against(Map::two_player(), 3, Script::rush(), 8);
    assert!(won(&g), "tick {}", g.sim.tick());
}

#[test]
fn the_ai_beats_the_passive_script_on_land() {
    let g = against(Map::two_player(), 3, Script::passive(), 10);
    assert!(won(&g), "tick {}", g.sim.tick());
}

#[test]
fn the_ai_ferries_across_water_and_beats_the_passive_script() {
    let g = against(Map::water_split(), 3, Script::passive(), 14);
    assert!(won(&g), "tick {}", g.sim.tick());
}

#[test]
fn a_mirror_is_a_function_of_its_seed() {
    let play = || {
        let pilots = vec![ai(Faction::Arm), ai(Faction::Core)];
        let mut g = Game::new(Map::two_player(), SimConfig::two_player(5), pilots).unwrap();
        g.run_to(4 * MINUTE).unwrap();
        (
            g.sim.tick(),
            g.sim.state_hash(),
            g.sim.unit_count(PlayerId(0)),
        )
    };
    let a = play();
    assert_eq!(a, play());
    assert!(a.2 > 10, "{a:?}");
}

/// What a whole AI-against-AI game did, for the assertions below.
#[derive(Debug, Default)]
struct Summary {
    decided: bool,
    ended: Tick,
    /// Per seat: completed and lost units by type name.
    built: [BTreeMap<String, u32>; 2],
    died: [BTreeMap<String, u32>; 2],
    /// Per seat: rejected orders by "reason/command".
    rejected: [BTreeMap<String, u32>; 2],
    /// Per seat: one-second samples with the commander in deep water.
    commander_wet_s: [u32; 2],
    /// Per seat: the dispatcher's counters at the end.
    stats: [DispatchStats; 2],
}

fn kind(c: &Command) -> &'static str {
    match c {
        Command::Build { .. } => "Build",
        Command::Repair(_) => "Repair",
        Command::ReclaimFeature(_) => "ReclaimFeature",
        Command::ReclaimUnit(_) => "ReclaimUnit",
        Command::Load(_) => "Load",
        Command::Unload(_) => "Unload",
        Command::Move(_) => "Move",
        Command::Attack(_) => "Attack",
        _ => "other",
    }
}

/// Deeper than any land unit but the commander walks (the sim's kbots
/// wade to 20).
const DEEP: i32 = 20;

/// Balanced against Balanced, one faction on both seats, to a decision or
/// `minutes`.
fn mirror(map: Map, seed: u64, faction: Faction, minutes: Tick) -> Summary {
    let side = match faction {
        Faction::Arm => Side::Arm,
        Faction::Core => Side::Core,
    };
    let mut cfg = SimConfig::two_player(seed);
    cfg.players = vec![PlayerSetup::new(side), PlayerSetup::new(side)];
    let mut g = Game::new(map, cfg, vec![ai(faction), ai(faction)]).unwrap();
    let mut s = Summary::default();
    while g.sim.tick() < minutes * MINUTE {
        let decided = g.sim.decided();
        g.step().unwrap();
        if decided {
            break;
        }
        if g.sim.tick() % TICKS_PER_SECOND == 0 {
            for (i, wet) in s.commander_wet_s.iter_mut().enumerate() {
                let p = PlayerId(u8::try_from(i).unwrap());
                let v = g.sim.view(p);
                let deep = v.own_units().iter().any(|u| {
                    v.unit_def(u.def).unwrap().commander && v.water_depth(u.pos.tile()) > DEEP
                });
                *wet += u32::from(deep);
            }
        }
    }
    s.decided = g.sim.decided();
    s.ended = g.sim.tick();
    let name = |d: DefId| g.sim.defs()[usize::from(d.0)].name.clone();
    for e in &g.sim.events {
        match *e {
            Event::Completed { owner, def, .. } => {
                *s.built[usize::from(owner.0)].entry(name(def)).or_default() += 1;
            }
            Event::Died { owner, def, .. } => {
                *s.died[usize::from(owner.0)].entry(name(def)).or_default() += 1;
            }
        }
    }
    for r in &g.sim.rejected {
        let k = format!("{:?}/{}", r.reason, kind(&r.order.command));
        *s.rejected[usize::from(r.player.0)].entry(k).or_default() += 1;
    }
    for (i, st) in s.stats.iter_mut().enumerate() {
        *st = g
            .ai(PlayerId(u8::try_from(i).unwrap()))
            .unwrap()
            .stats()
            .dispatch;
    }
    s
}

/// The water mirrors, each played once for all the tests that read it.
fn water(seed: u64, faction: Faction) -> &'static Summary {
    static GAMES: [OnceLock<Summary>; 4] = [const { OnceLock::new() }; 4];
    let i = usize::from(seed == 3) * 2 + usize::from(faction == Faction::Core);
    assert!(seed == 2 || seed == 3, "only seeds 2 and 3 are cached");
    GAMES[i].get_or_init(|| mirror(Map::water_split(), seed, faction, 20))
}

fn waters() -> Vec<(&'static str, &'static Summary)> {
    let games = [
        ("ARM 2", 2, Faction::Arm),
        ("ARM 3", 3, Faction::Arm),
        ("CORE 2", 2, Faction::Core),
        ("CORE 3", 3, Faction::Core),
    ];
    // Played side by side: each is a long game in a debug build.
    std::thread::scope(|sc| {
        for &(_, seed, f) in &games {
            sc.spawn(move || water(seed, f));
        }
    });
    games
        .iter()
        .map(|&(name, seed, f)| (name, water(seed, f)))
        .collect()
}

fn count(m: &BTreeMap<String, u32>, names: &[&str]) -> u32 {
    names.iter().map(|n| m.get(*n).copied().unwrap_or(0)).sum()
}

#[test]
fn water_mirrors_do_not_ferry_builders_to_the_far_shore() {
    // The far shore's extractor sites are the enemy's; a constructor
    // ferried there only dies (it was 14 and 20 lost a game).
    for (name, s) in waters() {
        for p in 0..2 {
            let lost = count(&s.died[p], &["ARMCK", "CORCK"]);
            assert!(lost <= 5, "{name} seat {p}: {lost} constructors lost");
        }
    }
}
