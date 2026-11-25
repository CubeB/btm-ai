#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::view::{Command, FogLeaks, GameView, OrderRecorder, Orders, Queue};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

fn game(seed: u64) -> Sim {
    Sim::new(Map::two_player(), SimConfig::two_player(seed)).unwrap()
}

fn def(sim: &Sim, name: &str) -> DefId {
    sim.def_id(name).unwrap()
}

fn commander(sim: &Sim, p: PlayerId) -> UnitId {
    let v = sim.view(p);
    v.own_units()
        .into_iter()
        .find(|u| v.unit_def(u.def).unwrap().commander)
        .unwrap()
        .id
}

/// Removes income and empties stock so that spending is measurable.
fn quiet_economy(sim: &mut Sim, p: PlayerId, metal: i64, energy: i64) {
    let pl = &mut sim.players[usize::from(p.0)];
    pl.income_factor = 0;
    pl.metal.stock = metal * 1000 * SUB;
    pl.energy.stock = energy * 1000 * SUB;
}

fn order(sim: &mut Sim, p: PlayerId, unit: UnitId, command: Command) {
    sim.orders_for(p).now(unit, command);
}

#[test]
fn economy_income_is_exact() {
    let mut sim = game(1);
    let solar = def(&sim, "ARMSOLAR");
    let mex = def(&sim, "ARMMEX");
    sim.spawn(P0, solar, Pos::new(300, 120), true).unwrap();
    // The patch at tiles (6,13)-(7,14): 4 tiles × 500 milli-metal/s.
    let m = sim.spawn(P0, mex, Pos::new(112, 224), true).unwrap();
    assert_eq!(sim.units[&m].extract, 2000);
    sim.players[0].metal.stock = 0;
    sim.players[0].energy.stock = 0;
    sim.run(30);
    let eco = sim.view(P0).economy();
    // Commander 1.5 m/s + extractor 2 m/s; commander 25 e/s + solar 20 e/s.
    assert_eq!(eco.metal.income, Amount(3500));
    assert_eq!(eco.metal.stock, Amount(3500));
    assert_eq!(eco.energy.income, Amount::units(45));
    assert_eq!(eco.energy.stock, Amount::units(45));
    assert_eq!(eco.metal.storage, Amount::units(1000));
    assert!(!eco.metal.stalled);

    // A maker turns 60 e/s into 1 m/s while on.
    let maker = sim
        .spawn(P0, def(&sim, "ARMMAKR"), Pos::new(300, 260), true)
        .unwrap();
    sim.players[0].energy.stock = 500 * 1000 * SUB;
    sim.run(30);
    let eco = sim.view(P0).economy();
    assert_eq!(eco.metal.stock, Amount(3500 + 4500));
    assert_eq!(eco.energy.stock, Amount::units(500 + 45 - 60));
    assert_eq!(eco.energy.demand, Amount::units(60));
    order(&mut sim, P0, maker, Command::SetOn(false));
    sim.run(30);
    let eco = sim.view(P0).economy();
    assert_eq!(eco.metal.stock, Amount(8000 + 3500));

    // Storage caps the stock, and a storage building raises the cap.
    sim.run(30 * 400);
    let eco = sim.view(P0).economy();
    assert_eq!(eco.metal.stock, Amount::units(1000));
    sim.spawn(P0, def(&sim, "ARMMSTOR"), Pos::new(400, 400), true)
        .unwrap();
    sim.run(30);
    assert_eq!(sim.view(P0).economy().metal.storage, Amount::units(2000));
    assert!(sim.view(P0).economy().metal.stock > Amount::units(1000));
}

#[test]
fn build_completes_on_time_and_costs_exactly() {
    let mut sim = game(2);
    quiet_economy(&mut sim, P0, 1000, 1000);
    let solar = def(&sim, "ARMSOLAR");
    let cmd = commander(&sim, P0);
    let at = Pos::new(232, 168);
    order(&mut sim, P0, cmd, Command::Build { def: solar, at });
    // 2800 work at 300 work/s: 9.33 s, 280 ticks; the site is in reach.
    sim.run(279);
    let frame = sim
        .view(P0)
        .own_units()
        .into_iter()
        .find(|u| u.def == solar)
        .unwrap();
    assert!(frame.frame.is_some());
    assert_eq!(frame.frame.unwrap().total, 2800 * 30);
    sim.run(1);
    let done = sim
        .view(P0)
        .own_units()
        .into_iter()
        .find(|u| u.def == solar)
        .unwrap();
    assert!(done.frame.is_none());
    assert_eq!(done.health, 500);
    assert_eq!(sim.view(P0).economy().metal.stock, Amount::units(850));
    assert!(sim
        .events
        .iter()
        .any(|e| matches!(e, Event::Completed { unit, .. } if *unit == done.id)));
    sim.run(2);
    assert!(sim
        .view(P0)
        .own_units()
        .iter()
        .all(|u| u.commands.is_empty()));
}

