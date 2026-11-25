//! Army behaviour in the test sim, through [`Ai`] with the economy stub.
//! Starting armies are spawned, so nothing depends on the economy.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::brain::{Ai, Ctx, Manager, Phase, Source, Staging, DEFAULT_ORDER_DELAY};
use crate::personality::{resolve, Difficulty, Faction, Personality, Resolved};
use crate::sim::{Event, Map, Script, Sim, SimConfig};
use crate::view::{
    Command, GameView, Order, OrderRecorder, PlayerId, Pos, Tick, UnitId, TICKS_PER_SECOND,
};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const S: Tick = TICKS_PER_SECOND;

fn tuning(overrides: &[(&str, &str)]) -> Resolved {
    resolve(
        &Personality::new("test"),
        Difficulty::Standard,
        Faction::from_name("ARM"),
        overrides,
    )
    .unwrap()
}

/// A flat 3072 × 1024 map with the starts 2300 apart.
fn wide() -> Map {
    Map::builder(192, 64)
        .start(Pos::new(400, 512))
        .start(Pos::new(2700, 512))
        .build()
        .unwrap()
}

/// The economy stub: these tests spawn their armies and keep the economy
/// out of the picture.
struct NoEconomy;

impl Manager for NoEconomy {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}

/// The AI on P0 (ARM) with the economy and the scouts stubbed out (a
/// scout pass would borrow a combat unit, SPEC §5.10), a script on P1
/// (CORE).
fn game(map: Map, script: Script, vision: bool, overrides: &[(&str, &str)]) -> (Sim, Ai) {
    let mut cfg = SimConfig::two_player(17);
    cfg.order_delay = DEFAULT_ORDER_DELAY;
    cfg.players[0].full_vision = vision;
    let mut sim = Sim::new(map, cfg).unwrap();
    let draw = sim.draw_seat_seed();
    sim.set_script(P1, script);
    let mut ai = Ai::new(P0, &tuning(overrides), u64::from(draw)).unwrap();
    ai.set_manager(Source::Economy, Box::new(NoEconomy));
    ai.set_manager(Source::Scouts, Box::new(NoEconomy));
    (sim, ai)
}

/// Ticks the AI and the sim together, appending the AI's orders to `log`;
/// `each` sees the sim after every tick.
fn play(
    sim: &mut Sim,
    ai: &mut Ai,
    ticks: Tick,
    log: &mut Vec<(Tick, Order)>,
    mut each: impl FnMut(&mut Sim, &Ai),
) {
    for _ in 0..ticks {
        let mut rec = OrderRecorder::new();
        let t = sim.tick();
        {
            let v = sim.view(P0);
            ai.tick(&v, &v, &mut rec).unwrap();
        }
        let orders = rec.take();
        log.extend(orders.iter().map(|&o| (t, o)));
        sim.submit(P0, orders);
        sim.step();
        each(sim, ai);
    }
}

fn spawn(sim: &mut Sim, p: PlayerId, name: &str, at: Pos) -> UnitId {
    let d = sim.def_id(name).unwrap();
    sim.spawn(p, d, at, true).unwrap()
}

fn spawn_row(sim: &mut Sim, p: PlayerId, name: &str, at: Pos, n: i32) -> Vec<UnitId> {
    (0..n)
        .map(|i| {
            spawn(
                sim,
                p,
                name,
                Pos::new(at.x + (i % 4) * 36, at.z + (i / 4) * 36),
            )
        })
        .collect()
}

fn where_is(sim: &Sim, p: PlayerId, u: UnitId) -> Option<Pos> {
    sim.view(p)
        .own_units()
        .into_iter()
        .find(|o| o.id == u)
        .map(|o| o.pos)
}

fn health(sim: &Sim, p: PlayerId, u: UnitId) -> Option<i32> {
    sim.view(p)
        .own_units()
        .into_iter()
        .find(|o| o.id == u)
        .map(|o| o.health)
}

fn deaths(sim: &Sim, owner: PlayerId) -> usize {
    sim.events
        .iter()
        .filter(|e| matches!(e, Event::Died { owner: o, .. } if *o == owner))
        .count()
}

