//! Air, scout, transport and navy behaviour in the test sim, through [`Ai`]
//! with the economy stubbed out. Units are spawned; roles the sim's roster
//! lacks (bombers, a sea transport) are made by renaming and reshaping an
//! unused CORE definition before the first tick, so the role table picks
//! them up.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::brain::{Ai, Ctx, LoanReason, Manager, Phase, Source, Staging, DEFAULT_ORDER_DELAY};
use crate::personality::{resolve, Difficulty, Faction, Personality, Resolved};
use crate::sim::{Event, Map, Script, Sim, SimConfig, TileRect};
use crate::view::{
    Command, DefId, GameView, Order, OrderRecorder, PlayerId, Pos, Tick, TransportDef, UnitDef,
    UnitId, VisCell, WeaponDef, TICKS_PER_SECOND, VIS_CELL_SIZE,
};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const S: Tick = TICKS_PER_SECOND;

struct NoEconomy;

impl Manager for NoEconomy {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}

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

/// The same map split by a strait 40 deep from x = 1280 to 1792, with a
/// shallow beach tile on either side so that the shore is walkable.
fn strait() -> Map {
    Map::builder(192, 64)
        .water(TileRect::new(80, 0, 112, 64), 40)
        .water(TileRect::new(79, 0, 80, 64), 10)
        .water(TileRect::new(112, 0, 113, 64), 10)
        .start(Pos::new(400, 512))
        .start(Pos::new(2700, 512))
        .build()
        .unwrap()
}

/// A sea 40 deep with a strip of land at either end.
fn sea() -> Map {
    Map::builder(128, 64)
        .water(TileRect::new(16, 0, 112, 64), 40)
        .start(Pos::new(128, 512))
        .start(Pos::new(1920, 512))
        .build()
        .unwrap()
}

/// The AI on P0 (ARM) with the economy stubbed out, a script on P1 (CORE).
/// `shape` may reshape the roster before the AI's first tick.
fn game(
    map: Map,
    vision: bool,
    overrides: &[(&str, &str)],
    shape: impl FnOnce(&mut Sim),
) -> (Sim, Ai) {
    let mut cfg = SimConfig::two_player(23);
    cfg.order_delay = DEFAULT_ORDER_DELAY;
    cfg.players[0].full_vision = vision;
    let mut sim = Sim::new(map, cfg).unwrap();
    let draw = sim.draw_seat_seed();
    sim.set_script(P1, Script::passive());
    shape(&mut sim);
    let mut ai = Ai::new(P0, &tuning(overrides), u64::from(draw)).unwrap();
    ai.set_manager(Source::Economy, Box::new(NoEconomy));
    (sim, ai)
}

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

/// Renames definition `from` to `to` and reshapes it.
fn remake(sim: &mut Sim, from: &str, to: &str, f: impl FnOnce(&mut UnitDef)) -> DefId {
    let id = sim.def_id(from).unwrap();
    let d = sim.def_mut(id).unwrap();
    d.name = to.to_string();
    f(d);
    id
}

/// A bomber made from CORE's fighter.
fn bomber(sim: &mut Sim) -> DefId {
    remake(sim, "CORVENG", "ARMTHUND", |d| {
        d.speed = 250;
        d.weapons = vec![WeaponDef {
            name: "BOMB".into(),
            range: 80,
            damage: 300,
            burst: 1,
            reload: 60,
            dropped: true,
            ..WeaponDef::default()
        }];
    })
}

fn where_is(sim: &Sim, p: PlayerId, u: UnitId) -> Option<Pos> {
    sim.view(p)
        .own_units()
        .into_iter()
        .find(|o| o.id == u)
        .map(|o| o.pos)
}

fn commands_to(log: &[(Tick, Order)], u: UnitId) -> Vec<Command> {
    log.iter()
        .filter(|(_, o)| o.unit == u)
        .map(|(_, o)| o.command)
        .collect()
}

fn died(sim: &Sim, u: UnitId) -> bool {
    sim.events
        .iter()
        .any(|e| matches!(e, Event::Died { unit, .. } if *unit == u))
}

// --- Air (SPEC §5.8) ---

