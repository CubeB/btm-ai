//! Transports and ferries on water maps (SPEC §5.9).
//!
//! Runs at [`crate::brain::Step::Transports`] when `ctx.due.tactics`
//! (SPEC §3.1 step 9, before the army). The transport slot is not
//! rate-limited (§2.3): load, unload and stop orders pass untouched, and so
//! do the moves of a muster.
//!
//! - **Ferry wanted.** The enemy is across water when the army's objective
//!   is not ground-reachable; with `ferry.want_from_map` that answer is
//!   latched, and before any objective exists "no land route to another
//!   start" counts. Published as `world.coord.army_needs_ferry`.
//! - **Muster** (`ferry.muster`): a dry, reachable shore point near the
//!   shipyard (else home) with naval-reachable water within
//!   [`MUSTER_WATER`]. While the army needs a sea ferry and we own no air
//!   transport it is the army's rally point (`world.coord.ferry_rally`).
//! - **Builder ferry** (islands, first): an idle transport takes one idle
//!   non-commander ground builder to the island extractor site
//!   (`world.eco.unreachable_expansion`), with no landing search.
//! - **Army ferry:** only in Assault, with the enemy across water now and a
//!   home point. Each free transport books passengers
//!   (`LoanReason::FerryPassenger`, so other passes leave them alone): air
//!   transports carry [`AIR_CAPACITY`], sea transports their capacity, and a
//!   sea transport waits for `ferry.min_party` combat units unless it is
//!   smaller. The landing search runs rings of [`LANDING_STEP`] from
//!   [`LANDING_FIRST`] out to `ferry.landing_rings` rings round the
//!   objective on the homeward fan, scored by anti-air cover, ground threat
//!   and distance. A failed search refuses the objective's
//!   `ferry.refusal_cell` cell for `ferry.refusal_s`
//!   (`world.coord.ferry_refusals`, read by the army's objective).
//! - **Trips:** a mustered sea ferry loads once the hull is within
//!   [`LOAD_REACH`] of the water point (first load Immediate, the rest
//!   Queued, then one queued unload); others load where the passengers
//!   stand. While still carrying with an empty queue, one more unload is
//!   issued (never two identical unloads queued). A trip ends when the
//!   transport was loaded and is empty; it is called off (Stop) with no
//!   living passengers or no pickup within `ferry.pickup_timeout_s`. A
//!   loaded sea hull that moves less than [`STALL_MOVE`] with an unchanged
//!   load for `ferry.stall_s` refuses its landing cell and searches again
//!   from where it is. Passengers go back to their lender when a trip ends.
//!
//! Reads `world.own`, `world.reach`, `world.map`, `world.army` (phase and
//! objective), `world.eco`, `world.grid`. Writes `world.coord.ferry_rally`,
//! `world.coord.ferry_refusals`, `world.coord.army_needs_ferry` and loans
//! in `world.ownership`. No randomness: ties go to the lower id or the
//! search order.

use std::collections::{BTreeMap, BTreeSet};

use super::geom::{on_bearing, pos, BEARINGS16};
use super::{dist, order, secs, RALLY_SLACK};
use crate::brain::roles::Role;
use crate::brain::{Coordination, Ctx, LoanReason, Manager, Phase, Source, Staging};
use crate::view::{Command, MoveClass, Orders, Pos, Tick, UnitDef, UnitId};