fn commands_to(log: &[(Tick, Order)], u: UnitId) -> Vec<Command> {
    log.iter()
        .filter(|(_, o)| o.unit == u)
        .map(|(_, o)| o.command)
        .collect()
}

fn rush_game(seconds: Tick) -> (Sim, Ai, Vec<(Tick, Order)>, Vec<Phase>) {
    let (mut sim, mut ai) = game(Map::two_player(), Script::rush(), false, &[]);
    // Equal starting armies, six raiders each.
    spawn_row(&mut sim, P0, "ARMPW", Pos::new(230, 230), 6);
    spawn_row(&mut sim, P1, "CORAK", Pos::new(700, 700), 6);
    let mut log = Vec::new();
    let mut phases = Vec::new();
    play(&mut sim, &mut ai, seconds * S, &mut log, |_, ai| {
        phases.push(ai.world().army.phase)
    });
    (sim, ai, log, phases)
}

#[test]
fn defends_against_a_rush_and_trades_well() {
    let (sim, ai, _log, phases) = rush_game(300);
    assert!(sim.player_alive(P0), "the AI survived");
    let lost = deaths(&sim, P0);
    let killed = deaths(&sim, P1);
    assert!(killed >= 6, "killed {killed}");
    assert!(lost < killed, "lost {lost}, killed {killed}");
    // The rush made it defend (SPEC §5.1 override), and with no factory it
    // fell back to Setup afterwards.
    assert!(phases.contains(&Phase::Defend));
    assert!(phases
        .iter()
        .all(|&p| p == Phase::Setup || p == Phase::Defend));
    // Raider contacts lean the lab towards rocket kbots (SPEC §5.3).
    let rock = sim.def_id("ARMROCK").unwrap();
    assert!(ai.world().army.lean.get(&rock).copied().unwrap_or(0) >= 2);
}

#[test]
fn two_runs_give_identical_order_logs() {
    let (sa, _, a, _) = rush_game(120);
    let (sb, _, b, _) = rush_game(120);
    assert!(a.len() > 20, "{}", a.len());
    assert_eq!(a, b);
    assert_eq!(sa.state_hash(), sb.state_hash());
}

/// Waves on the wide map: the AI sees everything (full vision), the enemy
/// is passive with six raiders at home.
fn odds_game() -> (Sim, Ai, Vec<UnitId>) {
    let (mut sim, ai) = game(wide(), Script::passive(), true, &[("army.wave_size", "12")]);
    spawn(&mut sim, P0, "ARMLAB", Pos::new(400, 760));
    let ours = spawn_row(&mut sim, P0, "ARMPW", Pos::new(450, 420), 6);
    spawn_row(&mut sim, P1, "CORAK", Pos::new(2600, 480), 6);
    (sim, ai, ours)
}

