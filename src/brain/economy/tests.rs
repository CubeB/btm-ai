//! Economy behaviour in the test sim, through [`Ai`] with the army stub
//! (every §5 slot quiet), against `Script::passive()`.
//!
//! The assertions are on behaviour the spec states (the opening order, rule
//! 0, rule E, rule 21, the reclaim rules, personality knobs, determinism),
//! not on incidental counts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::super::{Ai, Ctx, Manager, Source, Staging, DEFAULT_ORDER_DELAY};
use crate::personality::{resolve, Difficulty, Faction, Personality, Registry};
use crate::sim::{Event, Map, Script, Sim, SimConfig};
use crate::view::{
    Amount, Command, DefId, GameView, Order, OrderRecorder, PlayerId, Pos, Tick, UnitId,
    TICKS_PER_SECOND,
};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const FIVE_MINUTES: Tick = 5 * 60 * TICKS_PER_SECOND;

/// The army stub: every §5 slot does nothing.
struct Quiet;

impl Manager for Quiet {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}

fn personality(name: &str) -> Personality {
    match name {
        "test" => Personality::new("test"),
        other => Registry::builtin().get(other).unwrap().clone(),
    }
}

fn config(seed: u64) -> SimConfig {
    let mut cfg = SimConfig::two_player(seed);
    cfg.order_delay = DEFAULT_ORDER_DELAY;
    cfg
}

/// A game: the passive script on P1 (CORE), the AI on P0 (ARM).
fn game_with(cfg: SimConfig, who: &str) -> (Sim, Ai) {
    let mut sim = Sim::new(Map::two_player(), cfg).unwrap();
    let draw = sim.draw_seat_seed();
    sim.set_script(P1, Script::passive());
    let tuning = resolve(
        &personality(who),
        Difficulty::Standard,
        Faction::from_name("ARM"),
        &[],
    )
    .unwrap();
    let mut ai = Ai::new(P0, &tuning, u64::from(draw)).unwrap();
    for slot in [
        Source::Army,
        Source::Navy,
        Source::Scouts,
        Source::Transport,
        Source::Air,
    ] {
        ai.set_manager(slot, Box::new(Quiet));
    }
    (sim, ai)
}

fn game(seed: u64, who: &str) -> (Sim, Ai) {
    game_with(config(seed), who)
}

/// What we saw each tick, before the AI ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Snap {
    tick: Tick,
    energy_stalled: bool,
    metal_stalled: bool,
    /// Metal stock, permille of storage.
    metal_fill: i64,
}

#[derive(Default)]
struct Log {
    orders: Vec<(Tick, Order)>,
    snaps: Vec<Snap>,
}

fn play(sim: &mut Sim, ai: &mut Ai, ticks: Tick, log: &mut Log) {
    for _ in 0..ticks {
        let mut rec = OrderRecorder::new();
        let t = sim.tick();
        {
            let v = sim.view(P0);
            let e = v.economy();
            log.snaps.push(Snap {
                tick: t,
                energy_stalled: e.energy.stalled,
                metal_stalled: e.metal.stalled,
                metal_fill: e.metal.fill_permille(),
            });
            ai.tick(&v, &v, &mut rec).unwrap();
        }
        let orders = rec.take();
        log.orders.extend(orders.iter().map(|&o| (t, o)));
        sim.submit(P0, orders);
        sim.step();
    }
}

fn run(sim: &mut Sim, ai: &mut Ai, ticks: Tick) -> Log {
    let mut log = Log::default();
    play(sim, ai, ticks, &mut log);
    log
}

fn def(sim: &Sim, name: &str) -> DefId {
    sim.def_id(name).unwrap()
}

/// Build orders given to `unit` (any unit if `None`): (tick, type, site).
fn builds_by(log: &Log, unit: Option<UnitId>) -> Vec<(Tick, DefId, Pos)> {
    log.orders
        .iter()
        .filter(|(_, o)| unit.is_none_or(|u| o.unit == u))
        .filter_map(|&(t, o)| match o.command {
            Command::Build { def, at } => Some((t, def, at)),
            _ => None,
        })
        .collect()
}