/// §5.9: air transports carry one unit per trip, whatever their data says.
pub const AIR_CAPACITY: usize = 1;
/// §5.9: the landing search's first ring and ring step.
pub const LANDING_FIRST: i64 = 192;
pub const LANDING_STEP: i64 = 48;
/// §5.9: a sea landing needs naval-reachable water within this, towards
/// home.
pub const LANDING_WATER: i64 = 150;
/// §5.9: the fan of 9 bearings within ±90° of homeward, nearest home first,
/// in steps of 22.5°.
pub const FAN: [i64; 9] = [0, 1, -1, 2, -2, 3, -3, 4, -4];
/// §5.9: the muster search's ring step and reach, and its water radius.
pub const MUSTER_STEP: i64 = 48;
pub const MUSTER_REACH: i64 = 1440;
pub const MUSTER_WATER: i64 = 144;
/// §5.9: a mustered hull loads once this close to its water point.
pub const LOAD_REACH: i64 = 64;
/// §5.9: a loaded hull moving less than this is making no progress.
pub const STALL_MOVE: i64 = 24;
/// Points sampled per tile when looking for water along a line.
const WATER_SAMPLE: i64 = 16;

/// What a trip carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Builder,
    Army,
}

/// A shore point with its water point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Muster {
    origin: Pos,
    shore: Pos,
    water: Pos,
}

/// One transport's trip.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Trip {
    kind: Kind,
    passengers: Vec<UnitId>,
    objective: Pos,
    landing: Pos,
    muster: Option<Muster>,
    sea: bool,
    loads_issued: bool,
    started: Tick,
    loaded: bool,
    last_pos: Pos,
    last_carry: usize,
    still_since: Tick,
}

/// The transport manager.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransportManager {
    /// The enemy was seen across water once (latched).
    latched: bool,
    /// Trips by transport id.
    trips: BTreeMap<UnitId, Trip>,
    /// The muster point, cached for its origin.
    muster: Option<Muster>,
}

impl Manager for TransportManager {
    fn update(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        if ctx.step != crate::brain::Step::Transports {
            return;
        }
        let tick = ctx.tick;
        ctx.world
            .coord
            .ferry_refusals
            .retain(|_, &mut until| tick < until);

        // Trips whose transport is gone release their passengers.
        let alive: BTreeSet<UnitId> = ctx.world.own.transports.iter().copied().collect();
        let gone: Vec<UnitId> = self
            .trips
            .keys()
            .copied()
            .filter(|t| !alive.contains(t))
            .collect();
        for t in gone {
            if let Some(trip) = self.trips.remove(&t) {
                release(ctx, &trip);
            }
        }

        // When a ferry is wanted.
        let k = &ctx.knobs.ferry;
        let objective = ctx.world.army.objective;
        let across = objective.is_some_and(|o| !ctx.world.reach.ground_reachable(o));
        self.latched |= across;
        let needs = if k.want_from_map {
            self.latched || (objective.is_none() && !ctx.world.reach.land_route_to_enemy)
        } else {
            across
        };
        ctx.world.coord.army_needs_ferry = needs;

        // The sea muster and the rally point.
        let yard = first_yard(ctx);
        let own_air = ctx.world.own.transports.iter().any(|&t| flies(ctx, t));
        let own_sea = ctx.world.own.transports.iter().any(|&t| !flies(ctx, t));
        let sea_ferry = needs
            && ctx.world.roles.get(Role::SeaTransport).is_some()
            && (yard.is_some() || own_sea);
        let muster = match yard.or(ctx.home()) {
            Some(origin) if k.muster && sea_ferry => self.muster_point(ctx, origin),
            _ => None,
        };
        ctx.world.coord.ferry_rally = muster.filter(|_| !own_air).map(|m| m.shore);

        // Trip upkeep.
        let ids: Vec<UnitId> = self.trips.keys().copied().collect();
        for t in ids {
            if let Some(mut trip) = self.trips.remove(&t) {
                if self.upkeep(ctx, out, t, &mut trip) {
                    self.trips.insert(t, trip);
                } else {
                    release(ctx, &trip);
                }
            }
        }

        // New trips: the builder ferry first, then the army.
        self.builder_ferry(ctx, out);
        if let (true, Some(o), Some(home)) = (
            ctx.world.army.phase == Phase::Assault && across,
            objective,
            ctx.home(),
        ) {
            self.army_ferry(ctx, out, o, home, muster);
        }
    }
}

