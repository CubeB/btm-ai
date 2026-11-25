//! Reclaim (SPEC §4.12): field reclaim behind a wave, our own wreckage, and
//! the harvest fallback of an idle builder (§4.2 rule 11).

use std::collections::{BTreeMap, BTreeSet};

use super::super::World;
use super::facts::{dist, secs, Facts};
use super::siting::{self, Builder};
use crate::knobs::Knobs;
use crate::view::{
    Command, Feature, FeatureId, FeatureKind, FogLeaks, GameView, Pos, Tick, UnitId,
};

/// Wrecks needed to send a field worker, and below which it is recalled
/// (SPEC §4.12).
const FIELD_SEND: usize = 4;
const FIELD_RECALL: usize = 2;
/// A wreck counts for a death site within this (SPEC §4.12).
const SITE_RADIUS: i64 = 32;
/// A site with no wreck is dropped after this (SPEC §4.12).
const SITE_PRUNE_S: i32 = 5;
/// The harvest radius round home (SPEC §4.2 rule 11).
pub const HARVEST_RADIUS: i64 = 1200;

/// Reclaim state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reclaim {
    /// The one field worker.
    pub field_worker: Option<UnitId>,
    /// Builders on our own wreckage.
    own: BTreeSet<UnitId>,
    /// Death sites (position, tick of death) first seen empty, with when.
    empty_since: BTreeMap<(i32, i32, Tick), Tick>,
}

/// Features a builder's queue or in-flight orders already target.
pub fn claimed_features(w: &World, in_flight: &[crate::view::Order]) -> BTreeSet<FeatureId> {
    let mut out = BTreeSet::new();
    for s in &w.own.units {
        for c in &s.unit.commands {
            if let Command::ReclaimFeature(f) = *c {
                out.insert(f);
            }
        }
    }
    for o in in_flight {
        if let Command::ReclaimFeature(f) = o.command {
            out.insert(f);
        }
    }
    out
}

fn is_wreck(f: &Feature) -> bool {
    matches!(f.kind, FeatureKind::Wreck(_))
}

impl Reclaim {
    /// Wrecks round the wave centre that a field worker would take (SPEC
    /// §4.12 "count"), nearest `to` first.
    fn field_wrecks(
        view: &dyn GameView,
        leaks: &dyn FogLeaks,
        k: &Knobs,
        w: &World,
        to: Pos,
    ) -> Vec<Feature> {
        let Some(centre) = w.army.wave_centre else {
            return Vec::new();
        };
        if w.army.wave.len() < usize::try_from(k.reclaim.field_escort).unwrap_or(usize::MAX) {
            return Vec::new();
        }
        let r = dist(k.reclaim.field_radius);
        let mut v: Vec<Feature> = leaks
            .leak_explored_features()
            .into_iter()
            .filter(|f| is_wreck(f) && f.reclaimable && f.metal.milli() > 0)
            .filter(|f| f.pos.within(centre, r))
            .filter(|f| !siting::under_guns(view, k, w, f.pos))
            .collect();
        v.sort_by_key(|f| (f.pos.dist2(to), f.id));
        v
    }

    /// Recalls the field worker when fewer than 2 wrecks remain or the wave
    /// ends; returns its move home.
    pub fn recall(
        &mut self,
        view: &dyn GameView,
        leaks: &dyn FogLeaks,
        k: &Knobs,
        w: &World,
    ) -> Option<(UnitId, Command)> {
        let id = self.field_worker?;
        let Some(s) = w.own.get(id).filter(|_| w.own.builders.contains(&id)) else {
            self.field_worker = None;
            return None;
        };
        let left = Self::field_wrecks(view, leaks, k, w, s.unit.pos).len();
        if left < FIELD_RECALL {
            self.field_worker = None;
            return w.own.home.map(|h| (id, Command::Move(h)));
        }
        None
    }