#[test]
fn a_destroyed_frame_ends_its_build_order() {
    // Regression: a builder whose frame was killed placed a fresh frame
    // every tick under the same order (346 shipyard frames died in one AI
    // match), so the AI never saw the order fail.
    let mut sim = game(2);
    let solar = def(&sim, "ARMSOLAR");
    let cmd = commander(&sim, P0);
    order(
        &mut sim,
        P0,
        cmd,
        Command::Build {
            def: solar,
            at: Pos::new(232, 168),
        },
    );
    sim.run(60);
    let frame = sim.units.values().find(|u| u.def == solar).unwrap().id;
    sim.remove_unit(frame, None, false);
    sim.run(5);
    assert!(sim.units.values().all(|u| u.def != solar));
    assert!(sim.units[&cmd].cmds.is_empty());
    assert!(sim
        .rejected
        .iter()
        .any(|r| r.order.unit == cmd && r.reason == Reject::BadSite));
}

#[test]
fn build_stalls_without_metal() {
    let mut sim = game(3);
    quiet_economy(&mut sim, P0, 75, 1000);
    let solar = def(&sim, "ARMSOLAR");
    let cmd = commander(&sim, P0);
    order(
        &mut sim,
        P0,
        cmd,
        Command::Build {
            def: solar,
            at: Pos::new(232, 168),
        },
    );
    sim.run(600);
    let f = sim
        .view(P0)
        .own_units()
        .into_iter()
        .find(|u| u.def == solar)
        .unwrap();
    let p = f.frame.unwrap();
    assert!(p.done * 2 <= p.total && p.done * 2 > p.total - 300, "{p:?}");
    assert!(sim.view(P0).economy().metal.stalled);
}

#[test]
fn factory_produces_a_unit() {
    let mut sim = game(4);
    let lab = sim
        .spawn(P0, def(&sim, "ARMLAB"), Pos::new(320, 320), true)
        .unwrap();
    let pw = def(&sim, "ARMPW");
    order(
        &mut sim,
        P0,
        lab,
        Command::FactoryQueue { def: pw, count: 2 },
    );
    sim.run(1);
    let f = sim
        .view(P0)
        .own_units()
        .into_iter()
        .find(|u| u.id == lab)
        .unwrap();
    assert_eq!(f.factory.as_ref().unwrap().queue, vec![pw, pw]);
    // 1600 work at 100 work/s = 480 ticks each.
    sim.run(480);
    let made: Vec<_> = sim
        .view(P0)
        .own_units()
        .into_iter()
        .filter(|u| u.def == pw)
        .collect();
    assert_eq!(made.iter().filter(|u| u.frame.is_none()).count(), 1);
    order(
        &mut sim,
        P0,
        lab,
        Command::FactoryQueue { def: pw, count: -1 },
    );
    sim.run(100);
    let v = sim.view(P0);
    let f = v.own_units().into_iter().find(|u| u.id == lab).unwrap();
    assert!(f.factory.unwrap().queue.is_empty());
    assert_eq!(v.own_units().iter().filter(|u| u.def == pw).count(), 1);
    let first = made.iter().find(|u| u.frame.is_none()).unwrap();
    let now = v
        .own_units()
        .into_iter()
        .find(|u| u.id == first.id)
        .unwrap();
    assert!(
        now.pos.z > 320 + 48,
        "unit walked off the pad: {:?}",
        now.pos
    );
}

#[test]
fn an_assisted_factory_builds_each_queue_entry_once() {
    // Regression: when a guarding builder finished the factory's frame, the
    // factory kept the entry at the head of its queue and built it again.
    // Two assistants at 80 work/tick and the lab at 100 make the second
    // assistant land the last bit of work.
    let mut sim = game(4);
    let lab = sim
        .spawn(P0, def(&sim, "ARMLAB"), Pos::new(320, 320), true)
        .unwrap();
    let ck = def(&sim, "ARMCK");
    let pw = def(&sim, "ARMPW");
    for x in [300, 340] {
        let b = sim.spawn(P0, ck, Pos::new(x, 400), true).unwrap();
        order(&mut sim, P0, b, Command::Guard(lab));
    }
    order(
        &mut sim,
        P0,
        lab,
        Command::FactoryQueue { def: pw, count: 1 },
    );
    sim.run(1500);
    let v = sim.view(P0);
    assert_eq!(v.own_units().iter().filter(|u| u.def == pw).count(), 1);
    let f = v.own_units().into_iter().find(|u| u.id == lab).unwrap();
    assert!(f.factory.unwrap().queue.is_empty());
}

