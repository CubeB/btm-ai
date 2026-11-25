//! The cadence harness: the AI in the test sim against `Script::passive()`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use super::*;
use crate::personality::{resolve, Faction, Personality};
use crate::sim::{Map, Script, Sim, SimConfig};
use crate::view::{Command, Order, OrderRecorder, Queue, TICKS_PER_SECOND};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const THREE_MINUTES: Tick = 3 * 60 * TICKS_PER_SECOND;

fn tuning(d: Difficulty) -> Resolved {
    resolve(&Personality::new("test"), d, Faction::from_name("ARM"), &[]).unwrap()
}

/// A game with the passive script on P1 and the real AI seat on P0. The
/// economy slot is [`Quiet`]: these tests exercise the core, and were
/// written against the stub economy (economy behaviour is tested in
/// `economy::tests`).
fn game(seed: u64, d: Difficulty) -> (Sim, Ai) {
    let mut cfg = SimConfig::two_player(seed);
    cfg.order_delay = DEFAULT_ORDER_DELAY;
    let mut sim = Sim::new(Map::two_player(), cfg).unwrap();
    let draw = sim.draw_seat_seed();
    sim.set_script(P1, Script::passive());
    let mut ai = Ai::new(P0, &tuning(d), u64::from(draw)).unwrap();
    ai.set_manager(Source::Economy, Box::new(Quiet));
    (sim, ai)
}

/// Ticks the AI and the sim together; returns the orders sent, with ticks.
fn play(sim: &mut Sim, ai: &mut Ai, ticks: Tick) -> Vec<(Tick, Order)> {
    let mut log = Vec::new();
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
    }
    log
}

fn ticks_of(ai: &Ai, step: Step) -> Vec<Tick> {
    ai.pass_log()
        .iter()
        .filter(|&&(_, s)| s == step)
        .map(|&(t, _)| t)
        .collect()
}

fn every(first: Tick, step: Tick, end: Tick) -> Vec<Tick> {
    (first..end).step_by(step as usize).collect()
}

fn spawn_raiders(sim: &mut Sim, n: i32) -> Vec<UnitId> {
    let raider = sim.def_id("ARMPW").unwrap();
    (0..n)
        .map(|i| {
            sim.spawn(
                P0,
                raider,
                Pos::new(200 + (i % 4) * 40, 260 + (i / 4) * 40),
                true,
            )
            .unwrap()
        })
        .collect()
}

/// A manager that does nothing.
struct Quiet;