/// Four bombers at home; the enemy has a lab under three AA towers and a
/// lone solar in the open.
fn bomber_game(overrides: &[(&str, &str)]) -> (Sim, Ai, Vec<UnitId>, UnitId, UnitId) {
    let mut ids = DefId(0);
    let (mut sim, ai) = game(wide(), true, overrides, |sim| ids = bomber(sim));
    spawn(&mut sim, P0, "ARMAP", Pos::new(400, 800));
    let bombers: Vec<UnitId> = (0..4)
        .map(|i| {
            sim.spawn(P0, ids, Pos::new(450 + i * 30, 700), true)
                .unwrap()
        })
        .collect();
    let lab = spawn(&mut sim, P1, "CORLAB", Pos::new(2600, 160));
    for at in [
        Pos::new(2500, 260),
        Pos::new(2700, 260),
        Pos::new(2600, 330),
    ] {
        spawn(&mut sim, P1, "CORRL", at);
    }
    let solar = spawn(&mut sim, P1, "CORSOLAR", Pos::new(1800, 920));
    (sim, ai, bombers, lab, solar)
}

#[test]
fn bombers_patrol_and_strike_targets_outside_anti_air_cover() {
    let (mut sim, mut ai, bombers, lab, solar) = bomber_game(&[]);
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 40 * S, &mut log, |_, _| {});
    // The first two bombers by id patrol; patrol bombers never sortie.
    for &b in &bombers[..2] {
        let cmds = commands_to(&log, b);
        assert!(matches!(cmds.first(), Some(Command::Patrol(_))), "{cmds:?}");
        assert!(!cmds.iter().any(|c| matches!(c, Command::Attack(_))));
    }
    // The other two strike the solar: the lab scores higher but sits under
    // three AA towers, more than `air.bomb_max_aa` (2).
    for &b in &bombers[2..] {
        let cmds = commands_to(&log, b);
        assert!(cmds.contains(&Command::Attack(solar)), "{cmds:?}");
        assert!(!cmds.contains(&Command::Attack(lab)), "{cmds:?}");
    }
    assert!(died(&sim, solar), "the solar was bombed");
    assert!(!died(&sim, lab));
    assert!(bombers.iter().all(|&b| where_is(&sim, P0, b).is_some()));
}

#[test]
fn bombers_take_the_lab_when_its_cover_is_allowed() {
    let (mut sim, mut ai, bombers, lab, _) = bomber_game(&[("air.bomb_max_aa", "3")]);
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 5 * S, &mut log, |_, _| {});
    for &b in &bombers[2..] {
        assert!(commands_to(&log, b).contains(&Command::Attack(lab)));
    }
}

#[test]
fn bomber_strikes_wait_for_the_ground_wave() {
    for sync in ["true", "false"] {
        let (mut sim, mut ai, bombers, _, solar) =
            bomber_game(&[("air.sync", sync), ("army.wave_size", "4")]);
        spawn(&mut sim, P0, "ARMLAB", Pos::new(300, 300));
        spawn_row(&mut sim, P0, "ARMPW", Pos::new(450, 420), 6);
        let mut log = Vec::new();
        let mut assault = false;
        play(&mut sim, &mut ai, 4 * S, &mut log, |_, ai| {
            assault |= ai.world().army.phase == Phase::Assault && !ai.world().army.wave.is_empty();
        });
        assert!(assault, "a wave is out");
        let struck = bombers[2..]
            .iter()
            .any(|&b| commands_to(&log, b).contains(&Command::Attack(solar)));
        assert_eq!(struck, sync == "false", "sync {sync}");
    }
}

// --- Scouts (SPEC §5.10) ---

fn explored(sim: &Sim) -> usize {
    let v = sim.view(P0);
    let m = v.map();
    let (w, h) = (m.width() / VIS_CELL_SIZE, m.height() / VIS_CELL_SIZE);
    (0..h)
        .flat_map(|z| (0..w).map(move |x| VisCell { x, z }))
        .filter(|&c| v.cell_explored(c))
        .count()
}

#[test]
fn a_scout_explores_the_map() {
    let mut cover = Vec::new();
    for enabled in ["1", "0"] {
        let (mut sim, mut ai) = game(wide(), false, &[("scout.enabled", enabled)], |_| {});
        spawn(&mut sim, P0, "ARMFAV", Pos::new(450, 560));
        let mut log = Vec::new();
        let start = explored(&sim);
        play(&mut sim, &mut ai, 120 * S, &mut log, |_, _| {});
        cover.push((start, explored(&sim)));
    }
    let (start, scouted) = cover[0];
    let (_, idle) = cover[1];
    assert!(scouted > start * 3, "{start} -> {scouted}");
    assert!(scouted > idle * 2, "scouted {scouted}, idle {idle}");
}

