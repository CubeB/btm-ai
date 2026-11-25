//! Per-unit behaviour: commands, factories, building, combat and death.

use super::{
    Event, FeatureState, Reject, Rejection, Sim, Unit, BUILD_REACH, POS_SCALE, SUB,
    TRANSPORT_DELAY, WRECK_METAL_PCT,
};
use crate::view::{
    footprint_origin, Command, DefId, FeatureKind, MoveClass, Order, PlayerId, Pos, Queue, UnitDef,
    UnitId, TICKS_PER_SECOND, TILE_SIZE,
};

/// The parts of a definition the per-tick code needs, copied out so that
/// the sim can be mutated while they are in use.
#[derive(Clone, Copy, Debug)]
struct Caps {
    mobile: bool,
    flies: bool,
    factory: bool,
    armed: bool,
    worker: i64,
    speed: i64,
    mc: MoveClass,
    range: i64,
    capacity: usize,
    max_size: u8,
}

impl Caps {
    fn of(d: &UnitDef) -> Self {
        Caps {
            mobile: d.mobile,
            flies: d.flies,
            factory: d.is_factory(),
            armed: d.armed(),
            worker: d.worker_rate,
            speed: i64::from(d.speed),
            mc: d.move_class,
            range: i64::from(d.weapons.first().map_or(0, |w| w.range)),
            capacity: usize::from(d.transport.map_or(0, |t| t.capacity)),
            max_size: d.transport.map_or(0, |t| t.max_size),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MoveResult {
    Moving,
    Arrived,
    Stuck,
}

/// Flat distance from `p` to a footprint of `fp` tiles centred on `c`.
fn dist_to_rect(p: Pos, c: Pos, fp: (i32, i32)) -> i64 {
    let dx = (i64::from(p.x) - i64::from(c.x)).abs() - i64::from(fp.0 * TILE_SIZE / 2);
    let dz = (i64::from(p.z) - i64::from(c.z)).abs() - i64::from(fp.1 * TILE_SIZE / 2);
    let (dx, dz) = (dx.max(0), dz.max(0));
    (dx * dx + dz * dz).unsigned_abs().isqrt() as i64
}

impl Sim {
    /// Distance from a unit to another unit's body (footprint for buildings).
    fn reach_dist(&self, from: Pos, to: &Unit) -> i64 {
        let d = self.def(to.def);
        if d.mobile {
            (from.dist(to.pos()) - 16).max(0)
        } else {
            dist_to_rect(from, to.pos(), d.footprint)
        }
    }

    fn reject(&mut self, u: &Unit, command: Command, reason: Reject) {
        self.rejected.push(Rejection {
            tick: self.tick,
            player: u.owner,
            order: Order {
                unit: u.id,
                command,
                queue: Queue::Queued,
            },
            reason,
        });
    }

    pub(super) fn update_unit(&mut self, id: UnitId) {
        let Some(mut u) = self.units.remove(&id) else {
            return;
        };
        if u.frame.is_none() && u.carried_by.is_none() && u.hp > 0 {
            let c = Caps::of(self.def(u.def));
            if c.factory {
                self.factory_tick(&mut u, &c);
            } else if c.mobile {
                self.command_tick(&mut u, &c);
            }
            for cid in &u.carrying {
                if let Some(cu) = self.units.get_mut(cid) {
                    cu.x = u.x;
                    cu.z = u.z;
                }
            }
        }
        self.units.insert(id, u);
    }

    fn factory_tick(&mut self, u: &mut Unit, c: &Caps) {
        if let Some(f) = u.building {
            if self.units.get(&f).is_some_and(|fu| fu.frame.is_some()) {
                if self.advance_frame(f, u.owner, c.worker) {
                    u.fqueue.pop_front();
                    u.building = None;
                    let fp = self.def(u.def).footprint;
                    let p = u.pos();
                    let exit = Pos::new(p.x, p.z + fp.1 * TILE_SIZE / 2 + 32);
                    if let Some(nu) = self.units.get_mut(&f) {
                        nu.cmds.push_back(Command::Move(exit));
                    }
                }
                return;
            }
            u.building = None;
        }
        if let Some(&head) = u.fqueue.front() {
            if let Ok(fid) = self.spawn(u.owner, head, u.pos(), false) {
                u.building = Some(fid);
            }
        }
    }

    /// Adds up to `rate` work-ticks to frame `fid`, paid by `payer`, scaled
    /// down if stock runs short. Returns true if the frame completed.
    fn advance_frame(&mut self, fid: UnitId, payer: PlayerId, rate: i64) -> bool {
        let Some(f) = self.units.get(&fid) else {
            return false;
        };
        let Some(fr) = f.frame else {
            return false;
        };
        let d = self.def(f.def);
        let (cost_m, cost_e) = (d.cost_metal.0 * SUB, d.cost_energy.0 * SUB);
        let max_hp = i64::from(d.health) * 1000;
        let need = |s: i64| {
            (
                cost_m * (fr.done + s) / fr.total - fr.paid_m,
                cost_e * (fr.done + s) / fr.total - fr.paid_e,
            )
        };
        let full = rate.min(fr.total - fr.done);
        if full <= 0 {
            return false;
        }
        let Some(pl) = self.players.get_mut(usize::from(payer.0)) else {
            return false;
        };
        let (nm, ne) = need(full);
        pl.metal.demand_acc += nm;
        pl.energy.demand_acc += ne;
        let mut step = full;
        if nm > pl.metal.stock {
            pl.metal.stall_acc = true;
            step = step.min(full * pl.metal.stock / nm.max(1));
        }
        if ne > pl.energy.stock {
            pl.energy.stall_acc = true;
            step = step.min(full * pl.energy.stock / ne.max(1));
        }
        let mut tries = 0;
        while step > 0 && tries < 64 {
            let (m, e) = need(step);
            if m <= pl.metal.stock && e <= pl.energy.stock {
                break;
            }
            step -= 1;
            tries += 1;
        }
        let (m, e) = need(step);
        if step <= 0 || m > pl.metal.stock || e > pl.energy.stock {
            return false;
        }
        pl.metal.stock -= m;
        pl.energy.stock -= e;
        let Some(f) = self.units.get_mut(&fid) else {
            return false;
        };
        let Some(fr) = f.frame.as_mut() else {
            return false;
        };
        fr.done += step;
        fr.paid_m += m;
        fr.paid_e += e;
        let hp_target = max_hp * fr.done / fr.total;
        f.hp = (f.hp + hp_target - fr.hp_given).min(max_hp);
        fr.hp_given = hp_target;
        if fr.done < fr.total {
            return false;
        }
        f.frame = None;
        let (owner, def) = (f.owner, f.def);
        if let Some(fu) = self.units.get(&fid) {
            let ex = self.extraction(fu);
            if let Some(fu) = self.units.get_mut(&fid) {
                fu.extract = ex;
            }
        }
        self.events.push(Event::Completed {
            tick: self.tick,
            unit: fid,
            owner,
            def,
        });
        true
    }

    /// Moves a unit one tick towards `goal`.
    fn move_unit(&self, u: &mut Unit, c: &Caps, goal: Pos, arrive: i64) -> MoveResult {
        if u.pos().within(goal, arrive) {
            u.path.clear();
            return MoveResult::Arrived;
        }
        if c.speed <= 0 {
            return MoveResult::Stuck;
        }
        if !c.flies {
            let fresh = match u.path_goal {
                Some(g) => !g.within(goal, 32),
                None => true,
            };
            if fresh {
                let mut p = self.map.path(u.pos(), goal, c.mc);
                p.reverse();
                u.path = p;
                u.path_goal = Some(goal);
            }
            if u.path.is_empty() {
                return MoveResult::Stuck;
            }
        }
        let mut budget = (c.speed * POS_SCALE / i64::from(TICKS_PER_SECOND)).max(1);
        while budget > 0 {
            let w = if c.flies {
                goal
            } else {
                match u.path.last() {
                    Some(w) => *w,
                    None => break,
                }
            };
            let wx = i64::from(w.x) * POS_SCALE + POS_SCALE / 2;
            let wz = i64::from(w.z) * POS_SCALE + POS_SCALE / 2;
            let (dx, dz) = (wx - u.x, wz - u.z);
            let dist = (dx * dx + dz * dz).unsigned_abs().isqrt() as i64;
            if dist <= budget {
                u.x = wx;
                u.z = wz;
                budget -= dist;
                if c.flies {
                    break;
                }
                u.path.pop();
            } else {
                u.x += dx * budget / dist;
                u.z += dz * budget / dist;
                budget = 0;
            }
        }
        if u.pos().within(goal, arrive) {
            MoveResult::Arrived
        } else {
            MoveResult::Moving
        }
    }

    /// Moves towards a work site; drops the command if it cannot get there.
    fn approach(&mut self, u: &mut Unit, c: &Caps, goal: Pos, cmd: Command) {
        if self.move_unit(u, c, goal, 0) != MoveResult::Moving {
            self.reject(u, cmd, Reject::BadSite);
            u.pop();
        }
    }

    fn command_tick(&mut self, u: &mut Unit, c: &Caps) {
        let Some(&cmd) = u.cmds.front() else {
            return;
        };
        match cmd {
            Command::Move(p) => {
                if self.move_unit(u, c, p, 8) != MoveResult::Moving {
                    u.pop();
                }
            }
            Command::AttackMove(p) | Command::Patrol(p) => {
                if c.armed && !self.targets_in_range(u, i64::MAX).is_empty() {
                    return;
                }
                if self.move_unit(u, c, p, 16) != MoveResult::Moving {
                    u.pop();
                    if matches!(cmd, Command::Patrol(_)) {
                        u.cmds.push_back(cmd);
                    }
                }
            }
            Command::Attack(t) => match self.hostile_seen(u, t) {
                Some(tp) if u.pos().within(tp, c.range) => u.path.clear(),
                Some(tp) => {
                    if self.move_unit(u, c, tp, (c.range - 16).max(0)) == MoveResult::Stuck {
                        u.pop();
                    }
                }
                None => u.pop(),
            },
            Command::DGun(t) => self.dgun_tick(u, c, t),
            Command::Guard(t) => self.guard_tick(u, c, t),
            Command::Repair(t) => self.repair_tick(u, c, t, cmd),
            Command::ReclaimFeature(f) => self.reclaim_feature_tick(u, c, f, cmd),
            Command::ReclaimUnit(t) | Command::Capture(t) => self.take_unit_tick(u, c, t, cmd),
            Command::Build { def, at } => self.build_tick(u, c, def, at, cmd),
            Command::Load(t) => self.load_tick(u, c, t),
            Command::Unload(p) => self.unload_tick(u, c, p),
            Command::Stop | Command::SetOn(_) | Command::FactoryQueue { .. } => u.pop(),
        }
    }

    /// The position of `t` if it is a living non-allied unit `u` sees now.
    fn hostile_seen(&self, u: &Unit, t: UnitId) -> Option<Pos> {
        let tu = self.units.get(&t)?;
        (tu.hp > 0 && !self.allied(u.owner, tu.owner) && self.sees_unit(u.owner, tu))
            .then(|| tu.pos())
    }

    fn dgun_tick(&mut self, u: &mut Unit, c: &Caps, t: UnitId) {
        let Some(tp) = self.hostile_seen(u, t) else {
            u.pop();
            return;
        };
        let Some(w) = self.def(u.def).weapons.get(2) else {
            u.pop();
            return;
        };
        let (range, dmg, reload, cost) =
            (i64::from(w.range), w.damage, w.reload, w.energy_per_shot.0);
        if !u.pos().within(tp, range) {
            if self.move_unit(u, c, tp, (range - 16).max(0)) == MoveResult::Stuck {
                u.pop();
            }
            return;
        }
        u.path.clear();
        if u.reload.get(2).copied().unwrap_or(0) > 0 {
            return;
        }
        let Some(pl) = self.players.get_mut(usize::from(u.owner.0)) else {
            return;
        };
        let cost = cost * SUB;
        if pl.energy.stock >= cost {
            pl.energy.stock -= cost;
            if let Some(r) = u.reload.get_mut(2) {
                *r = reload;
            }
            if let Some(tu) = self.units.get_mut(&t) {
                tu.hp -= i64::from(dmg) * 1000;
                tu.last_attacker = Some(u.id);
            }
        }
        u.pop();
    }

    fn guard_tick(&mut self, u: &mut Unit, c: &Caps, t: UnitId) {
        let Some(tu) = self.units.get(&t) else {
            u.pop();
            return;
        };
        let tp = tu.pos();
        let frame = tu
            .building
            .filter(|f| self.units.get(f).is_some_and(|fu| fu.frame.is_some()));
        let near = self.reach_dist(u.pos(), tu) <= BUILD_REACH;
        match frame {
            Some(f) if near && self.allied(u.owner, tu.owner) => {
                u.path.clear();
                u.building = Some(f);
                self.advance_frame(f, u.owner, c.worker);
            }
            _ if near || u.pos().within(tp, 96) => u.path.clear(),
            _ => {
                self.move_unit(u, c, tp, 64);
            }
        }
    }

    fn repair_tick(&mut self, u: &mut Unit, c: &Caps, t: UnitId, cmd: Command) {
        let Some(tu) = self.units.get(&t) else {
            u.pop();
            return;
        };
        if !self.allied(u.owner, tu.owner) {
            u.pop();
            return;
        }
        if self.reach_dist(u.pos(), tu) > BUILD_REACH {
            let tp = tu.pos();
            self.approach(u, c, tp, cmd);
            return;
        }
        u.path.clear();
        if tu.frame.is_some() {
            u.building = Some(t);
            self.advance_frame(t, u.owner, c.worker);
            return;
        }
        let d = self.def(tu.def);
        let max = i64::from(d.health) * 1000;
        let heal = (max * c.worker / (d.build_time * SUB).max(1)).max(1);
        match self.units.get_mut(&t) {
            Some(tu) if tu.hp < max => tu.hp = (tu.hp + heal).min(max),
            _ => u.pop(),
        }
    }

    fn reclaim_feature_tick(
        &mut self,
        u: &mut Unit,
        c: &Caps,
        fid: crate::view::FeatureId,
        cmd: Command,
    ) {
        let Some(f) = self.features.get(&fid).copied() else {
            u.pop();
            return;
        };
        if !f.reclaimable {
            u.pop();
            return;
        }
        if dist_to_rect(u.pos(), f.pos, f.footprint) > BUILD_REACH {
            self.approach(u, c, f.pos, cmd);
            return;
        }
        u.path.clear();
        let rate = c.worker * 100;
        let (m, e) = (f.metal.min(rate), f.energy.min(rate));
        if let Some(pl) = self.players.get_mut(usize::from(u.owner.0)) {
            pl.metal.add(m);
            pl.energy.add(e);
        }
        let left = self.features.get_mut(&fid).map(|f| {
            f.metal -= m;
            f.energy -= e;
            f.metal + f.energy
        });
        if left.unwrap_or(0) <= 0 {
            self.features.remove(&fid);
            u.pop();
        }
    }

    /// Reclaim or capture a unit.
    fn take_unit_tick(&mut self, u: &mut Unit, c: &Caps, t: UnitId, cmd: Command) {
        let Some(tu) = self.units.get(&t) else {
            u.pop();
            return;
        };
        let capture = matches!(cmd, Command::Capture(_));
        if capture && self.allied(u.owner, tu.owner) {
            u.pop();
            return;
        }
        if self.reach_dist(u.pos(), tu) > BUILD_REACH {
            let tp = tu.pos();
            self.approach(u, c, tp, cmd);
            return;
        }
        u.path.clear();
        let d = self.def(tu.def);
        let total = (d.build_time * SUB).max(1);
        let (max, cost_m) = (i64::from(d.health) * 1000, d.cost_metal.0 * SUB);
        if capture {
            u.work += c.worker;
            if u.work >= total {
                if let Some(tu) = self.units.get_mut(&t) {
                    tu.owner = u.owner;
                    tu.cmds.clear();
                    tu.path.clear();
                }
                u.pop();
            }
            return;
        }
        let drain = (max * c.worker / total).max(1).min(tu.hp.max(0));
        let metal = cost_m * drain / max.max(1);
        if let Some(pl) = self.players.get_mut(usize::from(u.owner.0)) {
            pl.metal.add(metal);
        }
        let gone = self.units.get_mut(&t).is_none_or(|tu| {
            tu.hp -= drain;
            tu.hp <= 0
        });
        if gone {
            self.remove_unit(t, Some(u.id), false);
            u.pop();
        }
    }

    fn build_tick(&mut self, u: &mut Unit, c: &Caps, def: DefId, at: Pos, cmd: Command) {
        let center = self.footprint_center(def, at);
        let fp = self.def(def).footprint;
        let origin = footprint_origin(fp, center);
        let existing = self
            .units
            .values()
            .find(|o| o.def == def && footprint_origin(fp, o.pos()) == origin)
            .map(|o| (o.id, o.frame.is_some()));
        let in_reach = dist_to_rect(u.pos(), center, fp) <= BUILD_REACH;
        match existing {
            Some((_, false)) => u.pop(),
            Some((fid, true)) if in_reach => {
                u.path.clear();
                u.building = Some(fid);
                self.advance_frame(fid, u.owner, c.worker);
            }
            None if in_reach => {
                u.path.clear();
                if !self.can_place(def, center) {
                    self.reject(u, cmd, Reject::BadSite);
                    u.pop();
                    return;
                }
                if let Ok(fid) = self.spawn(u.owner, def, center, false) {
                    u.building = Some(fid);
                    self.advance_frame(fid, u.owner, c.worker);
                }
            }
            _ => self.approach(u, c, center, cmd),
        }
    }

    fn load_tick(&mut self, u: &mut Unit, c: &Caps, t: UnitId) {
        let ok = self.units.get(&t).filter(|tu| {
            let d = self.def(tu.def);
            tu.frame.is_none()
                && tu.carried_by.is_none()
                && self.allied(u.owner, tu.owner)
                && d.can_be_transported
                && d.size <= c.max_size
        });
        let Some(tp) = ok.map(|tu| tu.pos()) else {
            u.pop();
            return;
        };
        if u.carrying.len() >= c.capacity {
            u.pop();
            return;
        }
        if !u.pos().within(tp, 64) {
            u.work = 0;
            self.move_unit(u, c, tp, 48);
            return;
        }
        u.work += 1;
        if u.work >= i64::from(TRANSPORT_DELAY) {
            if let Some(tu) = self.units.get_mut(&t) {
                tu.carried_by = Some(u.id);
                tu.cmds.clear();
                tu.path.clear();
                tu.path_goal = None;
            }
            u.carrying.push(t);
            u.pop();
        }
    }

    fn unload_tick(&mut self, u: &mut Unit, c: &Caps, p: Pos) {
        if u.carrying.is_empty() {
            u.pop();
            return;
        }
        if self.move_unit(u, c, p, 16) == MoveResult::Moving {
            u.work = 0;
            return;
        }
        u.work += 1;
        if u.work >= i64::from(TRANSPORT_DELAY) {
            if let Some(cid) = u.carrying.pop() {
                if let Some(cu) = self.units.get_mut(&cid) {
                    cu.carried_by = None;
                    cu.set_pos(p);
                }
            }
            u.pop();
        }
    }

    /// Living non-allied units `u` sees within `limit` (and its longest
    /// range), nearest first, ties by id.
    fn targets_in_range(&self, u: &Unit, limit: i64) -> Vec<(i64, UnitId)> {
        let d = self.def(u.def);
        let range = i64::from(d.max_range()).min(limit);
        let p = u.pos();
        let mut out: Vec<(i64, UnitId)> = self
            .units
            .values()
            .filter(|t| {
                t.id != u.id
                    && t.hp > 0
                    && t.carried_by.is_none()
                    && !self.allied(u.owner, t.owner)
                    && p.within(t.pos(), range)
                    && self.sees_unit(u.owner, t)
            })
            .map(|t| (p.dist2(t.pos()), t.id))
            .collect();
        out.sort_unstable();
        out
    }

    /// Whether weapon `w` of a unit can hit `t` at all.
    fn weapon_can_hit(&self, w: &crate::view::WeaponDef, t: &Unit) -> bool {
        let td = self.def(t.def);
        if w.anti_air != td.flies {
            return false;
        }
        !w.underwater || td.floats || self.map.depth(t.pos().tile()) > 0
    }

    pub(super) fn can_hit_unit(&self, shooter: &Unit, t: &Unit) -> bool {
        let d = self.def(shooter.def);
        d.weapons
            .iter()
            .enumerate()
            .any(|(i, w)| !(d.commander && i == 2) && self.weapon_can_hit(w, t))
    }

    /// Fires every ready weapon of unit `id`.
    pub(super) fn fire(&mut self, id: UnitId) {
        let Some(u) = self.units.get(&id) else { return };
        if u.frame.is_some() || u.carried_by.is_some() || u.hp <= 0 {
            return;
        }
        let d = self.def(u.def);
        if d.weapons.is_empty() {
            return;
        }
        let forced = match u.cmds.front() {
            Some(Command::Attack(t)) => Some(*t),
            _ => None,
        };
        let cands = self.targets_in_range(u, i64::MAX);
        let p = u.pos();
        let mut shots = Vec::new();
        for (i, w) in d.weapons.iter().enumerate() {
            if (d.commander && i == 2) || u.reload.get(i).copied().unwrap_or(0) > 0 {
                continue;
            }
            let r = i64::from(w.range);
            let ok = |t: UnitId| {
                self.units
                    .get(&t)
                    .is_some_and(|tu| p.within(tu.pos(), r) && self.weapon_can_hit(w, tu))
            };
            let target = forced
                .filter(|t| cands.iter().any(|c| c.1 == *t) && ok(*t))
                .or_else(|| cands.iter().map(|c| c.1).find(|t| ok(*t)));
            if let Some(t) = target {
                shots.push((
                    i,
                    t,
                    i64::from(w.damage) * i64::from(w.burst.max(1)) * 1000,
                    w.reload,
                    w.lobbed,
                    w.energy_per_shot.0 * SUB,
                ));
            }
        }
        let owner = usize::from(u.owner.0);
        for (i, t, dmg, reload, lobbed, cost) in shots {
            if let Some(pl) = self.players.get_mut(owner) {
                if pl.energy.stock < cost {
                    pl.energy.stall_acc = true;
                    continue;
                }
                pl.energy.stock -= cost;
            }
            if let Some(r) = self.units.get_mut(&id).and_then(|u| u.reload.get_mut(i)) {
                *r = reload;
            }
            if lobbed && self.rng.below(4) == 0 {
                continue; // a miss
            }
            if let Some(tu) = self.units.get_mut(&t) {
                tu.hp -= dmg;
                tu.last_attacker = Some(id);
            }
        }
        if let Some(u) = self.units.get_mut(&id) {
            for r in &mut u.reload {
                *r = r.saturating_sub(1);
            }
        }
    }

    /// Removes a unit. A finished non-flying unit leaves a wreck if `wreck`.
    pub(super) fn remove_unit(&mut self, id: UnitId, killer: Option<UnitId>, wreck: bool) {
        let Some(u) = self.units.remove(&id) else {
            return;
        };
        let d = self.def(u.def);
        let (flies, commander, footprint, metal) = (
            d.flies,
            d.commander,
            d.footprint,
            d.cost_metal.0 * SUB * WRECK_METAL_PCT / 100,
        );
        self.events.push(Event::Died {
            tick: self.tick,
            unit: id,
            owner: u.owner,
            def: u.def,
            killer,
        });
        if wreck && u.frame.is_none() && !flies {
            self.push_feature(FeatureState {
                id: crate::view::FeatureId(0),
                kind: FeatureKind::Wreck(u.def),
                pos: u.pos(),
                footprint,
                metal,
                energy: 0,
                reclaimable: true,
            });
        }
        for c in &u.carrying {
            if let Some(cu) = self.units.get_mut(c) {
                cu.hp = 0;
            }
        }
        if commander && self.commander_death_ends {
            for o in self.units.values_mut() {
                if o.owner == u.owner {
                    o.hp = 0;
                }
            }
        }
    }

    /// Removes every unit at or below zero health, in id order.
    pub(super) fn resolve_deaths(&mut self) {
        loop {
            let dead: Vec<(UnitId, Option<UnitId>)> = self
                .units
                .values()
                .filter(|u| u.hp <= 0)
                .map(|u| (u.id, u.last_attacker))
                .collect();
            if dead.is_empty() {
                break;
            }
            for (id, killer) in dead {
                self.remove_unit(id, killer, true);
            }
        }
        for (i, pl) in self.players.iter_mut().enumerate() {
            if pl.alive {
                pl.alive = self.units.values().any(|u| usize::from(u.owner.0) == i);
            }
        }
    }
}
