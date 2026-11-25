//! The 15-tick sweep (SPEC §2.2, §5.5, §5.7): the exposed-commander verdict,
//! the commander's safety (danger, fight or flee, the d-gun) and builder
//! retreat.

use super::geom::{behind, centroid, dry, dry_path, inside, near_segment, on_bearing};
use super::{
    at_least, dist, health_pct, metal_of, secs, ArmyManager, BUILDING_REACH, COMMANDER_HURT,
    COVER_BEHIND, DGUN_CORRIDOR, DGUN_OVERREACH, FLEE_HOME_BEYOND, FLEE_STEP, LEASH_DEN, LEASH_NUM,
    MAP_MARGIN, RETREAT_MIN_STEP, SHIP_FALLBACK_GAIN, SHIP_MARGIN, SHIP_RINGS,
};
use crate::brain::contacts::{Contact, EnemyPicture};
use crate::brain::{Ctx, Priority, Staging, Standing};
use crate::view::{Amount, Command, GameView, Orders, Pos, Tick, UnitDef, UnitId};

/// What the sweep remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SafetyState {
    /// The commander's health at the last sweep.
    pub(crate) cmd_health: Option<i32>,
    /// The commander counts as in danger until this tick (§5.7).
    pub(crate) hurt_until: Tick,
    /// The latched hunt verdict: (enemy commander, allowed, until).
    pub(crate) hunt: Option<(UnitId, bool, Tick)>,
}

/// The exposure of a position (SPEC §5.7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Exposure {
    /// Metal of the recent armed non-air contacts whose reach covers it.
    pub threat: Amount,
    /// Metal of our armed units covering it.
    pub cover: Amount,
    /// The threats' centre, if any.
    pub centre: Option<Pos>,
    /// The threats' longest weapon range.
    pub max_range: i64,
}

impl Exposure {
    /// Cover < threat × `safety.cover_ratio`.
    pub fn exposed(&self, ratio: crate::knobs::Fixed) -> bool {
        self.threat > Amount::ZERO && !at_least(self.cover.milli(), ratio, self.threat.milli())
    }
}

fn is_ship(d: Option<&UnitDef>) -> bool {
    d.is_some_and(|d| d.mobile && !d.flies && !d.move_class.hovers && d.move_class.min_depth > 0)
}

/// The exposure of `p` (SPEC §5.7), counting only contacts seen within the
/// last `safety.contact_memory_ticks`.
pub fn exposure(ctx: &Ctx<'_>, p: Pos) -> Exposure {
    let k = &ctx.knobs.safety;
    let memory = Tick::try_from(k.contact_memory_ticks.max(0)).unwrap_or(0);
    let margin = dist(k.margin);
    let mut e = Exposure::default();
    let mut threats: Vec<Pos> = Vec::new();
    for c in ctx.world.contacts.iter() {
        if !c.armed || c.air || c.frame || ctx.tick.saturating_sub(c.last_seen) > memory {
            continue;
        }
        let range = i64::from(ctx.def(c.def).map_or(0, UnitDef::max_range));
        let reach = range + if c.building { BUILDING_REACH } else { margin };
        if c.pos.within(p, reach) {
            e.threat += c.metal;
            e.max_range = e.max_range.max(range);
            threats.push(c.pos);
        }
    }
    e.centre = centroid(threats);
    let Some(centre) = e.centre else {
        return e;
    };
    let cover_radius = dist(k.cover_radius);
    let cmd_metal = Amount::units(i64::from(ctx.knobs.cmd.fight_max_metal));
    for s in &ctx.world.own.units {
        let Some(d) = ctx.def(s.unit.def) else {
            continue;
        };
        if !d.armed() || d.flies || s.is_frame() {
            continue;
        }
        let range = i64::from(d.max_range());
        let covers = if s.building {
            s.unit.pos.within(centre, range + BUILDING_REACH) || s.unit.pos.within(p, range)
        } else {
            s.unit.pos.within(p, cover_radius) && behind(p, centre, s.unit.pos) <= COVER_BEHIND
        };
        if covers {
            e.cover += if s.commander { cmd_metal } else { d.cost_metal };
        }
    }
    e
}