impl Manager for Quiet {
    fn update(&mut self, _ctx: &mut Ctx<'_>, _out: &mut Staging) {}
}

#[test]
fn three_minutes_of_stub_managers_follow_the_cadence() {
    let (mut sim, mut ai) = game(11, Difficulty::Standard);
    ai.record_passes(true);
    let log = play(&mut sim, &mut ai, THREE_MINUTES);
    let end = THREE_MINUTES;

    // Every-tick steps, and the stubs were called at every one of their
    // steps (the counts are of calls into the slot's manager).
    let s = *ai.stats();
    assert_eq!(s.ticks, u64::from(end));
    for step in [
        Step::Sample,
        Step::Contacts,
        Step::Phase,
        Step::MetalMakers,
        Step::Dispatch,
    ] {
        assert_eq!(s.runs(step), u64::from(end), "{step:?}");
    }
    assert_eq!(ticks_of(&ai, Step::Grid), every(0, 30, end));
    assert_eq!(ticks_of(&ai, Step::Plan), every(0, 30, end));
    assert_eq!(ticks_of(&ai, Step::Sweep), every(5, 15, end));
    assert_eq!(ticks_of(&ai, Step::Scouts), every(10, 60, end));
    // The first tactics pass is on tick 15, not tick 0.
    for step in [Step::Transports, Step::Army, Step::Navy, Step::Air] {
        assert_eq!(ticks_of(&ai, step), every(15, 15, end), "{step:?}");
    }
    // Reachability: first chance (tick 0, the commander is there), then 20 s.
    assert_eq!(ticks_of(&ai, Step::Reach), every(0, 600, end));

    // Within each tick the steps run in pipeline order.
    let mut by_tick: BTreeMap<Tick, Vec<Step>> = BTreeMap::new();
    for &(t, step) in ai.pass_log() {
        by_tick.entry(t).or_default().push(step);
    }
    for (t, steps) in &by_tick {
        assert!(steps.windows(2).all(|w| w[0] < w[1]), "tick {t}: {steps:?}");
    }

    // The stubs order nothing.
    assert!(log.is_empty());
    assert_eq!(s.dispatch.staged, 0);

    // The core filled the shared state.
    let w = ai.world();
    assert_eq!(w.own.home, Some(Pos::new(160, 160)));
    assert!(w.own.commander.is_some());
    assert_eq!(w.own.idle_builders, vec![w.own.commander.unwrap()]);
    assert_eq!(w.enemies, vec![P1]);
    assert_eq!(w.map.character, MapCharacter::Land);
    // Four 2 × 2 patches per side and one in the middle.
    assert_eq!(w.map.deposits.len(), 9);
    assert!(w.map.deposits.iter().all(|d| d.tiles.len() == 4));
    assert_eq!(w.map.water_bodies, 2);
    // Each 14 × 12 lake, 40 deep, fits 7 × 5 shipyard footprints; none is
    // open (the ring runs onto dry land).
    assert_eq!(w.map.shipyard_sites.len(), 70);
    assert!(w.map.shipyard_sites.iter().all(|s| !s.open));
    let ground = w.reach.ground.as_ref().unwrap();
    assert_eq!(ground.mover, sim.def_id("ARMCK").unwrap());
    assert!(w.reach.land_route_to_enemy);
    assert!(w.reach.ground_reachable(Pos::new(864, 864)));
    assert!(w.reach.naval.is_none() && w.reach.hover.is_none());
    assert_eq!(w.roles.get(Role::KbotLab), sim.def_id("ARMLAB"));
    assert_eq!(w.roles.get(Role::Fusion), None);
    assert_eq!(w.grid.rebuilds, 180);
}

#[test]
fn tiers_change_the_cadence() {
    for (d, plan, tactics, scouts) in [
        (Difficulty::Easy, 60, 30, 120),
        (Difficulty::Hard, 20, 10, 45),
    ] {
        let (mut sim, mut ai) = game(3, d);
        ai.record_passes(true);
        play(&mut sim, &mut ai, 600);
        assert_eq!(ticks_of(&ai, Step::Plan), every(0, plan, 600), "{d:?}");
        assert_eq!(ticks_of(&ai, Step::Army), every(15, tactics, 600), "{d:?}");
        assert_eq!(ticks_of(&ai, Step::Scouts), every(10, scouts, 600), "{d:?}");
    }
}

#[test]
fn idle_seat_and_seat_errors() {
    let (mut sim, mut ai) = game(5, Difficulty::Idle);
    let log = play(&mut sim, &mut ai, 100);
    assert!(log.is_empty());
    assert_eq!(ai.stats().ticks, 0);

    let (sim, mut ai) = game(5, Difficulty::Standard);
    let v = sim.view(P1);
    let mut rec = OrderRecorder::new();
    assert_eq!(
        ai.tick(&v, &v, &mut rec),
        Err(BrainError::WrongSeat {
            expected: P0,
            got: P1
        })
    );
    let v = sim.view(P0);
    ai.tick(&v, &v, &mut rec).unwrap();
    // A second call on the same tick is a no-op.
    ai.tick(&v, &v, &mut rec).unwrap();
    assert_eq!(ai.stats().ticks, 1);

    let mut bad = tuning(Difficulty::Standard);
    bad.knobs.cadence.tactics_ticks = -1;
    assert!(matches!(
        Ai::new(P0, &bad, 1),
        Err(BrainError::Cadence(CadenceError {
            knob: "cadence.tactics_ticks",
            value: -1
        }))
    ));
}

/// Records every call: (tick, step, due).
type Calls = Rc<RefCell<Vec<(Tick, Step, Due, bool)>>>;

struct Probe(Calls);

impl Manager for Probe {
    fn update(&mut self, ctx: &mut Ctx<'_>, _out: &mut Staging) {
        let filled = ctx.home().is_some() && ctx.world.own.commander.is_some();
        self.0
            .borrow_mut()
            .push((ctx.tick, ctx.step, ctx.due, filled));
    }
}

#[test]
fn every_slot_is_called_at_its_steps_after_the_core() {
    let (mut sim, mut ai) = game(2, Difficulty::Standard);
    let calls = Rc::new(RefCell::new(Vec::new()));
    for slot in Source::ALL {
        ai.set_manager(slot, Box::new(Probe(calls.clone())));
    }
    play(&mut sim, &mut ai, 300);
    let calls = calls.borrow();
    assert!(
        calls.iter().all(|c| c.3),
        "core state filled before managers"
    );
    let at =
        |step: Step| -> Vec<Tick> { calls.iter().filter(|c| c.1 == step).map(|c| c.0).collect() };
    assert_eq!(at(Step::Phase), every(0, 1, 300));
    assert_eq!(at(Step::MetalMakers), every(0, 1, 300));
    assert_eq!(at(Step::Plan), every(0, 30, 300));
    assert_eq!(at(Step::Army), every(15, 15, 300));
    for c in calls.iter() {
        match c.1 {
            Step::Plan => assert!(c.2.plan),
            Step::Sweep => assert!(c.2.sweep),
            Step::Scouts => assert!(c.2.scouts),
            Step::Transports | Step::Army | Step::Navy | Step::Air => assert!(c.2.tactics),
            _ => {}
        }
    }
}

/// Moves every combat unit to one of two far points, alternating per pass,
/// so that no pass repeats the last one and de-duplication cannot help.
struct Spam(u32);

impl Manager for Spam {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        if ctx.step != Step::Army {
            return;
        }
        self.0 += 1;
        let flip = self.0.is_multiple_of(2);
        let to = if flip {
            Pos::new(450, 200)
        } else {
            Pos::new(200, 450)
        };
        for &u in &ctx.world.own.combat {
            out.now(u, Command::Move(to));
        }
    }
}