fn flies(ctx: &Ctx<'_>, u: UnitId) -> bool {
    ctx.world
        .own
        .get(u)
        .and_then(|s| ctx.def(s.unit.def))
        .is_some_and(|d| d.flies)
}

fn depth(ctx: &Ctx<'_>, p: Pos) -> i32 {
    ctx.view.water_depth(p.tile())
}

/// Our first shipyard (id order).
fn first_yard(ctx: &Ctx<'_>) -> Option<Pos> {
    let w = &ctx.world;
    w.own
        .factories
        .iter()
        .filter_map(|&f| w.own.get(f))
        .find(|s| {
            w.roles.is(s.unit.def, Role::Shipyard) || w.roles.is(s.unit.def, Role::AdvancedShipyard)
        })
        .map(|s| s.unit.pos)
}

/// The ground layer's movement class (the constructor's, else the
/// commander's).
fn ground_class(ctx: &Ctx<'_>) -> Option<MoveClass> {
    let w = &ctx.world;
    w.reach
        .ground
        .as_ref()
        .map(|l| l.mover)
        .or_else(|| {
            w.own
                .commander
                .and_then(|c| w.own.get(c))
                .map(|s| s.unit.def)
        })
        .and_then(|d| ctx.def(d))
        .map(|d| d.move_class)
}

/// Dry and ground-walkable.
fn walkable(ctx: &Ctx<'_>, class: Option<MoveClass>, p: Pos) -> bool {
    ctx.view.map().contains(p)
        && depth(ctx, p) == 0
        && class.is_none_or(|c| ctx.view.standable(p.tile(), c))
}

/// Naval-reachable water.
fn sea(ctx: &Ctx<'_>, p: Pos) -> bool {
    ctx.view.map().contains(p) && depth(ctx, p) > 0 && ctx.world.reach.naval_reachable(p)
}

/// Gives every passenger still lent to the transports back.
fn release(ctx: &mut Ctx<'_>, trip: &Trip) {
    for &p in &trip.passengers {
        if ctx
            .world
            .ownership
            .loan(p)
            .is_some_and(|l| l.to == Source::Transport)
        {
            // An error means the loan already ended; nothing to undo.
            let _ = ctx.world.ownership.give_back(p, Source::Transport);
        }
    }
}

/// Refuses the cell of `p` for `ferry.refusal_s`.
fn refuse(ctx: &mut Ctx<'_>, p: Pos) {
    let k = &ctx.knobs.ferry;
    let cell = Coordination::refusal_cell(p, k.refusal_cell);
    let until = ctx.tick.saturating_add(secs(k.refusal_s));
    ctx.world.coord.ferry_refusals.insert(cell, until);
}

/// The point `r` from `o` on the homeward bearing rotated by `step` ×
/// 22.5°, with `(dx, dz)` the homeward vector of length `len`.
fn rotated(o: Pos, dx: i64, dz: i64, len: i64, step: i64, r: i64) -> Pos {
    let k = usize::try_from(step.rem_euclid(16)).unwrap_or(0);
    let (c, s) = BEARINGS16.get(k).copied().unwrap_or((1000, 0));
    let den = len.max(1) * 1000;
    pos(
        i64::from(o.x) + r * (dx * c - dz * s) / den,
        i64::from(o.z) + r * (dx * s + dz * c) / den,
    )
}

