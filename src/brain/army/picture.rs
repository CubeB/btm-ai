//! The phase step (SPEC §3.1 step 5), every tick: the threat picture round
//! home (§5.4), the radar early warning (§5.4), the odds gate and the phase
//! machine (§5.1).

use std::collections::{BTreeMap, BTreeSet};

use super::geom::centroid;
use super::{
    at_least, dist, metal_of, secs, ArmyManager, BESIEGED_FACTORY_RADIUS, RADAR_CLOSING, RADAR_HOLD,
};
use crate::brain::threat::RadarWarning;
use crate::brain::{sector_of, Ctx, Phase, Standing};
use crate::view::{Amount, Pos, Tick, UnitId};

/// What the phase step remembers between ticks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PictureState {
    /// Last tick's intruders, so a contact's metal enters its sector once.
    pub(crate) intruders: BTreeSet<UnitId>,
    /// Each sampled unit's distance from home at the last radar sample.
    pub(crate) radar_last: BTreeMap<UnitId, i64>,
    /// The odds gate (§5.1).
    pub(crate) odds_open: bool,
    /// The last tick spent in Assault, or the first Growth tick (§5.1
    /// patience).
    pub(crate) last_assault: Option<Tick>,
    /// The wave is spent (§5.2); kept here because the phase machine reads
    /// it. Cleared when Assault ends.
    pub(crate) spent: bool,
}

/// The known enemy mobile ground army's metal: remembered armed mobile
/// non-air contacts, commanders excepted (they are not army; SPEC §5.1).
pub fn enemy_army_metal(ctx: &Ctx<'_>) -> Amount {
    ctx.world
        .contacts
        .iter()
        .filter(|c| c.armed_ground_mobile() && !c.commander)
        .fold(Amount::ZERO, |a, c| a + c.metal)
}

impl ArmyManager {
    pub(crate) fn phase_step(&mut self, ctx: &mut Ctx<'_>) {
        self.threat_picture(ctx);
        if ctx.due.radar {
            self.radar_sample(ctx);
        }
        let tick = ctx.tick;
        let w = &mut ctx.world.threat.radar_warning;
        if w.is_some_and(|r| tick > r.until) {
            *w = None;
        }
        self.odds_gate(ctx);
        self.phase_machine(ctx);
    }

    /// Intruders, the target intruder and the sector memory (§5.4).
    fn threat_picture(&mut self, ctx: &mut Ctx<'_>) {
        let k = &ctx.knobs.army;
        let home = ctx.world.own.home;
        let defend = dist(k.defend_radius);
        let besieged: Vec<Pos> = ctx
            .world
            .enemy
            .besieged_factories
            .iter()
            .filter_map(|&f| ctx.world.own.get(f).map(|s| s.unit.pos))
            .collect();
        let intruders: Vec<UnitId> = ctx
            .world
            .contacts
            .iter()
            .filter(|c| c.armed)
            .filter(|c| {
                home.is_some_and(|h| c.pos.within(h, defend))
                    || besieged
                        .iter()
                        .any(|&f| c.pos.within(f, BESIEGED_FACTORY_RADIUS))
            })
            .map(|c| c.id)
            .collect();
        let mut target: Option<(i64, UnitId)> = None;
        let mut metal = Amount::ZERO;
        let mut count = 0u32;
        for &id in &intruders {
            let Some(c) = ctx.world.contacts.get(id) else {
                continue;
            };
            if !ctx.world.contacts.seen_now(id) || c.frame {
                continue;
            }
            if !matches!(ctx.standing(id), Some(Standing::Live(_))) {
                continue;
            }
            metal += c.metal;
            count += 1;
            let d = home.map_or(0, |h| c.pos.dist2(h));
            if target.is_none_or(|(b, _)| d < b) {
                target = Some((d, id));
            }
        }
        // Sector memory: a contact's metal is added when it first becomes
        // an intruder, again if it leaves and comes back.
        let mut adds: Vec<(usize, Amount)> = Vec::new();
        if let (true, Some(h)) = (k.sectors, home) {
            for &id in &intruders {
                if self.pic.intruders.contains(&id) {
                    continue;
                }
                if let Some(c) = ctx.world.contacts.get(id) {
                    adds.push((sector_of(h, c.pos), c.metal));
                }
            }
        }
        let halflife = secs(k.sector_halflife_s);
        let t = &mut ctx.world.threat;
        for (s, m) in adds {
            t.sectors.add(s, m);
        }
        t.sectors.decay(ctx.tick, halflife);
        t.target_intruder = target.map(|(_, id)| id);
        t.intruder_metal = metal;
        t.intruder_count = count;
        self.pic.intruders = intruders.iter().copied().collect();
        t.intruders = intruders;
    }