#[test]
fn a_ground_scout_is_not_sent_to_a_start_across_water() {
    // Regression: the opening leg skipped the terrain and reachability test,
    // so a ground scout was sent to the enemy start across the strait, got
    // as far as the shore, stopped, and was sent again every pass.
    let (mut sim, mut ai) = game(strait(), false, &[], |_| {});
    let fav = spawn(&mut sim, P0, "ARMFAV", Pos::new(450, 560));
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 60 * S, &mut log, |_, _| {});
    let far = Pos::new(2700, 512);
    let moves = commands_to(&log, fav);
    assert!(!moves.is_empty(), "the scout scouts its own side");
    assert!(!moves.contains(&Command::Move(far)), "{moves:?}");
    assert!(moves.len() < 20, "no re-issue loop: {}", moves.len());
}

#[test]
fn a_combat_unit_is_borrowed_as_scout_and_returned() {
    let (mut sim, mut ai) = game(wide(), false, &[], |_| {});
    spawn(&mut sim, P0, "ARMLAB", Pos::new(300, 300));
    let raiders = spawn_row(&mut sim, P0, "ARMPW", Pos::new(450, 420), 3);
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 3 * S, &mut log, |_, _| {});
    let w = ai.world();
    assert_eq!(w.coord.borrowed_scout, Some(raiders[0]));
    assert_eq!(
        w.ownership.loan(raiders[0]).map(|l| l.reason),
        Some(LoanReason::BorrowedScout)
    );
    assert!(!w.own.combat.contains(&raiders[0]));
    assert_eq!(w.own.army_size, 2, "the borrowed scout is not army");
    assert!(commands_to(&log, raiders[0])
        .iter()
        .any(|c| matches!(c, Command::Move(_))));

    // A real scout appears: the raider goes back to the army.
    spawn(&mut sim, P0, "ARMFAV", Pos::new(400, 600));
    play(&mut sim, &mut ai, 3 * S, &mut log, |_, _| {});
    let w = ai.world();
    assert_eq!(w.coord.borrowed_scout, None);
    assert!(w.ownership.loan(raiders[0]).is_none());
    assert!(w.own.combat.contains(&raiders[0]));
    assert_eq!(w.own.army_size, 3);
}

// --- Transport (SPEC §5.9) ---

/// Six raiders, a shipyard and a sea transport (CORE's destroyer made into
/// an unarmed six-seat ARMTSHIP) west of the strait; the enemy east of it.
fn ferry_game() -> (Sim, Ai, Vec<UnitId>, UnitId) {
    let mut ship = DefId(0);
    let (mut sim, ai) = game(strait(), true, &[("army.wave_size", "4")], |sim| {
        ship = remake(sim, "CORROY", "ARMTSHIP", |d| {
            d.weapons.clear();
            d.speed = 80;
            d.transport = Some(TransportDef {
                capacity: 6,
                max_size: 3,
            });
        });
    });
    spawn(&mut sim, P0, "ARMSY", Pos::new(1392, 300));
    spawn(&mut sim, P0, "ARMFAV", Pos::new(400, 700));
    let hull = sim.spawn(P0, ship, Pos::new(1400, 600), true).unwrap();
    let raiders = spawn_row(&mut sim, P0, "ARMPW", Pos::new(500, 400), 6);
    (sim, ai, raiders, hull)
}

#[test]
fn a_sea_ferry_carries_the_army_across_the_strait() {
    let (mut sim, mut ai, raiders, hull) = ferry_game();
    let mut log = Vec::new();
    let mut landed = 0;
    let mut rally = None;
    let mut rallied = false;
    play(&mut sim, &mut ai, 120 * S, &mut log, |sim, ai| {
        let across = raiders
            .iter()
            .filter_map(|&u| sim.view(P0).own_units().into_iter().find(|o| o.id == u))
            .filter(|o| o.carried_by.is_none() && o.pos.x > 1792)
            .count();
        landed = landed.max(across);
        let w = ai.world();
        rally = rally.or(w.coord.ferry_rally);
        rallied |= w.coord.ferry_rally.is_some() && w.army.rally == w.coord.ferry_rally;
    });
    let w = ai.world();
    assert!(w.coord.army_needs_ferry);
    // The rally point was the shore muster, west of the strait.
    let shore = rally.unwrap();
    assert!(shore.x < 1280, "{shore:?}");
    assert!(rallied, "the army rallied on the shore");
    let cmds = commands_to(&log, hull);
    let loads = cmds
        .iter()
        .filter(|c| matches!(c, Command::Load(_)))
        .count();
    assert_eq!(loads, 6, "{cmds:?}");
    assert!(cmds
        .iter()
        .any(|c| matches!(c, Command::Unload(p) if p.x > 1792)));
    assert_eq!(landed, 6, "raiders landed east of the strait");
    // Every passenger was given back to the army.
    assert!(w
        .ownership
        .lent_for(LoanReason::FerryPassenger)
        .next()
        .is_none());
}