/// Whether `p` is exposed (SPEC §5.7).
pub fn is_exposed(ctx: &Ctx<'_>, p: Pos) -> bool {
    exposure(ctx, p).exposed(ctx.knobs.safety.cover_ratio)
}

impl ArmyManager {
    pub(crate) fn sweep(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        self.hunt_verdict(ctx);
        let before = out.set_priority(Priority::Urgent);
        self.commander_safety(ctx, out);
        self.builder_retreat(ctx, out);
        out.set_priority(before);
    }

    /// The exposed-commander verdict (§5.5), latched for
    /// `army.hunt_latch_s` and cleared when the position becomes unknown.
    fn hunt_verdict(&mut self, ctx: &mut Ctx<'_>) {
        let k = &ctx.knobs.army;
        let w = &ctx.world;
        let cmdr = match w.enemy.focus {
            Some(f) => EnemyPicture::commander_of(&w.contacts, f),
            None => w.contacts.iter().find(|c| c.commander),
        };
        let Some(c) = cmdr.copied() else {
            self.safe.hunt = None;
            ctx.world.army.hunt_target = None;
            return;
        };
        let held = self
            .safe
            .hunt
            .filter(|&(id, _, until)| id == c.id && ctx.tick < until);
        let allowed = match held {
            Some((_, v, _)) => v,
            None => {
                let v = k.hunt_commander && (!k.hunt_exposed_only || exposed_commander(ctx, &c));
                self.safe.hunt = Some((c.id, v, ctx.tick.saturating_add(secs(k.hunt_latch_s))));
                v
            }
        };
        ctx.world.army.hunt_target = allowed.then_some(c.id);
    }

    /// The commander's danger, and fight or flight (§5.7).
    fn commander_safety(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let Some((cid, cpos, health, def)) = ctx.world.own.commander.and_then(|c| {
            let s = ctx.world.own.get(c)?;
            Some((c, s.unit.pos, s.unit.health, s.unit.def))
        }) else {
            self.safe.cmd_health = None;
            ctx.world.coord.commander_fleeing = false;
            ctx.world.army.commander_danger = false;
            ctx.world.army.commander_threats.clear();
            return;
        };
        let k = &ctx.knobs.cmd;
        if self.safe.cmd_health.is_some_and(|h| health < h) {
            self.safe.hurt_until = ctx.tick.saturating_add(COMMANDER_HURT);
        }
        self.safe.cmd_health = Some(health);
        let radius = dist(k.danger_radius);
        let threats: Vec<Contact> = ctx
            .world
            .contacts
            .iter()
            .filter(|c| c.armed && !c.air && !c.frame && ctx.world.contacts.seen_now(c.id))
            .filter(|c| {
                let range = i64::from(ctx.def(c.def).map_or(0, UnitDef::max_range));
                let r = if dry(ctx.view, c.pos) {
                    radius
                } else {
                    radius.max(range)
                };
                c.pos.within(cpos, r)
            })
            .filter(|c| matches!(ctx.standing(c.id), Some(Standing::Live(_))))
            .copied()
            .collect();
        let danger = ctx.tick < self.safe.hurt_until || !threats.is_empty();
        ctx.world.army.commander_danger = danger;
        ctx.world.army.commander_threats = threats.iter().map(|c| c.id).collect();
        if !danger {
            ctx.world.coord.commander_fleeing = false;
            return;
        }
        let cdef = ctx.def(def);
        let threat_metal = threats.iter().fold(Amount::ZERO, |a, c| a + c.metal);
        let all_ships = !threats.is_empty() && threats.iter().all(|c| is_ship(ctx.def(c.def)));
        let fires_here =
            dry(ctx.view, cpos) || cdef.is_some_and(|d| d.weapons.iter().any(|w| w.underwater));
        let fight = k.stands_ground
            && fires_here
            && health_pct(health, cdef) >= i64::from(k.retreat_pct)
            && threat_metal <= Amount::units(i64::from(k.fight_max_metal))
            && !(k.shuns_ships && all_ships);
        ctx.world.coord.commander_fleeing = !fight;
        if fight {
            self.commander_fight(ctx, out, cid, cpos, &threats, threat_metal);
        } else {
            commander_flee(ctx, out, cid, cpos, &threats, all_ships);
        }
    }