    /// The radar early-warning sample (§5.4), once a second.
    fn radar_sample(&mut self, ctx: &mut Ctx<'_>) {
        let Some(home) = ctx.world.own.home else {
            self.pic.radar_last.clear();
            return;
        };
        let k = &ctx.knobs.army;
        let ring = k.radar_rings.apply(dist(k.defend_radius));
        let mut now: BTreeMap<UnitId, (i64, Pos)> = BTreeMap::new();
        // Seen units must be armed.
        for c in ctx.world.contacts.iter() {
            if ctx.world.contacts.seen_now(c.id)
                && !c.building
                && c.armed
                && c.pos.within(home, ring)
            {
                now.insert(c.id, (c.pos.dist(home), c.pos));
            }
        }
        // Unseen ones must be on radar. Mobility of a dot is the reference
        // AI's named leak (SPEC §1.4 "radar warning").
        for dot in ctx.view.radar_dots() {
            if now.contains_key(&dot.id) || !dot.pos.within(home, ring) {
                continue;
            }
            if ctx.leaks.leak_radar_dot_mobile(dot.id) == Some(true) {
                now.insert(dot.id, (dot.pos.dist(home), dot.pos));
            }
        }
        let closing: Vec<Pos> = now
            .iter()
            .filter(|(id, (d, _))| {
                self.pic
                    .radar_last
                    .get(id)
                    .is_some_and(|&last| *d + RADAR_CLOSING < last)
            })
            .map(|(_, &(_, p))| p)
            .collect();
        self.pic.radar_last = now.into_iter().map(|(id, (d, _))| (id, d)).collect();
        let min = usize::try_from(k.radar_min_contacts.max(1)).unwrap_or(usize::MAX);
        if closing.len() >= min {
            if let Some(toward) = centroid(closing) {
                ctx.world.threat.radar_warning = Some(RadarWarning {
                    toward,
                    until: ctx.tick.saturating_add(RADAR_HOLD),
                });
            }
        }
    }

    /// The odds gate (§5.1).
    fn odds_gate(&mut self, ctx: &Ctx<'_>) {
        let k = &ctx.knobs.army;
        let theirs = enemy_army_metal(ctx).milli();
        let ours = ctx.world.enemy.our_army_metal.milli();
        if !k.odds || theirs <= 0 {
            self.pic.odds_open = false;
            return;
        }
        if self.pic.odds_open {
            if !at_least(ours, k.odds_close, theirs) {
                self.pic.odds_open = false;
            }
        } else {
            let need = k.spent_size.max((k.wave_size + 1) / 2).max(0);
            let army = i64::from(ctx.world.own.army_size);
            if army >= i64::from(need) && at_least(ours, k.odds_open, theirs) {
                self.pic.odds_open = true;
            }
        }
    }

    /// The strategic phase (§5.1).
    fn phase_machine(&mut self, ctx: &mut Ctx<'_>) {
        let k = &ctx.knobs.army;
        let tick = ctx.tick;
        let phase = ctx.world.army.phase;
        let intruders = !ctx.world.threat.intruders.is_empty();
        let factory = !ctx.world.own.factories.is_empty();
        let army = i64::from(ctx.world.own.army_size);
        let spent_size = i64::from(k.spent_size);

        // 1. The Defend override.
        if intruders {
            let wave_out =
                k.waves && k.reserve_defends && phase == Phase::Assault && !self.pic.spent;
            let recall = wave_out && k.proportional && k.sectors && {
                let wave_metal: i64 = ctx
                    .world
                    .army
                    .wave
                    .iter()
                    .filter_map(|&u| ctx.world.own.get(u))
                    .map(|s| metal_of(ctx.def(s.unit.def)).milli())
                    .sum();
                // Sector threat is whole metal; the wave's is milli.
                !at_least(
                    wave_metal,
                    k.recall_ratio,
                    ctx.world.threat.sectors.total().saturating_mul(1000),
                )
            };
            if !wave_out || recall {
                if phase != Phase::Defend {
                    self.dissolve(ctx);
                }
                ctx.world.army.phase = Phase::Defend;
                return;
            }
        }
        let next = match phase {
            Phase::Setup => {
                if factory {
                    Phase::Growth
                } else {
                    Phase::Setup
                }
            }
            Phase::Growth => {
                let last = *self.pic.last_assault.get_or_insert(tick);
                let target = ctx.world.enemy_base().is_some() || !ctx.world.contacts.is_empty();
                let hulls = i64::try_from(ctx.world.own.hulls.len()).unwrap_or(i64::MAX);
                let navy = ctx.knobs.navy.attack_size;
                let patience =
                    tick.saturating_sub(last) >= secs(k.patience_s) && army >= spent_size;
                let o = &ctx.knobs.open;
                let timed = o.timed
                    && tick >= secs(o.first_wave_by_s)
                    && army >= i64::from(o.first_wave_size)
                    && enemy_army_metal(ctx) <= ctx.world.enemy.our_army_metal;
                let go = army >= i64::from(k.wave_size)
                    || (navy > 0 && hulls >= i64::from(navy))
                    || patience
                    || self.pic.odds_open
                    || timed;
                if target && go {
                    Phase::Assault
                } else {
                    Phase::Growth
                }
            }
            Phase::Assault => {
                let done = if k.waves {
                    self.pic.spent
                } else {
                    army < spent_size
                };
                if done {
                    self.dissolve(ctx);
                    self.pic.odds_open = false;
                    Phase::Growth
                } else {
                    Phase::Assault
                }
            }
            Phase::Defend => {
                if factory {
                    Phase::Growth
                } else {
                    Phase::Setup
                }
            }
        };
        if next == Phase::Assault {
            self.pic.last_assault = Some(tick);
        }
        ctx.world.army.phase = next;
    }

    /// Dissolves the wave, its reserve and the raid (entering Defend, or a
    /// spent wave going home).
    pub(crate) fn dissolve(&mut self, ctx: &mut Ctx<'_>) {
        let a = &mut ctx.world.army;
        a.wave.clear();
        a.reserve.clear();
        a.wave_centre = None;
        a.contact_since = None;
        self.pic.spent = false;
        self.tac.wave_health.clear();
        self.tac.raid.clear();
        self.tac.raid_released = false;
    }
}
