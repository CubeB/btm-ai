//! Scripted opponents to test against. They play only through the same
//! [`GameView`] / [`FogLeaks`] / [`Orders`] contract as the AI.

use crate::view::{
    Command, DefId, FogLeaks, GameView, OrderRecorder, Orders, OwnUnit, Pos, Tile, TILE_SIZE,
};

/// A hard-coded opponent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Script {
    /// Builds two extractors and two solars, then sits still.
    Passive(State),
    /// Builds a lab, an extractor and a solar, then pumps raiders and sends
    /// every group of four to attack-move on the enemy start.
    Rush(State),
}

/// What a script remembers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    home: Option<Pos>,
}

impl Script {
    pub fn passive() -> Self {
        Script::Passive(State::default())
    }

    pub fn rush() -> Self {
        Script::Rush(State::default())
    }

    pub(super) fn think<V: GameView + FogLeaks>(&mut self, v: &V, out: &mut OrderRecorder) {
        if !v.tick().is_multiple_of(30) {
            return;
        }
        let rush = matches!(self, Script::Rush(_));
        let (Script::Passive(st) | Script::Rush(st)) = self;
        let units = v.own_units();
        let is =
            |u: &OwnUnit, f: fn(&crate::view::UnitDef) -> bool| v.unit_def(u.def).is_some_and(f);
        let Some(cmdr) = units.iter().find(|u| is(u, |d| d.commander)) else {
            return;
        };
        let home = *st.home.get_or_insert(cmdr.pos);
        let core = v
            .players()
            .iter()
            .any(|p| p.id == v.me() && p.side.eq_ignore_ascii_case("CORE"));
        let name = |arm: &str, core_name: &str| v.def_named(if core { core_name } else { arm });
        let (Some(mex), Some(solar), Some(lab), Some(raider)) = (
            name("ARMMEX", "CORMEX"),
            name("ARMSOLAR", "CORSOLAR"),
            name("ARMLAB", "CORLAB"),
            name("ARMPW", "CORAK"),
        ) else {
            return;
        };
        let count = |d: DefId| units.iter().filter(|u| u.def == d).count();
        if cmdr.commands.is_empty() {
            let want = if rush && count(lab) == 0 {
                Some(lab)
            } else if count(mex) < if rush { 1 } else { 2 } {
                Some(mex)
            } else if count(solar) < if rush { 1 } else { 2 } {
                Some(solar)
            } else {
                None
            };
            let site = match want {
                Some(d) if d == mex => mex_site(v, mex, home),
                Some(d) => ring_site(v, d, home),
                None => None,
            };
            match (want, site) {
                (Some(def), Some(at)) => out.now(cmdr.id, Command::Build { def, at }),
                _ => {
                    if let Some(l) = units.iter().find(|u| u.def == lab && u.frame.is_none()) {
                        out.now(cmdr.id, Command::Guard(l.id));
                    }
                }
            }
        }
        if !rush {
            return;
        }
        for f in units.iter().filter(|u| u.def == lab && u.frame.is_none()) {
            if f.factory.as_ref().is_some_and(|q| q.queue.is_empty()) {
                out.now(
                    f.id,
                    Command::FactoryQueue {
                        def: raider,
                        count: 1,
                    },
                );
            }
        }
        let idle: Vec<&OwnUnit> = units
            .iter()
            .filter(|u| u.def == raider && u.frame.is_none() && u.commands.is_empty())
            .collect();
        let target = v
            .start_positions()
            .into_iter()
            .filter(|s| !s.within(home, 256))
            .min_by_key(|s| (s.dist2(home), s.x, s.z));
        if let (true, Some(t)) = (idle.len() >= 4, target) {
            for u in idle {
                out.now(u.id, Command::AttackMove(t));
            }
        }
    }
}

/// The free extractor placement covering the most metal, nearest `home`.
fn mex_site<V: GameView + FogLeaks>(v: &V, mex: DefId, home: Pos) -> Option<Pos> {
    let fp = v.unit_def(mex)?.footprint;
    v.metal_spots()
        .into_iter()
        .filter_map(|(t, _)| {
            let at = Pos::new(
                t.x * TILE_SIZE + fp.0 * TILE_SIZE / 2,
                t.z * TILE_SIZE + fp.1 * TILE_SIZE / 2,
            );
            if !v.leak_can_place(mex, at) {
                return None;
            }
            let covered: i32 = (0..fp.1)
                .flat_map(|dz| (0..fp.0).map(move |dx| Tile::new(t.x + dx, t.z + dz)))
                .map(|tt| v.metal(tt))
                .sum();
            Some((-covered, at.dist2(home), at))
        })
        .min()
        .map(|(_, _, at)| at)
}

/// The first legal site on square rings round `home` that covers no metal.
fn ring_site<V: GameView + FogLeaks>(v: &V, def: DefId, home: Pos) -> Option<Pos> {
    let fp = v.unit_def(def)?.footprint;
    let pitch = (fp.0.max(fp.1) + 2) * TILE_SIZE;
    for ring in 1i32..12 {
        for dz in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs() != ring && dz.abs() != ring {
                    continue;
                }
                let at = Pos::new(home.x + dx * pitch, home.z + dz * pitch);
                let o = crate::view::footprint_origin(fp, at);
                let on_metal =
                    (0..fp.1).any(|z| (0..fp.0).any(|x| v.metal(Tile::new(o.x + x, o.z + z)) > 0));
                if !on_metal && v.leak_can_place(def, at) {
                    return Some(at);
                }
            }
        }
    }
    None
}