#[test]
fn the_wave_waits_for_the_odds_then_attacks_and_the_reserve_stays_home() {
    let (mut sim, mut ai, _) = odds_game();
    let mut log = Vec::new();
    let mut phases = Vec::new();
    play(&mut sim, &mut ai, 60 * S, &mut log, |_, ai| {
        phases.push(ai.world().army.phase)
    });
    // Equal metal (6 raiders each) is below 1.1: held in Growth, well
    // short of the 240 s patience, with no attack order.
    let w = ai.world();
    assert!(
        phases.iter().skip(1).all(|&p| p == Phase::Growth),
        "{phases:?}"
    );
    assert!(w.army.wave.is_empty());
    assert!(!log
        .iter()
        .any(|(_, o)| matches!(o.command, Command::AttackMove(_))));
    let rally = w.army.rally.unwrap();
    for &u in &w.own.combat {
        assert!(where_is(&sim, P0, u).unwrap().within(rally, 150));
    }
    // The enemy mix is all raiders: rocket kbots get the trigger lean and
    // the mobile bonus (SPEC §5.3: at most +4).
    let rock = sim.def_id("ARMROCK").unwrap();
    assert_eq!(w.army.lean.get(&rock), Some(&4));

    // Two rocket kbots tip the odds: 500 metal ≥ 1.1 × 300.
    spawn_row(&mut sim, P0, "ARMROCK", Pos::new(600, 400), 2);
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 5 * S, &mut log, |_, _| {});
    let w = ai.world();
    assert_eq!(w.army.phase, Phase::Assault);
    // 8 units: round(8 × 0.7) = 6 in the wave, in id order; 2 held back.
    assert_eq!(w.army.wave.len(), 6);
    assert_eq!(w.army.reserve.len(), 2);
    let wave = w.army.wave.clone();
    let reserve = w.army.reserve.clone();
    for &u in &wave {
        assert!(
            commands_to(&log, u)
                .iter()
                .any(|c| matches!(c, Command::AttackMove(_))),
            "wave unit {} attack-moves",
            u.0
        );
    }

    // The wave marches; the reserve holds the rally point until the wave
    // has had contact for 20 s.
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 15 * S, &mut log, |sim, _| {
        for &u in &reserve {
            let p = where_is(sim, P0, u).unwrap();
            assert!(p.within(rally, 200), "reserve {} at {p:?}", u.0);
        }
    });
    for &u in &wave {
        let p = where_is(&sim, P0, u).unwrap();
        assert!(!p.within(rally, 400), "wave {} still at {p:?}", u.0);
    }
    assert_eq!(ai.world().army.reserve, reserve);
}

#[test]
fn the_commander_fights_small_threats_and_flees_big_ones() {
    for (raiders, flees) in [(11, false), (13, true)] {
        let (mut sim, mut ai) = game(wide(), Script::passive(), false, &[]);
        // 50 metal each: 550 is within cmd.fight_max_metal (600), 650 not.
        // They stand together inside the danger radius (450) but out of
        // the commander's laser (300), so the count holds.
        let enemy: Vec<UnitId> = (0..raiders)
            .map(|_| spawn(&mut sim, P1, "CORAK", Pos::new(730, 512)))
            .collect();
        let mut log = Vec::new();
        let mut fled = false;
        play(&mut sim, &mut ai, 3 * S, &mut log, |_, ai| {
            fled |= ai.world().coord.commander_fleeing;
        });
        let w = ai.world();
        let cmdr = w.own.commander.unwrap();
        assert!(w.army.commander_danger, "{raiders}: in danger");
        assert_eq!(fled, flees, "{raiders}: fleeing");
        let cmds = commands_to(&log, cmdr);
        let attacks = cmds
            .iter()
            .any(|c| matches!(c, Command::Attack(_) | Command::DGun(_)));
        assert_eq!(attacks, !flees, "{raiders}: {cmds:?}");
        if flees {
            // At home it steps 350 straight away from the nearest threat.
            assert!(
                matches!(cmds.first(), Some(Command::Move(p)) if p.x < 400),
                "{cmds:?}"
            );
            let p = where_is(&sim, P1, enemy[0]).unwrap();
            let c = where_is(&sim, P0, cmdr).unwrap();
            assert!(c.dist(p) > 300 + 60, "{c:?}");
        }
    }
}

#[test]
fn builders_retreat_from_an_intruder() {
    let (mut sim, mut ai) = game(wide(), Script::passive(), false, &[]);
    let ck = spawn(&mut sim, P0, "ARMCK", Pos::new(1200, 512));
    let raider = spawn(&mut sim, P1, "CORAK", Pos::new(1450, 512));
    let mut log = Vec::new();
    let mut sheltered = false;
    play(&mut sim, &mut ai, 8 * S, &mut log, |_, ai| {
        sheltered |= ai.world().coord.sheltering.contains_key(&ck);
    });
    assert!(sheltered);
    let cmds = commands_to(&log, ck);
    assert!(
        matches!(cmds.first(), Some(Command::Move(p)) if p.x < 1200),
        "{cmds:?}"
    );
    // Home lies away from the threat, so it heads home, out of reach
    // (range 200 + margin 150).
    let p = where_is(&sim, P0, ck).unwrap();
    let r = where_is(&sim, P1, raider).unwrap();
    assert!(p.dist(r) > 350, "{p:?}");
    assert_eq!(health(&sim, P0, ck), Some(600));
}