fn micro_sent(log: &[(Tick, Order)]) -> Vec<Tick> {
    log.iter()
        .filter(|(_, o)| o.command.is_micro())
        .map(|&(t, _)| t)
        .collect()
}

#[test]
fn the_limiter_caps_micro_orders_per_minute() {
    let (mut sim, mut ai) = game(4, Difficulty::Standard);
    let raiders = spawn_raiders(&mut sim, 12);
    ai.set_manager(Source::Army, Box::new(Spam(0)));
    let log = play(&mut sim, &mut ai, THREE_MINUTES);
    let sent = micro_sent(&log);
    // No 60 s window holds more than 120, and the burst is 4.
    for &t in &sent {
        let n = sent.iter().filter(|&&s| s >= t && s < t + 1800).count();
        assert!(n <= 120, "{n} orders in the minute from tick {t}");
    }
    assert!(sent.iter().filter(|&&t| t < 30).count() <= 4 + 2);
    // Demand far exceeds the cap, so the cap is reached.
    assert!(sent.len() >= 340, "{}", sent.len());
    assert!(sent.len() <= 4 + 360, "{}", sent.len());
    let s = ai.stats().dispatch;
    assert_eq!(s.limiter_admitted, sent.len() as u64);
    assert!(s.limiter_refused > 1000, "{s:?}");
    // Least recently served first: every unit keeps getting orders.
    for r in raiders {
        let n = log.iter().filter(|(_, o)| o.unit == r).count();
        assert!(n >= 20, "unit {} got {n}", r.0);
    }

    // Hard has no limit: every staged order goes out.
    let (mut sim, mut ai) = game(4, Difficulty::Hard);
    spawn_raiders(&mut sim, 12);
    ai.set_manager(Source::Army, Box::new(Spam(0)));
    let log = play(&mut sim, &mut ai, 600);
    let passes = every(15, 10, 600).len();
    assert_eq!(micro_sent(&log).len(), 12 * passes);
    assert_eq!(ai.stats().dispatch.limiter_refused, 0);
}