#[test]
fn combat_kills_and_leaves_a_wreck() {
    let mut sim = game(5);
    let a = def(&sim, "ARMPW");
    let c = def(&sim, "CORAK");
    let a1 = sim.spawn(P0, a, Pos::new(500, 480), true).unwrap();
    let a2 = sim.spawn(P0, a, Pos::new(500, 520), true).unwrap();
    let victim = sim.spawn(P1, c, Pos::new(600, 500), true).unwrap();
    sim.step();
    // 300 hp against two 12-damage guns firing every 10 ticks: 13 volleys.
    sim.run(200);
    let died = sim.events.iter().find_map(|e| match e {
        Event::Died {
            unit, killer, tick, ..
        } if *unit == victim => Some((*killer, *tick)),
        _ => None,
    });
    let (killer, tick) = died.expect("the lone raider dies");
    assert!(killer == Some(a1) || killer == Some(a2));
    assert!(tick < 150, "died at {tick}");
    assert!(sim.units.contains_key(&a1) && sim.units.contains_key(&a2));
    let wrecks = sim.view(P0).visible_features();
    assert_eq!(wrecks.len(), 1);
    assert_eq!(wrecks[0].kind, FeatureKind::Wreck(c));
    assert_eq!(wrecks[0].metal, Amount::units(30));
}

#[test]
fn reclaim_returns_feature_metal() {
    let mut sim = game(6);
    quiet_economy(&mut sim, P0, 0, 0);
    let wreck = sim.add_feature(FeatureKind::Rock, Pos::new(260, 160), 100, 0);
    let cmd = commander(&sim, P0);
    order(&mut sim, P0, cmd, Command::ReclaimFeature(wreck));
    // Walks ~28 units into reach, then drains 300 × 100 sub-units (1 metal)
    // per tick: 100 ticks.
    sim.run(150);
    assert!(sim.rejected.is_empty());
    assert!(!sim.features.contains_key(&wreck));
    assert_eq!(sim.view(P0).economy().metal.stock, Amount::units(100));
}

#[test]
fn reclaiming_a_unit_refunds_its_metal() {
    let mut sim = game(7);
    quiet_economy(&mut sim, P0, 0, 0);
    let mex = sim
        .spawn(P0, def(&sim, "ARMMEX"), Pos::new(208, 160), true)
        .unwrap();
    let cmd = commander(&sim, P0);
    order(&mut sim, P0, cmd, Command::ReclaimUnit(mex));
    sim.run(300);
    assert!(!sim.units.contains_key(&mex));
    let got = sim.view(P0).economy().metal.stock;
    assert!(got >= Amount(49_900) && got <= Amount::units(50), "{got:?}");
    assert!(sim.features.is_empty(), "reclaim leaves no wreck");
}

#[test]
fn movement_paths_round_impassable_cells() {
    let mut sim = game(8);
    let pw = sim
        .spawn(P0, def(&sim, "ARMPW"), Pos::new(320, 560), true)
        .unwrap();
    let goal = Pos::new(440, 560);
    order(&mut sim, P0, pw, Command::Move(goal));
    let mut arrived = false;
    for _ in 0..600 {
        sim.step();
        let p = sim.units[&pw].pos();
        let mc = sim.def(sim.units[&pw].def).move_class;
        assert!(sim.map.standable(p.tile(), mc), "stood on {p:?}");
        if sim.units[&pw].cmds.is_empty() {
            arrived = p.within(goal, 8);
            break;
        }
    }
    assert!(arrived);
}

#[test]
fn transport_loads_and_unloads() {
    let mut sim = game(9);
    let atlas = sim
        .spawn(P0, def(&sim, "ARMATLAS"), Pos::new(300, 300), true)
        .unwrap();
    let pw = sim
        .spawn(P0, def(&sim, "ARMPW"), Pos::new(400, 300), true)
        .unwrap();
    let drop = Pos::new(300, 700);
    let mut sink = sim.orders_for(P0);
    sink.now(atlas, Command::Load(pw));
    sink.queued(atlas, Command::Unload(drop));
    sim.run(60);
    assert_eq!(sim.units[&pw].carried_by, Some(atlas));
    assert!(sim.view(P1).enemy(pw).is_none());
    sim.run(200);
    assert_eq!(sim.units[&pw].carried_by, None);
    assert!(sim.units[&pw].pos().within(drop, 1));
    assert!(sim.units[&atlas].carrying.is_empty());
}