/// A rocket kbot against a slowed raider walking at it; returns the closest
/// approach, the rocket's health and whether the raider died.
fn kite_duel(kite: bool) -> (i64, i32, bool) {
    let flag = if kite { "true" } else { "false" };
    let (mut sim, mut ai) = game(wide(), Script::passive(), false, &[("army.kite", flag)]);
    let slow = sim.def_id("CORAK").unwrap();
    sim.def_mut(slow).unwrap().speed = 20;
    let rocket = spawn(&mut sim, P0, "ARMROCK", Pos::new(1500, 512));
    let raider = spawn(&mut sim, P1, "CORAK", Pos::new(1800, 512));
    let mut closest = i64::MAX;
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 25 * S, &mut log, |sim, _| {
        let (Some(a), Some(b)) = (where_is(sim, P0, rocket), where_is(sim, P1, raider)) else {
            return;
        };
        closest = closest.min(a.dist(b));
        if sim.tick().is_multiple_of(15) {
            sim.submit(
                P1,
                [Order {
                    unit: raider,
                    command: Command::Move(a),
                    queue: crate::view::Queue::Immediate,
                }],
            );
        }
    });
    let hp = health(&sim, P0, rocket).unwrap_or(0);
    (closest, hp, where_is(&sim, P1, raider).is_none())
}

#[test]
fn kiting_keeps_a_ranged_unit_out_of_a_slow_raiders_reach() {
    // Raider range 200 + kite margin 40 is under the rocket's 450.
    let (closest, hp, dead) = kite_duel(true);
    assert!(dead);
    assert!(closest >= 200, "closest {closest}");
    assert_eq!(hp, 400, "never hit");
    // Without kiting the raider walks into range.
    let (closest, _, _) = kite_duel(false);
    assert!(closest < 200, "closest {closest}");
}

#[test]
fn defenders_go_and_look_for_an_intruder_out_of_sight() {
    // Regression: an intruder remembered but not seen now (here a tower a
    // constructor glimpsed, and lost sight of once the first response
    // stopped) held the AI in Defend with no
    // target, so the whole army stood at the rally for the rest of the
    // game. One responder now goes to the remembered spot.
    let (mut sim, mut ai) = game(wide(), Script::passive(), false, &[]);
    let raiders = spawn_row(&mut sim, P0, "ARMPW", Pos::new(450, 600), 4);
    let tower_at = Pos::new(1150, 512);
    let tower = spawn(&mut sim, P1, "CORLLT", tower_at);
    spawn(&mut sim, P0, "ARMCK", Pos::new(1150, 640));
    let mut log = Vec::new();
    let mut defended = false;
    play(&mut sim, &mut ai, 60 * S, &mut log, |_, ai| {
        defended |= ai.world().army.phase == Phase::Defend;
    });
    assert!(defended, "the tower was an intruder");
    let went = raiders.iter().any(|&u| {
        log.iter()
            .filter(|(t, o)| *t >= 10 * S && o.unit == u)
            .any(|(_, o)| match &o.command {
                Command::Attack(t) => *t == tower,
                Command::AttackMove(p) => p.within(tower_at, 64),
                _ => false,
            })
    });
    assert!(went, "someone went to look after the first response");
}

#[test]
fn a_unit_set_down_across_water_is_not_reordered_every_pass() {
    // Regression: a raider landed on the far shore was sent to the rally on
    // our side every pass; the walk ended at once and the limiter (SPEC
    // §2.3) refused most of the army's other orders behind it.
    let (mut sim, mut ai) = game(Map::water_split(), Script::passive(), false, &[]);
    let landed = spawn(&mut sim, P0, "ARMPW", Pos::new(1400, 900));
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 60 * S, &mut log, |_, _| {});
    let moves = commands_to(&log, landed)
        .into_iter()
        .filter(|c| matches!(c, Command::Move(_) | Command::AttackMove(_)))
        .count();
    assert!(moves <= 2, "{moves} moves to a unit that cannot walk home");
}