/// Stages the same order every tick from the economy (the commander) and
/// also tries orders it may not give; records the in-flight view.
struct Repeat {
    seen: Rc<RefCell<Vec<(Tick, bool, bool)>>>,
}

impl Manager for Repeat {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let Some(c) = ctx.world.own.commander else {
            return;
        };
        match ctx.step {
            Step::MetalMakers => {
                let idle = ctx.world.own.idle_builders.contains(&c);
                self.seen
                    .borrow_mut()
                    .push((ctx.tick, ctx.in_flight(c).is_some(), idle));
                let far = Command::Move(Pos::new(900, 160));
                // A consecutive duplicate and a repeat of last tick's group.
                out.set_priority(Priority::Tasked);
                out.now(c, far);
                out.now(c, far);
                // The economy may not order the army's raiders.
                for &u in &ctx.world.own.combat {
                    out.now(u, Command::Move(Pos::new(0, 0)));
                }
            }
            Step::Army => {
                // Routine army orders for the shared commander lose to the
                // planner's Tasked job.
                out.now(c, Command::Move(Pos::new(160, 900)));
            }
            _ => {}
        }
    }
}

#[test]
fn dedup_order_delay_ownership_and_conflicts() {
    let (mut sim, mut ai) = game(6, Difficulty::Hard);
    spawn_raiders(&mut sim, 2);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let m = || Box::new(Repeat { seen: seen.clone() });
    ai.set_manager(Source::Economy, m());
    ai.set_manager(Source::Army, m());
    let log = play(&mut sim, &mut ai, 300);
    let cmdr = ai.world().own.commander.unwrap();
    // One move goes out, once: in flight for 7 ticks, then visible in the
    // queue, so every later repeat is a no-op.
    let to_cmdr: Vec<_> = log.iter().filter(|(_, o)| o.unit == cmdr).collect();
    assert_eq!(to_cmdr.len(), 1, "{to_cmdr:?}");
    assert_eq!(to_cmdr[0].0, 0);
    assert_eq!(to_cmdr[0].1.command, Command::Move(Pos::new(900, 160)));
    assert_eq!(to_cmdr[0].1.queue, Queue::Immediate);
    let seen = seen.borrow();
    for &(t, flying, idle) in seen.iter().filter(|s| s.0 >= 1 && s.0 <= 7) {
        assert!(flying, "tick {t}: in flight");
        assert!(
            !idle,
            "tick {t}: a builder with orders in flight is not idle"
        );
    }
    assert!(!seen.iter().any(|s| s.0 == 8 && s.1));
    let s = ai.stats().dispatch;
    assert!(s.duplicate >= 2 * 299, "{s:?}");
    assert!(s.not_owner >= 2 * 300, "{s:?}");
    assert!(s.conflict >= 29, "{s:?}");
    assert!(log.iter().all(|(_, o)| o.unit == cmdr));
}

/// The economy asks for a solar every plan pass.
struct Solars;

impl Manager for Solars {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let (Some(c), Some(solar)) = (ctx.world.own.commander, ctx.view.def_named("ARMSOLAR"))
        else {
            return;
        };
        if ctx.step == Step::Plan {
            let at = Pos::new(300 + (ctx.tick / 30 % 5) as i32 * 64, 120);
            out.queued(c, Command::Build { def: solar, at });
        }
    }
}

#[test]
fn the_reservation_gate_caps_types() {
    let (mut sim, mut ai) = game(7, Difficulty::Standard);
    let solar = sim.def_id("ARMSOLAR").unwrap();
    ai.set_manager(Source::Economy, Box::new(Solars));
    ai.set_reservations(Some(ReservationTable {
        limits: [(solar, 2)].into_iter().collect(),
    }));
    let log = play(&mut sim, &mut ai, 600);
    let builds = log
        .iter()
        .filter(|(_, o)| o.command.produces() == Some(solar))
        .count();
    assert_eq!(builds, 2);
    assert_eq!(ai.stats().dispatch.gated, 20 - 2);
}

