//! Whole games in the sim through `sim::Game`: the AI against the scripted
//! opponents on the land and water maps, and against itself.
//!
//! These check that games end the way they should against the scripts, not
//! how strong the AI is: the sim is not the game.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use btm_ai::personality::{resolve, Difficulty, Faction, Registry};
use btm_ai::sim::{Game, Map, Pilot, Script, SimConfig};
use btm_ai::view::{PlayerId, Tick, TICKS_PER_SECOND};

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