    fn commander_fight(
        &mut self,
        ctx: &Ctx<'_>,
        out: &mut Staging,
        cid: UnitId,
        cpos: Pos,
        threats: &[Contact],
        threat_metal: Amount,
    ) {
        let k = &ctx.knobs.cmd;
        let Some(s) = ctx.world.own.get(cid) else {
            return;
        };
        let frame = s
            .unit
            .working_on
            .filter(|f| ctx.world.own.get(*f).is_some_and(|fs| fs.is_frame()));
        // 4. The chase leash.
        let leash = dist(k.danger_radius) * LEASH_NUM / LEASH_DEN;
        if let Some(Command::Attack(t)) = super::current_order(ctx, cid) {
            if ctx
                .world
                .contacts
                .get(t)
                .is_some_and(|c| !c.pos.within(cpos, leash))
            {
                out.now(cid, Command::Move(cpos));
                return;
            }
        }
        // 1. Keeping a frame.
        if let (true, Some(f)) = (k.keeps_frames, frame) {
            if self.keeps_frame(ctx, out, f, threat_metal) {
                return;
            }
        }
        // 2. The d-gun.
        if k.dgun {
            if let Some(t) = dgun_target(ctx, cid, cpos, threats) {
                out.now(cid, Command::DGun(t));
                if let Some(f) = frame {
                    out.queued(cid, Command::Repair(f));
                }
                return;
            }
        }
        // 3. The nearest land threat.
        let nearest = threats
            .iter()
            .filter(|c| dry(ctx.view, c.pos))
            .min_by_key(|c| (c.pos.dist2(cpos), c.id));
        if let Some(t) = nearest {
            if ctx.target(t.id).is_some() {
                let want = Command::Attack(t.id);
                if super::current_order(ctx, cid) != Some(want) {
                    out.now(cid, want);
                    if let Some(f) = frame {
                        out.queued(cid, Command::Repair(f));
                    }
                }
            }
        }
    }

    /// Rule 1 of the fight: stay on a frame that would be lost, or hand it
    /// over. True when the commander stays.
    fn keeps_frame(
        &self,
        ctx: &Ctx<'_>,
        out: &mut Staging,
        frame: UnitId,
        threat_metal: Amount,
    ) -> bool {
        let k = &ctx.knobs.cmd;
        let Some(fs) = ctx.world.own.get(frame) else {
            return false;
        };
        let Some(p) = fs.unit.frame else {
            return false;
        };
        // Remaining life: built fraction × energy cost, in ticks.
        let energy = ctx.def(fs.unit.def).map_or(0, |d| d.cost_energy.whole());
        let life = if p.total > 0 {
            p.done.saturating_mul(energy) / p.total
        } else {
            0
        };
        let cover_r = dist(k.frame_cover_radius);
        let cover = ctx
            .world
            .own
            .combat
            .iter()
            .chain(&ctx.world.own.anti_air)
            .filter_map(|&u| ctx.world.own.get(u))
            .filter(|s| s.unit.pos.within(fs.unit.pos, cover_r))
            .fold(Amount::ZERO, |a, s| a + metal_of(ctx.def(s.unit.def)));
        if life < i64::from(secs(k.frame_absence_s)) && cover >= threat_metal {
            return true;
        }
        // Hand the frame to the nearest suitable builder that is not exposed.
        let hand = dist(k.frame_handover_radius);
        let fdef = fs.unit.def;
        let helper = ctx
            .world
            .own
            .builders
            .iter()
            .filter_map(|&b| ctx.world.own.get(b))
            .filter(|b| !b.commander && b.unit.working_on != Some(frame))
            .filter(|b| ctx.def(b.unit.def).is_some_and(|d| d.can_build(fdef)))
            .filter(|b| b.unit.pos.within(fs.unit.pos, hand))
            .filter(|b| !is_exposed(ctx, b.unit.pos))
            .min_by_key(|b| (b.unit.pos.dist2(fs.unit.pos), b.id()));
        if let Some(b) = helper {
            out.now(b.id(), Command::Repair(frame));
        }
        false
    }