/// Units of a type a player has: (finished, total including frames).
fn count(sim: &Sim, p: PlayerId, unit: &str) -> (usize, usize) {
    let v = sim.view(p);
    let d = v.def_named(unit).unwrap();
    let units = v.own_units();
    let all = units.iter().filter(|u| u.def == d);
    let done = all.clone().filter(|u| u.frame.is_none()).count();
    (done, all.count())
}

fn commander(sim: &Sim) -> UnitId {
    let v = sim.view(P0);
    v.own_units()
        .into_iter()
        .find(|u| v.unit_def(u.def).unwrap().commander)
        .unwrap()
        .id
}

/// The first tick a unit of `unit` was completed for `p`.
fn completed_at(sim: &Sim, p: PlayerId, unit: DefId) -> Option<Tick> {
    sim.events.iter().find_map(|e| match *e {
        Event::Completed {
            tick, owner, def, ..
        } if owner == p && def == unit => Some(tick),
        _ => None,
    })
}

#[test]
fn the_opening_is_three_extractors_four_solars_then_the_lab() {
    let (mut sim, mut ai) = game(1, "test");
    let cmdr = commander(&sim);
    let log = run(&mut sim, &mut ai, FIVE_MINUTES);
    let (mex, solar, lab) = (
        def(&sim, "ARMMEX"),
        def(&sim, "ARMSOLAR"),
        def(&sim, "ARMLAB"),
    );

    // No stall before the lab was ordered, so the plain opening applies
    // (SPEC §4.3: solars first only when energy is stalled).
    let first8: Vec<DefId> = builds_by(&log, Some(cmdr))
        .iter()
        .take(8)
        .map(|b| b.1)
        .collect();
    let lab_ordered = builds_by(&log, Some(cmdr))
        .iter()
        .find(|b| b.1 == lab)
        .map(|b| b.0)
        .unwrap();
    assert!(log
        .snaps
        .iter()
        .filter(|s| s.tick <= lab_ordered)
        .all(|s| !s.energy_stalled));
    assert_eq!(
        first8,
        vec![mex, mex, mex, solar, solar, solar, solar, lab],
        "the opening"
    );
    // Every extractor of the opening sits on a metal patch.
    let w = ai.world();
    for &(_, d, at) in builds_by(&log, Some(cmdr)).iter().take(3) {
        assert_eq!(d, mex);
        assert!(
            w.map.deposits.iter().any(|dep| dep.center.within(at, 32)),
            "{at:?} is on a deposit"
        );
    }
    // By five minutes the opening stands.
    assert!(count(&sim, P0, "ARMMEX").0 >= 3);
    assert!(count(&sim, P0, "ARMSOLAR").0 >= 4);
    assert!(count(&sim, P0, "ARMLAB").0 >= 1);

    // The lab's first order is a constructor (SPEC §4.3).
    let lab_id = sim
        .view(P0)
        .own_units()
        .iter()
        .find(|u| u.def == lab)
        .unwrap()
        .id;
    let first_queue = log
        .orders
        .iter()
        .find(|(_, o)| o.unit == lab_id)
        .map(|(_, o)| o.command);
    assert_eq!(
        first_queue,
        Some(Command::FactoryQueue {
            def: def(&sim, "ARMCK"),
            count: 1
        })
    );
}

#[test]
fn expansion_claims_more_metal_than_the_passive_script() {
    let (mut sim, mut ai) = game(2, "test");
    let cmdr = commander(&sim);
    let log = run(&mut sim, &mut ai, FIVE_MINUTES);
    let mex = def(&sim, "ARMMEX");
    let ours = count(&sim, P0, "ARMMEX").1;
    let theirs = count(&sim, P1, "CORMEX").1;
    // The passive script stops at two extractors.
    assert!(ours > theirs.max(2), "ours {ours}, theirs {theirs}");
    // Rule E: a constructor after the first is pushed out to expand; its
    // first build is an extractor.
    let ck = def(&sim, "ARMCK");
    let mut cks: Vec<UnitId> = sim
        .view(P0)
        .own_units()
        .iter()
        .filter(|u| u.def == ck && u.frame.is_none())
        .map(|u| u.id)
        .collect();
    cks.sort();
    assert!(cks.len() >= 2, "a second constructor was built");
    let second = builds_by(&log, Some(cks[1]));
    assert_eq!(second.first().map(|b| b.1), Some(mex), "{second:?}");
    // Every extractor order targets a different site.
    let mut sites: Vec<Pos> = builds_by(&log, None)
        .iter()
        .filter(|b| b.1 == mex)
        .map(|b| b.2)
        .collect();
    let n = sites.len();
    sites.sort();
    sites.dedup();
    assert_eq!(sites.len(), n);
    let _ = cmdr;
}