/// The landing search (§5.9). `None` when nothing qualifies.
fn landing(ctx: &Ctx<'_>, objective: Pos, origin: Pos, air: bool, sea_lift: bool) -> Option<Pos> {
    let k = &ctx.knobs.ferry;
    let class = ground_class(ctx);
    let w = &ctx.world;
    let qualifies = |p: Pos| {
        walkable(ctx, class, p)
            && !w.reach.ground_reachable(p)
            && !w.coord.landing_refused(p, k.refusal_cell, ctx.tick)
            && (!sea_lift || water_towards(ctx, p, origin))
    };
    let dx = i64::from(origin.x) - i64::from(objective.x);
    let dz = i64::from(origin.z) - i64::from(objective.z);
    let len = objective.dist(origin);
    let fan = k.landing_fan && len > 0;
    let rings = i64::from(k.landing_rings.max(0));
    let radius = i32::try_from(dist(k.landing_threat_radius)).unwrap_or(i32::MAX);
    // (aa cover, ground threat, distance², search index)
    let mut best: Option<((i64, i64, i64, usize), Pos)> = None;
    let mut r = LANDING_FIRST;
    while r <= LANDING_STEP * rings {
        let points: Vec<Pos> = if fan {
            FAN.iter()
                .map(|&s| rotated(objective, dx, dz, len, s, r))
                .collect()
        } else {
            (0..16).map(|b| on_bearing(objective, b, r)).collect()
        };
        let mut ring: Option<((i64, i64, i64, usize), Pos)> = None;
        for (i, p) in points.into_iter().enumerate() {
            if !qualifies(p) {
                continue;
            }
            if !k.landing_avoid_threat {
                return Some(p);
            }
            let aa = if air {
                i64::from(w.grid.aa_at(p.vis_cell()))
            } else {
                0
            };
            let key = (aa, w.grid.threat_in_radius(p, radius), p.dist2(origin), i);
            if ring.is_none_or(|(b, _)| key < b) {
                ring = Some((key, p));
            }
        }
        if let Some((key, p)) = ring {
            if key.0 == 0 && key.1 == 0 {
                return Some(p);
            }
            if best.is_none_or(|(b, _)| (key.0, key.1, key.2) < (b.0, b.1, b.2)) {
                best = Some((key, p));
            }
        }
        r += LANDING_STEP;
    }
    best.map(|(_, p)| p)
        .or_else(|| qualifies(objective).then_some(objective))
}

/// Naval-reachable water within [`LANDING_WATER`] of `p` towards `origin`.
fn water_towards(ctx: &Ctx<'_>, p: Pos, origin: Pos) -> bool {
    let mut d = WATER_SAMPLE;
    loop {
        let d_now = d.min(LANDING_WATER);
        if sea(ctx, p.toward(origin, d_now)) {
            return true;
        }
        if d_now >= LANDING_WATER {
            return false;
        }
        d += WATER_SAMPLE;
    }
}