    /// Builder retreat (§5.7, `safety.builders`).
    fn builder_retreat(&mut self, ctx: &mut Ctx<'_>, out: &mut Staging) {
        let tick = ctx.tick;
        ctx.world.coord.sheltering.retain(|_, &mut t| tick < t);
        let k = &ctx.knobs.safety;
        if !k.builders {
            return;
        }
        let home = ctx.world.own.home;
        let mut shelters: Vec<(UnitId, Tick)> = Vec::new();
        for &b in &ctx.world.own.builders {
            let Some(s) = ctx.world.own.get(b) else {
                continue;
            };
            if s.commander || s.building || ctx.def(s.unit.def).is_none_or(|d| d.flies) {
                continue;
            }
            let frame = s
                .unit
                .working_on
                .and_then(|f| ctx.world.own.get(f))
                .and_then(|f| f.unit.frame.map(|p| (f.id(), p)));
            if let Some((_, p)) = frame {
                if p.total > 0
                    && p.done.saturating_mul(100) >= i64::from(k.finish_above_pct) * p.total
                {
                    continue;
                }
            }
            let e = exposure(ctx, s.unit.pos);
            let Some(centre) = e.centre else {
                continue;
            };
            if !e.exposed(k.cover_ratio) {
                continue;
            }
            // Already on its way out: leave it be.
            let moving = matches!(super::current_order(ctx, b), Some(Command::Move(_)));
            if ctx.world.coord.is_sheltering(b, tick) && moving {
                continue;
            }
            let here = s.unit.pos;
            let step = RETREAT_MIN_STEP.max(e.max_range + dist(k.margin) - here.dist(centre));
            let away = here.toward(centre, -step);
            let to = match home {
                Some(h) if heads_home(here, away, h) => {
                    if here.within(h, step) {
                        h
                    } else {
                        here.toward(h, step)
                    }
                }
                _ => away,
            };
            let to = inside(ctx.view, to, super::MAP_MARGIN);
            out.now(b, Command::Move(to));
            if let (true, Some((f, _))) = (k.resume, frame) {
                out.queued(b, Command::Repair(f));
            }
            shelters.push((b, tick.saturating_add(secs(k.shelter_s))));
        }
        for (b, until) in shelters {
            ctx.world.coord.sheltering.insert(b, until);
        }
    }
}

/// Home lies in the retreat direction.
fn heads_home(here: Pos, away: Pos, home: Pos) -> bool {
    let ax = i64::from(away.x) - i64::from(here.x);
    let az = i64::from(away.z) - i64::from(here.z);
    let hx = i64::from(home.x) - i64::from(here.x);
    let hz = i64::from(home.z) - i64::from(here.z);
    ax * hx + az * hz > 0
}

/// Whether the enemy commander `c` is exposed (§5.5).
fn exposed_commander(ctx: &Ctx<'_>, c: &Contact) -> bool {
    let k = &ctx.knobs.army;
    let base = centroid(
        ctx.world
            .contacts
            .iter()
            .filter(|b| b.building && b.owner == c.owner)
            .map(|b| b.pos),
    );
    let far = base.is_none_or(|b| c.pos.dist(b) >= dist(k.hunt_min_distance));
    let escort = ctx
        .world
        .contacts
        .near(c.pos, dist(k.hunt_escort_radius))
        .filter(|e| e.armed && !e.commander)
        .fold(Amount::ZERO, |a, e| a + e.metal);
    far && escort < Amount::units(i64::from(k.hunt_max_escort))
}