#[test]
fn bad_orders_are_rejected() {
    let mut sim = game(10);
    let theirs = commander(&sim, P1);
    let mine = commander(&sim, P0);
    let core_lab = def(&sim, "CORLAB");
    order(&mut sim, P0, theirs, Command::Stop);
    order(
        &mut sim,
        P0,
        mine,
        Command::Build {
            def: core_lab,
            at: Pos::new(300, 300),
        },
    );
    order(&mut sim, P0, mine, Command::Load(theirs));
    order(&mut sim, P0, UnitId(999), Command::Stop);
    sim.step();
    let reasons: Vec<Reject> = sim.rejected.iter().map(|r| r.reason).collect();
    assert_eq!(
        reasons,
        vec![
            Reject::NotOwner,
            Reject::CannotDo,
            Reject::CannotDo,
            Reject::NoSuchUnit
        ]
    );
}

#[test]
fn queued_orders_run_in_turn_and_delay_applies() {
    let mut cfg = SimConfig::two_player(11);
    cfg.order_delay = 7;
    let mut sim = Sim::new(Map::two_player(), cfg).unwrap();
    let cmd = commander(&sim, P0);
    let mut rec = OrderRecorder::new();
    rec.now(cmd, Command::Move(Pos::new(200, 160)));
    rec.queued(cmd, Command::Move(Pos::new(200, 200)));
    assert_eq!(rec.orders()[1].queue, Queue::Queued);
    sim.submit(P0, rec.take());
    sim.run(7);
    assert!(
        sim.units[&cmd].cmds.is_empty(),
        "not applied before the delay"
    );
    sim.step();
    assert_eq!(sim.units[&cmd].cmds.len(), 2);
    sim.run(120);
    assert!(sim.units[&cmd].pos().within(Pos::new(200, 200), 8));
}

#[test]
fn fog_hides_enemies_outside_line_of_sight() {
    let mut sim = game(12);
    let v = sim.view(P0);
    let theirs = commander(&sim, P1);
    assert!(v.visible_enemies().is_empty());
    assert!(v.enemy(theirs).is_none());
    assert!(!v.in_los(Pos::new(864, 864)));
    assert!(v.in_los(Pos::new(160, 160)));
    assert!(v.radar_dots().is_empty());
    assert!(!v.can_hit(commander(&sim, P0), theirs));

    // An enemy walks into sight; an enemy tower stays out of it.
    let raider = sim
        .spawn(P1, def(&sim, "CORAK"), Pos::new(400, 160), true)
        .unwrap();
    let tower = sim
        .spawn(P1, def(&sim, "CORLLT"), Pos::new(700, 300), true)
        .unwrap();
    sim.step();
    let v = sim.view(P0);
    let seen: Vec<UnitId> = v.visible_enemies().iter().map(|e| e.id).collect();
    assert_eq!(seen, vec![raider]);
    assert!(v.enemy(tower).is_none());
    assert!(v.can_hit(commander(&sim, P0), raider));
    // ...but the named placement leak sees it.
    let leaked: Vec<UnitId> = v
        .leak_buildings_near(Pos::new(700, 300), 50)
        .iter()
        .map(|b| b.id)
        .collect();
    assert_eq!(leaked, vec![tower]);
    assert!(!v.leak_can_place(def(&sim, "ARMLLT"), Pos::new(700, 300)));

    // The commander's radar (700) shows the tower as a dot, and only the
    // leak says that it is a building.
    let dots: Vec<UnitId> = v.radar_dots().iter().map(|d| d.id).collect();
    assert_eq!(dots, vec![tower]);
    assert_eq!(v.leak_radar_dot_mobile(tower), Some(false));
    assert_eq!(v.leak_radar_dot_mobile(raider), None);
}