#[test]
fn a_wave_without_a_ferry_waits_at_the_rally() {
    // Regression: with the objective across the strait and no ferry, the
    // wave attack-moved at it, stopped on the beach, and was re-ordered
    // every pass (tens of thousands of orders refused by the limiter).
    let (mut sim, mut ai) = game(strait(), true, &[("army.wave_size", "4")], |_| {});
    spawn(&mut sim, P0, "ARMLAB", Pos::new(300, 300));
    let raiders = spawn_row(&mut sim, P0, "ARMPW", Pos::new(450, 420), 6);
    let mut log = Vec::new();
    let mut assault = false;
    play(&mut sim, &mut ai, 120 * S, &mut log, |_, ai| {
        assault |= ai.world().army.phase == Phase::Assault && !ai.world().army.wave.is_empty();
    });
    assert!(assault, "a wave formed");
    let rally = ai.world().army.rally.unwrap();
    for &u in &raiders {
        let cmds = commands_to(&log, u);
        assert!(cmds.len() <= 4, "{u:?} was re-ordered: {cmds:?}");
        let at = where_is(&sim, P0, u).unwrap();
        assert!(at.within(rally, 200), "{u:?} at {at:?}, rally {rally:?}");
    }
}

#[test]
fn a_failed_landing_is_refused_and_books_nobody() {
    // No landing rings, and the objective inland is too far from water for
    // a sea landing: the search fails.
    let mut ship = DefId(0);
    let (mut sim, mut ai) = game(
        strait(),
        true,
        &[("army.wave_size", "4"), ("ferry.landing_rings", "0")],
        |sim| {
            ship = remake(sim, "CORROY", "ARMTSHIP", |d| {
                d.weapons.clear();
                d.transport = Some(TransportDef {
                    capacity: 6,
                    max_size: 3,
                });
            });
        },
    );
    spawn(&mut sim, P0, "ARMSY", Pos::new(1392, 300));
    spawn(&mut sim, P0, "ARMFAV", Pos::new(400, 700));
    sim.spawn(P0, ship, Pos::new(1400, 600), true).unwrap();
    spawn_row(&mut sim, P0, "ARMPW", Pos::new(500, 400), 6);
    let mut log = Vec::new();
    let mut refused = Vec::new();
    play(&mut sim, &mut ai, 3 * S, &mut log, |sim, ai| {
        let w = ai.world();
        if let Some(o) = w.army.objective {
            refused.push((sim.tick(), o, w.coord.landing_refused(o, 192, sim.tick())));
        }
    });
    let w = ai.world();
    assert!(!w.coord.ferry_refusals.is_empty());
    assert!(w
        .ownership
        .lent_for(LoanReason::FerryPassenger)
        .next()
        .is_none());
    assert!(refused.iter().any(|&(_, _, r)| r), "{refused:?}");
    // The refusal lapses after `ferry.refusal_s`.
    let (&cell, &until) = w.coord.ferry_refusals.iter().next().unwrap();
    let p = Pos::new(cell.0 * 192 + 1, cell.1 * 192 + 1);
    assert!(w.coord.landing_refused(p, 192, until - 1));
    assert!(!w.coord.landing_refused(p, 192, until));
}

// --- Navy (SPEC §5.12) ---

#[test]
fn the_fleet_sails_and_sinks_enemy_ships() {
    let (mut sim, mut ai) = game(sea(), true, &[], |_| {});
    spawn(&mut sim, P0, "ARMSY", Pos::new(600, 200));
    let fleet = spawn_row(&mut sim, P0, "ARMROY", Pos::new(600, 380), 3);
    let enemy = [
        spawn(&mut sim, P1, "CORROY", Pos::new(1300, 520)),
        spawn(&mut sim, P1, "CORROY", Pos::new(1340, 560)),
    ];
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 120 * S, &mut log, |_, _| {});
    // The fleet sails on the nearest enemy ship and sinks both.
    for &u in &fleet {
        let cmds = commands_to(&log, u);
        assert_eq!(
            cmds.first(),
            Some(&Command::AttackMove(Pos::new(1340, 560))),
            "{cmds:?}"
        );
    }
    for e in enemy {
        let sunk = sim.events.iter().any(|ev| {
            matches!(ev, Event::Died { unit, killer: Some(k), .. } if *unit == e && fleet.contains(k))
        });
        assert!(sunk, "sank {}", e.0);
    }
}