/// The d-gun target (§5.7): the most valuable (then nearest) seen, standing,
/// armed, non-air threat on dry ground within range, with a clear line.
fn dgun_target(ctx: &Ctx<'_>, cid: UnitId, cpos: Pos, threats: &[Contact]) -> Option<UnitId> {
    let s = ctx.world.own.get(cid)?;
    let w = ctx.def(s.unit.def)?.weapons.get(2)?;
    if ctx.world.economy.raw.energy.stock < w.energy_per_shot {
        return None;
    }
    let range = i64::from(w.range);
    let by_value = ctx.knobs.cmd.dgun_by_value;
    let mut cands: Vec<&Contact> = threats
        .iter()
        .filter(|c| c.pos.within(cpos, range) && dry(ctx.view, c.pos))
        .filter(|c| ctx.target(c.id).is_some())
        .collect();
    cands.sort_by_key(|c| {
        let value = if by_value { -c.metal.milli() } else { 0 };
        (value, c.pos.dist2(cpos), c.id)
    });
    cands
        .into_iter()
        .find(|c| {
            let end = cpos.toward(c.pos, range + DGUN_OVERREACH);
            !ctx.world
                .own
                .units
                .iter()
                .any(|o| o.id() != cid && near_segment(o.unit.pos, cpos, end, DGUN_CORRIDOR))
        })
        .map(|c| c.id)
}

/// Fleeing (§5.7).
fn commander_flee(
    ctx: &Ctx<'_>,
    out: &mut Staging,
    cid: UnitId,
    cpos: Pos,
    threats: &[Contact],
    all_ships: bool,
) {
    let home = ctx.world.own.home;
    let to = if all_ships {
        flee_ships(ctx, cpos, threats, home)
    } else {
        let nearest = threats.iter().min_by_key(|c| (c.pos.dist2(cpos), c.id));
        match (home, nearest) {
            (Some(h), _) if !cpos.within(h, FLEE_HOME_BEYOND) => Some(h),
            (h, Some(n)) => {
                let p = inside(ctx.view, cpos.toward(n.pos, -FLEE_STEP), MAP_MARGIN);
                if dry(ctx.view, p) {
                    Some(p)
                } else {
                    h
                }
            }
            (h, None) => h.filter(|h| !cpos.within(*h, FLEE_HOME_BEYOND)),
        }
    };
    if let Some(p) = to {
        super::order(ctx, out, cid, Command::Move(p));
    }
}

/// Room left at `p` outside every ship's reach + margin (negative: inside).
fn ship_room(ctx: &Ctx<'_>, p: Pos, ships: &[Contact]) -> i64 {
    ships
        .iter()
        .map(|s| {
            let reach = i64::from(ctx.def(s.def).map_or(0, UnitDef::max_range)) + SHIP_MARGIN;
            p.dist(s.pos) - reach
        })
        .min()
        .unwrap_or(i64::MAX)
}

/// The dry point out of every ship's reach (§5.7 "Ships").
fn flee_ships(ctx: &Ctx<'_>, cpos: Pos, ships: &[Contact], home: Option<Pos>) -> Option<Pos> {
    let view: &dyn GameView = ctx.view;
    let m = view.map();
    let ok = |p: Pos| m.contains(p) && dry(view, p) && dry_path(view, cpos, p);
    let mut anywhere: Option<(i64, Pos)> = None;
    for r in SHIP_RINGS {
        let mut best: Option<(i64, Pos)> = None;
        for k in 0..16 {
            let p = on_bearing(cpos, k, r);
            if !ok(p) {
                continue;
            }
            let room = ship_room(ctx, p, ships);
            if anywhere.is_none_or(|(b, _)| room > b) {
                anywhere = Some((room, p));
            }
            if room > 0 && best.is_none_or(|(b, _)| room > b) {
                best = Some((room, p));
            }
        }
        if let Some((_, p)) = best {
            return Some(p);
        }
    }
    let here = ship_room(ctx, cpos, ships);
    if let Some((room, p)) = anywhere {
        if room > here.saturating_add(SHIP_FALLBACK_GAIN) {
            return Some(p);
        }
    }
    match home {
        Some(h) if ship_room(ctx, h, ships) > 0 => Some(h),
        _ => Some(cpos),
    }
}