impl TransportManager {
    /// The sea muster for `origin`, searched once per origin (§5.9).
    fn muster_point(&mut self, ctx: &Ctx<'_>, origin: Pos) -> Option<Muster> {
        if let Some(m) = self.muster.filter(|m| m.origin == origin) {
            return Some(m);
        }
        let class = ground_class(ctx);
        let mut r = MUSTER_STEP;
        let mut found = None;
        'rings: while r <= MUSTER_REACH {
            for b in 0..16 {
                let shore = on_bearing(origin, b, r);
                if !walkable(ctx, class, shore) || !ctx.world.reach.ground_reachable(shore) {
                    continue;
                }
                let mut rr = MUSTER_STEP;
                while rr <= MUSTER_WATER {
                    if let Some(water) = (0..16)
                        .map(|b2| on_bearing(shore, b2, rr))
                        .find(|&q| sea(ctx, q))
                    {
                        found = Some(Muster {
                            origin,
                            shore,
                            water,
                        });
                        break 'rings;
                    }
                    rr += MUSTER_STEP;
                }
            }
            r += MUSTER_STEP;
        }
        self.muster = found;
        found
    }

    /// One trip's upkeep; false when the trip is over.
    fn upkeep(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging, t: UnitId, trip: &mut Trip) -> bool {
        let Some((here, carry)) = ctx
            .world
            .own
            .get(t)
            .map(|s| (s.unit.pos, s.unit.carrying.len()))
        else {
            return false;
        };
        if carry > 0 {
            trip.loaded = true;
        }
        if trip.loaded && carry == 0 {
            return false;
        }
        let living = trip
            .passengers
            .iter()
            .any(|&p| ctx.world.own.get(p).is_some());
        let k = &ctx.knobs.ferry;
        let timed_out =
            !trip.loaded && ctx.tick.saturating_sub(trip.started) >= secs(k.pickup_timeout_s);
        if (!living && carry == 0) || timed_out {
            out.now(t, Command::Stop);
            return false;
        }
        if !trip.loads_issued {
            match trip.muster {
                Some(m) => {
                    for &p in &trip.passengers {
                        let near = ctx.world.own.get(p).is_none_or(|s| {
                            s.unit.carried_by.is_some() || s.unit.pos.within(m.shore, RALLY_SLACK)
                        });
                        if !near {
                            order(ctx, out, p, Command::Move(m.shore));
                        }
                    }
                    if here.within(m.water, LOAD_REACH) {
                        issue_loads(ctx, out, t, trip);
                    } else {
                        order(ctx, out, t, Command::Move(m.water));
                    }
                }
                None => issue_loads(ctx, out, t, trip),
            }
        } else if trip.loaded && carry > 0 && !ctx.busy(t) {
            out.now(t, Command::Unload(trip.landing));
        }

        // A loaded sea hull making no progress re-plans its landing.
        if trip.sea && trip.loaded && carry > 0 {
            if !here.within(trip.last_pos, STALL_MOVE - 1) || carry != trip.last_carry {
                trip.last_pos = here;
                trip.last_carry = carry;
                trip.still_since = ctx.tick;
            } else if ctx.tick.saturating_sub(trip.still_since) >= secs(k.stall_s) {
                refuse(ctx, trip.landing);
                if let Some(l) = landing(ctx, trip.objective, here, false, true) {
                    trip.landing = l;
                    out.now(t, Command::Unload(l));
                }
                trip.still_since = ctx.tick;
            }
        }
        true
    }

    /// The island builder ferry (§5.9): one at a time, before the army.
    fn builder_ferry(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let Some(site) = ctx.world.eco.unreachable_expansion else {
            return;
        };
        if self.trips.values().any(|t| t.kind == Kind::Builder) {
            return;
        }
        let Some((t, max_size, air)) = self.free_transports(ctx).first().copied() else {
            return;
        };
        let w = &ctx.world;
        let builder = w.own.idle_builders.iter().copied().find(|&b| {
            w.own.get(b).is_some_and(|s| {
                !s.commander
                    && w.reach.ground_reachable(s.unit.pos)
                    && ctx.def(s.unit.def).is_some_and(|d| fits(d, max_size))
            })
        });
        let Some(b) = builder else {
            return;
        };
        if ctx
            .world
            .ownership
            .lend(
                b,
                Source::Economy,
                Source::Transport,
                LoanReason::FerryPassenger,
            )
            .is_err()
        {
            return;
        }
        let mut trip = self.new_trip(ctx, t, Kind::Builder, vec![b], site, site, None, !air);
        issue_loads(ctx, out, t, &mut trip);
        self.trips.insert(t, trip);
    }

    /// Army ferries (§5.9) for every free transport.
    fn army_ferry(
        &mut self,
        ctx: &mut Ctx<'_>,
        out: &mut Staging,
        objective: Pos,
        home: Pos,
        muster: Option<Muster>,
    ) {
        let k = &ctx.knobs.ferry;
        let min_party = usize::try_from(k.min_party.max(0)).unwrap_or(0);
        let army = usize::try_from(ctx.world.own.army_size).unwrap_or(usize::MAX);
        let mut taken: BTreeSet<UnitId> = BTreeSet::new();
        for (t, max_size, air) in self.free_transports(ctx) {
            let Some(d) = ctx.world.own.get(t).and_then(|s| ctx.def(s.unit.def)) else {
                continue;
            };
            let capacity = if air {
                AIR_CAPACITY
            } else {
                usize::from(d.transport.map_or(0, |x| x.capacity))
            };
            if capacity == 0 || (!air && capacity >= min_party && army < min_party) {
                continue;
            }
            let w = &ctx.world;
            let passengers: Vec<UnitId> = w
                .own
                .combat
                .iter()
                .copied()
                .filter(|u| !taken.contains(u))
                .filter(|&u| {
                    w.own.get(u).is_some_and(|s| {
                        s.unit.carried_by.is_none()
                            && w.reach.ground_reachable(s.unit.pos)
                            && ctx.def(s.unit.def).is_some_and(|d| fits(d, max_size))
                    })
                })
                .take(capacity)
                .collect();
            if passengers.is_empty() {
                return;
            }
            let mustered = muster.filter(|_| !air);
            let origin = mustered.map_or(home, |m| m.shore);
            let Some(l) = landing(ctx, objective, origin, air, !air) else {
                refuse(ctx, objective);
                return;
            };
            let cell = Coordination::refusal_cell(objective, k.refusal_cell);
            ctx.world.coord.ferry_refusals.remove(&cell);
            let booked: Vec<UnitId> = passengers
                .into_iter()
                .filter(|&p| {
                    ctx.world
                        .ownership
                        .lend(
                            p,
                            Source::Army,
                            Source::Transport,
                            LoanReason::FerryPassenger,
                        )
                        .is_ok()
                })
                .collect();
            if booked.is_empty() {
                continue;
            }
            taken.extend(booked.iter().copied());
            let mut trip = self.new_trip(ctx, t, Kind::Army, booked, objective, l, mustered, !air);
            if mustered.is_none() {
                issue_loads(ctx, out, t, &mut trip);
            }
            self.trips.insert(t, trip);
        }
    }

    /// Transports with no trip, nothing aboard and no orders, in id order,
    /// with their size limit and whether they fly.
    fn free_transports(&self, ctx: &Ctx<'_>) -> Vec<(UnitId, u8, bool)> {
        ctx.world
            .own
            .transports
            .iter()
            .copied()
            .filter(|t| !self.trips.contains_key(t) && !ctx.busy(*t))
            .filter_map(|t| {
                let s = ctx.world.own.get(t)?;
                let d = ctx.def(s.unit.def)?;
                let cap = d.transport?;
                s.unit
                    .carrying
                    .is_empty()
                    .then_some((t, cap.max_size, d.flies))
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn new_trip(
        &self,
        ctx: &Ctx<'_>,
        t: UnitId,
        kind: Kind,
        passengers: Vec<UnitId>,
        objective: Pos,
        landing: Pos,
        muster: Option<Muster>,
        sea: bool,
    ) -> Trip {
        let here = ctx.world.own.get(t).map(|s| s.unit.pos).unwrap_or_default();
        Trip {
            kind,
            passengers,
            objective,
            landing,
            muster,
            sea,
            loads_issued: false,
            started: ctx.tick,
            loaded: false,
            last_pos: here,
            last_carry: 0,
            still_since: ctx.tick,
        }
    }
}

/// A passenger the transport can lift: ground-going (no hovercraft, no
/// ship), transportable and within the size limit.
fn fits(d: &UnitDef, max_size: u8) -> bool {
    d.mobile
        && !d.flies
        && !d.move_class.hovers
        && d.move_class.min_depth == 0
        && d.can_be_transported
        && d.size <= max_size
}

/// The loads (first Immediate, the rest Queued) and one queued unload.
fn issue_loads(ctx: &Ctx<'_>, out: &mut Staging, t: UnitId, trip: &mut Trip) {
    let mut first = true;
    for &p in &trip.passengers {
        if ctx.world.own.get(p).is_none() {
            continue;
        }
        if first {
            out.now(t, Command::Load(p));
            first = false;
        } else {
            out.queued(t, Command::Load(p));
        }
    }
    if !first {
        out.queued(t, Command::Unload(trip.landing));
    }
    trip.loads_issued = true;
}