#[test]
fn features_leak_only_through_named_methods() {
    let mut sim = game(13);
    // A scout explores a far cell, then dies.
    let scout = sim
        .spawn(P0, def(&sim, "ARMFAV"), Pos::new(600, 600), true)
        .unwrap();
    sim.step();
    sim.units.get_mut(&scout).unwrap().hp = 0;
    sim.step();
    let wreck = sim.add_feature(FeatureKind::Rock, Pos::new(600, 600), 20, 0);
    let far = sim.add_feature(FeatureKind::Tree, Pos::new(1000, 40), 0, 5);
    let v = sim.view(P0);
    assert!(v.cell_explored(Pos::new(600, 600).vis_cell()));
    assert!(!v.cell_visible(Pos::new(600, 600).vis_cell()));
    let ids = |fs: Vec<crate::view::Feature>| fs.iter().map(|f| f.id).collect::<Vec<_>>();
    // The scout's own wreck lies there too.
    assert!(ids(v.visible_features()).is_empty());
    assert!(ids(v.leak_explored_features()).contains(&wreck));
    assert!(!ids(v.leak_explored_features()).contains(&far));
    assert_eq!(ids(v.leak_features_near(Pos::new(1000, 40), 10)), vec![far]);
}

#[test]
fn allies_are_never_enemies_and_full_vision_sees_all() {
    let map = Map::builder(40, 40)
        .start(Pos::new(100, 100))
        .start(Pos::new(140, 100))
        .start(Pos::new(560, 560))
        .build()
        .unwrap();
    let mut cfg = SimConfig::two_player(14);
    cfg.players = vec![
        PlayerSetup {
            team: Some(TeamId(0)),
            ..PlayerSetup::new(Side::Arm)
        },
        PlayerSetup {
            team: Some(TeamId(0)),
            ..PlayerSetup::new(Side::Core)
        },
        PlayerSetup {
            full_vision: true,
            ..PlayerSetup::new(Side::Core)
        },
    ];
    let mut sim = Sim::new(map, cfg).unwrap();
    sim.step();
    let v = sim.view(P0);
    assert!(v.visible_enemies().is_empty());
    assert!(v.enemy(commander(&sim, P1)).is_none());
    assert!(v.players()[0].allied_with(&v.players()[1]));
    // Allies share sight.
    assert!(v.in_los(Pos::new(140 + 400, 100)));
    // Full vision sees both enemies.
    let cheat = sim.view(PlayerId(2));
    assert!(cheat.full_vision());
    assert_eq!(cheat.visible_enemies().len(), 2);
}

#[test]
fn fog_holds_throughout_a_rush_game() {
    let mut sim = game(15);
    sim.set_script(P0, Script::rush());
    sim.set_script(P1, Script::rush());
    let mut sightings = 0;
    for t in 0..4500 {
        sim.step();
        if t % 15 != 0 {
            continue;
        }
        for p in [P0, P1] {
            let v = sim.view(p);
            let eyes: Vec<(Pos, i64)> = sim
                .units
                .values()
                .filter(|u| u.owner == p && u.frame.is_none() && u.carried_by.is_none())
                .map(|u| (u.pos(), i64::from(sim.def(u.def).sight)))
                .collect();
            for e in v.visible_enemies() {
                sightings += 1;
                // A seen unit lies in a cell whose centre is in someone's
                // sight, so within sight + half a cell diagonal of them.
                assert!(
                    eyes.iter().any(|(at, r)| at.within(e.pos, r + 23)),
                    "player {} saw {:?} out of sight",
                    p.0,
                    e
                );
                assert!(v.in_los(e.pos));
            }
            for d in v.radar_dots() {
                assert!(!v.in_los(d.pos) && v.enemy(d.id).is_none());
            }
        }
    }
    assert!(sightings > 0, "the rushes should meet");
}

#[test]
fn passive_script_builds_its_base() {
    let mut sim = game(16);
    sim.set_script(P1, Script::passive());
    sim.run(30 * 120);
    let v = sim.view(P1);
    let count = |n: &str| {
        let d = v.def_named(n).unwrap();
        v.own_units()
            .iter()
            .filter(|u| u.def == d && u.frame.is_none())
            .count()
    };
    assert_eq!(count("CORMEX"), 2);
    assert_eq!(count("CORSOLAR"), 2);
    assert!(v.economy().metal.income > Amount(5000));
}

#[test]
fn same_seed_same_game() {
    let run = |seed: u64| {
        let mut cfg = SimConfig::two_player(seed);
        cfg.random_starts = true;
        let mut sim = Sim::new(Map::two_player(), cfg).unwrap();
        let seat = sim.draw_seat_seed();
        sim.set_script(P0, Script::rush());
        sim.set_script(P1, Script::rush());
        sim.run(3000);
        (sim.state_hash(), seat, sim.units.len(), sim.events.len())
    };
    let a = run(42);
    let b = run(42);
    assert_eq!(a, b);
    assert!(a.2 > 4 && a.3 > 4, "the game did something: {a:?}");
    assert_ne!(run(43).1, a.1);
}