/// A manager whose orders depend on contacts, claims and the random stream.
struct Busy;

impl Manager for Busy {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        if ctx.step != Step::Army {
            return;
        }
        let spots: Vec<Pos> = ctx.world.map.deposits.iter().map(|d| d.center).collect();
        let base = ctx.world.enemy_base();
        for &u in &ctx.world.own.combat.clone() {
            let to = match base {
                Some(b) if u.0 % 2 == 0 => b,
                _ => ctx.rng.pick(&spots).copied().unwrap_or_default(),
            };
            out.now(u, Command::AttackMove(to));
        }
    }
}

fn busy_game(seed: u64) -> (Vec<(Tick, Order)>, u64) {
    let (mut sim, mut ai) = game(seed, Difficulty::Standard);
    spawn_raiders(&mut sim, 8);
    ai.set_manager(Source::Army, Box::new(Busy));
    let log = play(&mut sim, &mut ai, THREE_MINUTES);
    (log, sim.state_hash())
}

#[test]
fn two_runs_give_identical_order_logs() {
    let (a, ha) = busy_game(21);
    let (b, hb) = busy_game(21);
    assert!(!a.is_empty());
    assert_eq!(a, b);
    assert_eq!(ha, hb);
}

#[test]
fn contacts_are_forgotten_only_when_looked_at() {
    let (mut sim, mut ai) = game(8, Difficulty::Standard);
    // Let the passive script finish its base.
    play(&mut sim, &mut ai, 120 * TICKS_PER_SECOND);
    let enemy_cmdr = {
        let v = sim.view(P1);
        v.own_units()
            .into_iter()
            .find(|u| v.unit_def(u.def).unwrap().commander)
            .unwrap()
    };
    assert!(ai.world().contacts.get(enemy_cmdr.id).is_none());

    // A scout plane over the enemy base records it.
    let peep = sim.def_id("ARMPEEP").unwrap();
    let scout = sim.spawn(P0, peep, Pos::new(864, 650), true).unwrap();
    play(&mut sim, &mut ai, 30);
    let w = ai.world();
    let c = *w.contacts.get(enemy_cmdr.id).unwrap();
    assert!(c.commander && w.contacts.seen_now(c.id));
    let buildings: Vec<UnitId> = w
        .contacts
        .iter()
        .filter(|c| c.building)
        .map(|c| c.id)
        .collect();
    assert_eq!(buildings.len(), 4, "two extractors and two solars");
    assert_eq!(w.enemy.focus, Some(P1));
    assert!(w.enemy_base().is_some());

    let order = |unit, to| Order {
        unit,
        command: Command::Move(to),
        queue: Queue::Immediate,
    };
    // The scout leaves: nobody is looking, so everything is remembered.
    sim.submit(P0, [order(scout, Pos::new(1000, 100))]);
    play(&mut sim, &mut ai, 300);
    let w = ai.world();
    assert!(!w.contacts.seen_now(enemy_cmdr.id));
    let v = sim.view(P0);
    assert!(matches!(
        w.contacts.standing(&v, enemy_cmdr.id),
        Some(Standing::Remembered(_))
    ));
    assert!(w.contacts.target(&v, enemy_cmdr.id).is_none());
    assert!(buildings.iter().all(|&b| w.contacts.get(b).is_some()));

    // The commander walks off unseen: still remembered where it was.
    sim.submit(P1, [order(enemy_cmdr.id, Pos::new(400, 1000))]);
    play(&mut sim, &mut ai, 600);
    let c = *ai.world().contacts.get(enemy_cmdr.id).unwrap();
    assert!(c.pos.within(enemy_cmdr.pos, 64), "{:?}", c.pos);

    // The scout looks at the spot again: the commander is not there, so it
    // is forgotten; the buildings are there, so they are refreshed.
    sim.submit(P0, [order(scout, Pos::new(1000, 650))]);
    play(&mut sim, &mut ai, 150);
    let w = ai.world();
    assert!(w.contacts.get(enemy_cmdr.id).is_none());
    assert!(buildings.iter().all(|&b| w.contacts.seen_now(b)));
}