#[test]
fn a_hull_guards_the_commander_from_a_ship() {
    let (mut sim, mut ai) = game(sea(), true, &[], |_| {});
    spawn(&mut sim, P0, "ARMSY", Pos::new(600, 200));
    // Out of the hull's 450 engage radius, within the guard radius (1500)
    // of the commander, which the enemy ship is shelling.
    let hull = spawn(&mut sim, P0, "ARMROY", Pos::new(1000, 512));
    let raider_ship = spawn(&mut sim, P1, "CORROY", Pos::new(420, 512));
    let mut log = Vec::new();
    let mut danger = false;
    play(&mut sim, &mut ai, 2 * S, &mut log, |_, ai| {
        danger |= ai.world().army.commander_danger;
        assert!(!ai.world().army.commander_threats.is_empty() || !ai.world().army.commander_danger);
    });
    assert!(danger);
    let cmds = commands_to(&log, hull);
    assert_eq!(
        cmds.first(),
        Some(&Command::Attack(raider_ship)),
        "{cmds:?}"
    );
}

#[test]
fn a_hull_on_other_water_does_not_guard_the_commander() {
    // §5.6: the guard is for hulls "on the same water". A hull in a pond
    // behind the commander was sent after a ship on the sea it can neither
    // reach nor hit from where it floats.
    let map = Map::builder(128, 64)
        .water(TileRect::new(16, 0, 112, 64), 40)
        .water(TileRect::new(2, 54, 12, 64), 40)
        .start(Pos::new(128, 512))
        .start(Pos::new(1920, 512))
        .build()
        .unwrap();
    let (mut sim, mut ai) = game(map, true, &[], |_| {});
    spawn(&mut sim, P0, "ARMSY", Pos::new(600, 200));
    let pond_hull = spawn(&mut sim, P0, "ARMROY", Pos::new(112, 950));
    let sea_hull = spawn(&mut sim, P0, "ARMROY", Pos::new(1000, 300));
    let ship = spawn(&mut sim, P1, "CORROY", Pos::new(420, 300));
    let mut log = Vec::new();
    let mut danger = false;
    play(&mut sim, &mut ai, 2 * S, &mut log, |_, ai| {
        danger |= ai.world().army.commander_danger;
    });
    assert!(danger);
    assert_eq!(
        commands_to(&log, sea_hull).first(),
        Some(&Command::Attack(ship))
    );
    let pond = commands_to(&log, pond_hull);
    assert!(!pond.contains(&Command::Attack(ship)), "{pond:?}");
}
#[test]
fn a_lone_hull_holds_the_station() {
    let (mut sim, mut ai) = game(sea(), true, &[], |_| {});
    let yard = spawn(&mut sim, P0, "ARMSY", Pos::new(600, 200));
    let hull = spawn(&mut sim, P0, "ARMROY", Pos::new(1000, 700));
    let mut log = Vec::new();
    play(&mut sim, &mut ai, 2 * S, &mut log, |_, _| {});
    // One hull is short of `navy.sail_size` (3): it heads for the station,
    // 320 from the yard on deep water.
    let yard = where_is(&sim, P0, yard).unwrap();
    let cmds = commands_to(&log, hull);
    match cmds.first() {
        Some(Command::Move(p)) => assert_eq!(p.dist(yard), 320, "{p:?}"),
        other => panic!("{other:?}"),
    }
}

// --- Determinism (SPEC §7) ---

#[test]
fn two_ferry_runs_give_identical_order_logs() {
    let run = || {
        let (mut sim, mut ai, _, _) = ferry_game();
        let mut log = Vec::new();
        play(&mut sim, &mut ai, 60 * S, &mut log, |_, _| {});
        (log, sim.state_hash())
    };
    let (a, ha) = run();
    let (b, hb) = run();
    assert!(a.len() > 20, "{}", a.len());
    assert_eq!(a, b);
    assert_eq!(ha, hb);
}

#[test]
fn two_air_runs_give_identical_order_logs() {
    let run = || {
        let (mut sim, mut ai, _, _, _) = bomber_game(&[]);
        let mut log = Vec::new();
        play(&mut sim, &mut ai, 30 * S, &mut log, |_, _| {});
        (log, sim.state_hash())
    };
    assert_eq!(run(), run());
}