/// The longest run of consecutive ticks with energy stalled.
fn longest_stall(log: &Log) -> Tick {
    let (mut best, mut run) = (0, 0);
    for s in &log.snaps {
        run = if s.energy_stalled { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

#[test]
fn energy_does_not_stall_and_a_stall_brings_a_solar_first() {
    // The default game: energy never stalls past one emergency job. The
    // tolerance is the spec's reaction time: rule 0 makes a solar the next
    // job, and the commander takes at most one opening job before it.
    let (mut sim, mut ai) = game(3, "test");
    let log = run(&mut sim, &mut ai, FIVE_MINUTES);
    let tolerance = 30 * TICKS_PER_SECOND;
    assert!(longest_stall(&log) <= tolerance, "{}", longest_stall(&log));

    // Starting with no energy and three extra constructors forces a long
    // stall: every job a base builder (the commander, or the first
    // constructor) is given while energy is stalled is a solar (SPEC §4.3
    // rule 0). Later constructors put the extractor ahead of everything
    // (rule E), so they are left out.
    let mut cfg = config(3);
    cfg.start_energy = Amount::ZERO;
    let (mut sim, mut ai) = game_with(cfg, "test");
    let cmdr = commander(&sim);
    let ck = def(&sim, "ARMCK");
    let cks: Vec<UnitId> = (0..3)
        .map(|i| {
            sim.spawn(P0, ck, Pos::new(200 + i * 30, 220), true)
                .unwrap()
        })
        .collect();
    let log = run(&mut sim, &mut ai, 3 * 60 * TICKS_PER_SECOND);
    let solar = def(&sim, "ARMSOLAR");
    let stalled_at = |t: Tick| {
        log.snaps
            .iter()
            .find(|s| s.tick == t)
            .is_some_and(|s| s.energy_stalled)
    };
    let base_builders = [cmdr, cks[0]];
    let in_stall: Vec<_> = log
        .orders
        .iter()
        .filter(|(t, o)| base_builders.contains(&o.unit) && stalled_at(*t))
        .filter_map(|&(t, o)| match o.command {
            Command::Build { def, .. } => Some((t, o.unit, def)),
            _ => None,
        })
        .collect();
    assert!(!in_stall.is_empty(), "the stall was answered");
    assert!(in_stall.iter().all(|b| b.2 == solar), "{in_stall:?}");
}

#[test]
fn the_second_factory_comes_when_rule_21_holds() {
    let (mut sim, mut ai) = game(4, "test");
    let log = run(&mut sim, &mut ai, FIVE_MINUTES);
    let (lab, vp) = (def(&sim, "ARMLAB"), def(&sim, "ARMVP"));
    let vp_orders: Vec<_> = builds_by(&log, None)
        .into_iter()
        .filter(|b| b.1 == vp)
        .collect();
    assert!(!vp_orders.is_empty(), "a vehicle plant was started");
    let lab_done = completed_at(&sim, P0, lab).unwrap();
    let first = vp_orders[0].0;
    // Rule 21: a factory exists, opening energy, not metal short, land map.
    assert!(first >= lab_done, "after the lab: {first} vs {lab_done}");
    let snap = log.snaps.iter().find(|s| s.tick == first).unwrap();
    assert!(
        !snap.metal_stalled && snap.metal_fill >= 100,
        "not metal short: {snap:?}"
    );
    let w = ai.world();
    assert_eq!(w.map.character, super::super::MapCharacter::Land);
    // Only one vehicle plant (`build.vehicle_plants` 1).
    assert_eq!(count(&sim, P0, "ARMVP").1, 1);
    // It builds its scout vehicle first (SPEC §4.4).
    let plant = sim
        .view(P0)
        .own_units()
        .into_iter()
        .find(|u| u.def == vp && u.frame.is_none());
    if let Some(p) = plant {
        let first_queue = log
            .orders
            .iter()
            .find(|(_, o)| o.unit == p.id)
            .map(|(_, o)| o.command);
        assert_eq!(
            first_queue,
            Some(Command::FactoryQueue {
                def: def(&sim, "ARMFAV"),
                count: 1
            })
        );
    }
}

/// Defence structures (towers, AA towers, walls), finished or not.
fn defence(sim: &Sim) -> usize {
    ["ARMLLT", "ARMRL", "ARMDRAG"]
        .iter()
        .map(|u| count(sim, P0, u).1)
        .sum()
}

#[test]
fn a_turtle_builds_more_defence_than_a_rush() {
    let ticks = 7 * 60 * TICKS_PER_SECOND;
    let (mut sim, mut ai) = game(5, "Turtle");
    run(&mut sim, &mut ai, ticks);
    let turtle = defence(&sim);
    let turtle_llt = count(&sim, P0, "ARMLLT").1;
    let (mut sim, mut ai) = game(5, "Rush");
    run(&mut sim, &mut ai, ticks);
    let rush = defence(&sim);
    let rush_llt = count(&sim, P0, "ARMLLT").1;
    assert!(turtle > rush, "turtle {turtle}, rush {rush}");
    // Rush sets `def.towers` 0: no light tower at base.
    assert!(
        turtle_llt > rush_llt,
        "turtle {turtle_llt}, rush {rush_llt}"
    );
}

#[test]
fn a_wreck_on_a_metal_spot_is_reclaimed_before_the_extractor() {
    // A wreck covering the nearest deposit: the builder reclaims it and the
    // extractor is queued behind it (SPEC §4.2 "issue the job").
    let (mut sim, mut ai) = game(8, "test");
    let raider = def(&sim, "ARMPW");
    let home = Pos::new(160, 160);
    let nearest = {
        let v = sim.view(P0);
        let mut spots: Vec<Pos> = v.metal_spots().iter().map(|(t, _)| t.center()).collect();
        spots.sort_by_key(|p| (p.dist2(home), p.x, p.z));
        spots[0]
    };
    let wreck = sim.add_feature(crate::view::FeatureKind::Wreck(raider), nearest, 30, 0);
    let log = run(&mut sim, &mut ai, 60 * TICKS_PER_SECOND);
    let cmdr = commander(&sim);
    let mine: Vec<Order> = log
        .orders
        .iter()
        .filter(|(_, o)| o.unit == cmdr)
        .map(|(_, o)| *o)
        .collect();
    let i = mine
        .iter()
        .position(|o| o.command == Command::ReclaimFeature(wreck))
        .expect("the wreck is reclaimed");
    let next = mine.get(i + 1).copied().unwrap();
    let mex = def(&sim, "ARMMEX");
    assert!(matches!(next.command, Command::Build { def, .. } if def == mex));
    assert_eq!(next.queue, crate::view::Queue::Queued);
}

fn order_log(seed: u64) -> (Vec<(Tick, Order)>, u64) {
    let (mut sim, mut ai) = game(seed, "test");
    let log = run(&mut sim, &mut ai, FIVE_MINUTES);
    (log.orders, sim.state_hash())
}

#[test]
fn two_runs_give_identical_order_logs() {
    let (a, ha) = order_log(9);
    let (b, hb) = order_log(9);
    assert!(a.len() > 20);
    assert_eq!(a, b);
    assert_eq!(ha, hb);
}

#[test]
fn every_build_is_staged_by_the_economy_at_most_once_per_plan_pass() {
    // One builder job per planning pass (SPEC §4.2): at most one builder
    // gets a new build order on any tick, and only on plan ticks.
    let (mut sim, mut ai) = game(10, "test");
    let log = run(&mut sim, &mut ai, FIVE_MINUTES);
    let plan = Tick::try_from(ai.knobs().cadence.plan_ticks).unwrap();
    let mut per_tick: std::collections::BTreeMap<Tick, Vec<UnitId>> = Default::default();
    for (t, o) in &log.orders {
        if matches!(o.command, Command::Build { .. }) && o.queue == crate::view::Queue::Immediate {
            per_tick.entry(*t).or_default().push(o.unit);
        }
    }
    for (t, units) in per_tick {
        assert!(t % plan == 0, "build on tick {t}");
        let mut u = units.clone();
        u.dedup();
        assert_eq!(u.len(), 1, "tick {t}: {units:?}");
    }
}