    /// Field reclaim for this builder (SPEC §4.12): one non-commander builder
    /// at base, at ≥ 4 wrecks, up to `reclaim.field_batch` reclaims queued.
    pub fn field(
        &mut self,
        view: &dyn GameView,
        leaks: &dyn FogLeaks,
        k: &Knobs,
        w: &World,
        b: &Builder,
    ) -> Vec<FeatureId> {
        if self.field_worker.is_some() || b.commander || !b.at_base {
            return Vec::new();
        }
        let wrecks = Self::field_wrecks(view, leaks, k, w, b.pos);
        if wrecks.len() < FIELD_SEND {
            return Vec::new();
        }
        self.field_worker = Some(b.id);
        let n = usize::try_from(k.reclaim.field_batch.max(1)).unwrap_or(1);
        wrecks.iter().take(n).map(|f| f.id).collect()
    }

    /// Own-wreck reclaim (SPEC §4.12).
    #[allow(clippy::too_many_arguments)]
    pub fn own_wrecks(
        &mut self,
        view: &dyn GameView,
        leaks: &dyn FogLeaks,
        k: &Knobs,
        w: &World,
        f: &Facts,
        b: &Builder,
        claimed: &BTreeSet<FeatureId>,
    ) -> Vec<FeatureId> {
        // Reclaimers that went idle are done.
        let idle: BTreeSet<UnitId> = w.own.idle_builders.iter().copied().collect();
        self.own
            .retain(|u| w.own.builders.contains(u) && !idle.contains(u));
        if f.near_base || (k.reclaim.own_only_when_short && !f.metal_short) {
            return Vec::new();
        }
        if self.own.len() >= usize::try_from(k.reclaim.own_reclaimers.max(0)).unwrap_or(0) {
            return Vec::new();
        }
        let delay = secs(k.reclaim.own_delay_s);
        let prune = secs(SITE_PRUNE_S);
        let mut ripe: Vec<Feature> = Vec::new();
        for site in &w.own.wreck_sites {
            let key = (site.pos.x, site.pos.z, site.tick);
            if self
                .empty_since
                .get(&key)
                .is_some_and(|&t| f.tick.saturating_sub(t) >= prune)
            {
                continue;
            }
            let here: Vec<Feature> = leaks
                .leak_features_near(site.pos, i32::try_from(SITE_RADIUS).unwrap_or(32))
                .into_iter()
                .filter(|x| is_wreck(x) && x.reclaimable)
                .collect();
            if here.is_empty() {
                self.empty_since.entry(key).or_insert(f.tick);
                continue;
            }
            self.empty_since.remove(&key);
            if f.tick.saturating_sub(site.tick) < delay {
                continue;
            }
            for x in here {
                if !claimed.contains(&x.id)
                    && !siting::under_guns(view, k, w, x.pos)
                    && !ripe.iter().any(|r| r.id == x.id)
                {
                    ripe.push(x);
                }
            }
        }
        let live: BTreeSet<(i32, i32, Tick)> = w
            .own
            .wreck_sites
            .iter()
            .map(|s| (s.pos.x, s.pos.z, s.tick))
            .collect();
        self.empty_since.retain(|key, _| live.contains(key));
        if ripe.is_empty() {
            return Vec::new();
        }
        ripe.sort_by_key(|x| (x.pos.dist2(b.pos), x.id));
        self.own.insert(b.id);
        let n = usize::try_from(k.reclaim.own_batch.max(1)).unwrap_or(1);
        ripe.iter().take(n).map(|x| x.id).collect()
    }
}

/// The harvest fallback (SPEC §4.2 rule 11): builders at base and ships;
/// reclaimable features on explored ground within 1200 of home (or the
/// ship), not under guns, with metal (or energy when energy is wanted);
/// metal-bearing first, then the nearest.
pub fn harvest(
    view: &dyn GameView,
    leaks: &dyn FogLeaks,
    k: &Knobs,
    w: &World,
    f: &Facts,
    b: &Builder,
) -> Option<FeatureId> {
    if !b.at_base && !b.naval {
        return None;
    }
    let centre = if b.naval { b.pos } else { f.home? };
    leaks
        .leak_explored_features()
        .into_iter()
        .filter(|x| x.reclaimable && x.pos.within(centre, HARVEST_RADIUS))
        .filter(|x| x.metal.milli() > 0 || (f.energy_wanted && x.energy.milli() > 0))
        .filter(|x| !b.naval || view.water_depth(x.pos.tile()) > 0)
        .filter(|x| !siting::under_guns(view, k, w, x.pos))
        .min_by_key(|x| (x.metal.milli() <= 0, x.pos.dist2(b.pos), x.id))
        .map(|x| x.id)
}
