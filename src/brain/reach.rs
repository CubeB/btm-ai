//! Reachability layers and the flags derived from them (SPEC §3.4).
//!
//! Each layer is a 4-connected flood fill of "standable" tiles for one
//! movement class and footprint: a tile is standable when the mover's whole
//! footprint, with that tile as its top-left corner, is standable. The fill
//! labels every region once and is recomputed only when the movement class
//! changes; otherwise the layer is just re-homed (a new set of claimed
//! regions). A layer that has not been built answers "reachable" to
//! everything.
//!
//! Known weakness (kept from the spec): the fill is 4-connected while the
//! pathfinder moves in 8 directions, so narrow diagonal passes can read as
//! cut.

use super::mapintel::{label, tile_index, MapCharacter, MapIntel, Prefix};
use super::own::{OwnState, UnitClass};
use super::roles::{Role, RoleTable};
use crate::view::{footprint_origin, DefId, GameView, MoveClass, Pos, Tick, Tile};

/// One reachability layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// The mover's type.
    pub mover: DefId,
    class: MoveClass,
    footprint: (i32, i32),
    w: i32,
    h: i32,
    labels: Vec<u32>,
    sizes: Vec<u32>,
    /// The claimed region labels.
    home: Vec<u32>,
    /// The tile the layer is homed on.
    pub home_tile: Tile,
    /// Standable tiles in all regions.
    pub walkable: u32,
}

impl Layer {
    fn build(view: &dyn GameView, mover: DefId) -> Option<Layer> {
        let d = view.unit_def(mover)?;
        let info = view.map();
        let (w, h) = (info.width_tiles.max(0), info.height_tiles.max(0));
        let base: Vec<bool> = (0..h)
            .flat_map(|z| (0..w).map(move |x| Tile::new(x, z)))
            .map(|t| view.standable(t, d.move_class))
            .collect();
        let pre = Prefix::new(w, h, |i| base.get(i).copied().unwrap_or(false));
        let (fx, fz) = (d.footprint.0.max(1), d.footprint.1.max(1));
        let fits = |i: usize| {
            let (x, z) = ((i as i32) % w.max(1), (i as i32) / w.max(1));
            pre.all(x, z, fx, fz)
        };
        let (labels, count) = label(w, h, fits, false);
        let mut sizes = vec![0u32; count as usize + 1];
        for &l in &labels {
            if let Some(s) = sizes.get_mut(l as usize) {
                *s += 1;
            }
        }
        let walkable = sizes.iter().skip(1).sum();
        Some(Layer {
            mover,
            class: d.move_class,
            footprint: (fx, fz),
            w,
            h,
            labels,
            sizes,
            home: Vec::new(),
            home_tile: Tile::default(),
            walkable,
        })
    }

    fn label_at(&self, t: Tile) -> u32 {
        tile_index(self.w, self.h, t)
            .and_then(|i| self.labels.get(i).copied())
            .unwrap_or(0)
    }

    /// Claims the region of `t`, or its four neighbours' regions if `t` is not
    /// standable.
    fn rehome(&mut self, t: Tile) {
        self.home_tile = t;
        self.home.clear();
        let l = self.label_at(t);
        if l > 0 {
            self.home.push(l);
        } else {
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = self.label_at(Tile::new(t.x + dx, t.z + dz));
                if n > 0 && !self.home.contains(&n) {
                    self.home.push(n);
                }
            }
        }
        self.home.sort_unstable();
    }

    /// Whether a tile (a footprint's top-left) is in a claimed region.
    pub fn reachable_tile(&self, t: Tile) -> bool {
        let l = self.label_at(t);
        l > 0 && self.home.binary_search(&l).is_ok()
    }

    /// Whether a unit of `footprint` centred at `p` is reachable.
    pub fn reachable(&self, footprint: (i32, i32), p: Pos) -> bool {
        self.reachable_tile(footprint_origin(footprint, p))
    }

    /// Standable tiles in the claimed regions.
    pub fn reachable_count(&self) -> u32 {
        self.home
            .iter()
            .filter_map(|&l| self.sizes.get(l as usize))
            .sum()
    }

    /// The mover's footprint origin for a point.
    pub fn origin(&self, p: Pos) -> Tile {
        footprint_origin(self.footprint, p)
    }
}

/// Answers "reachable" for a layer that may not be built.
fn reachable_opt(layer: Option<&Layer>, p: Pos) -> bool {
    layer.is_none_or(|l| l.reachable_tile(l.origin(p)))
}

/// The reachability layers and derived flags (SPEC §3.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reachability {
    /// The side's level-one constructor's layer (the commander's if none),
    /// homed on home, or on the ground home.
    pub ground: Option<Layer>,
    /// The first walkable factory, when home reaches no own factory.
    pub ground_home: Option<Pos>,
    /// The commander's own class (`cmd.own_reachability`).
    pub commander: Option<Layer>,
    /// Non-Land maps only: the sea transport, else destroyer, submarine,
    /// scout ship or construction ship; homed on our shipyard, else the
    /// shipyard site nearest home.
    pub naval: Option<Layer>,
    /// Non-Land maps with at least 2 starts, for data with a hovercraft.
    pub hover: Option<Layer>,
    /// Walkable tiles exceed reachable tiles by more than 64.
    pub unreachable_ground_exists: bool,
    /// Another start is reachable on the ground layer (unknown counts yes).
    pub land_route_to_enemy: bool,
    /// The same test on the hover layer.
    pub hover_route_to_enemy: bool,
    /// The last re-home.
    pub last_rehome: Option<Tick>,
}

/// Ticks between re-homes (SPEC §2.2: every 20 s).
pub const REHOME_TICKS: Tick = 20 * 30;

impl Reachability {
    /// Whether the re-home is due: at the first chance (a home exists), then
    /// every 20 s.
    pub fn due(&self, tick: Tick, own: &OwnState) -> bool {
        own.home.is_some()
            && self
                .last_rehome
                .is_none_or(|t| tick.saturating_sub(t) >= REHOME_TICKS)
    }

    /// Rebuilds layers whose mover class changed, re-homes every layer and
    /// recomputes the flags.
    pub fn update(
        &mut self,
        view: &dyn GameView,
        roles: &RoleTable,
        own: &OwnState,
        map: &MapIntel,
        tick: Tick,
    ) {
        let Some(home) = own.home else { return };
        self.last_rehome = Some(tick);
        let commander_def = own.commander.and_then(|c| own.get(c)).map(|s| s.unit.def);

        // Ground.
        let ground_mover = roles.get(Role::Constructor).or(commander_def);
        refresh(&mut self.ground, view, ground_mover);
        self.ground_home = None;
        if let Some(g) = self.ground.as_mut() {
            g.rehome(g.origin(home));
            let factories: Vec<(Pos, (i32, i32))> = own
                .units
                .iter()
                .filter(|s| s.class == UnitClass::Factory && !s.is_frame())
                .filter_map(|s| view.unit_def(s.unit.def).map(|d| (s.unit.pos, d.footprint)))
                .collect();
            // A factory's query point is its own footprint's top-left tile.
            let any_reached = factories.iter().any(|&(p, fp)| g.reachable(fp, p));
            if !factories.is_empty() && !any_reached {
                let walkable = factories
                    .iter()
                    .find(|&&(p, fp)| g.label_at(footprint_origin(fp, p)) > 0)
                    .copied();
                if let Some((p, fp)) = walkable {
                    g.rehome(footprint_origin(fp, p));
                    self.ground_home = Some(p);
                }
            }
        }

        // Commander.
        refresh(&mut self.commander, view, commander_def);
        if let (Some(l), Some(c)) = (
            self.commander.as_mut(),
            own.commander.and_then(|c| own.get(c)),
        ) {
            l.rehome(l.origin(c.unit.pos));
        }

        // Naval.
        let wet = map.character != MapCharacter::Land;
        let naval_mover = if wet {
            [
                Role::SeaTransport,
                Role::Destroyer,
                Role::Submarine,
                Role::ScoutShip,
                Role::ConstructionShip,
            ]
            .iter()
            .find_map(|&r| roles.get(r))
        } else {
            None
        };
        refresh(&mut self.naval, view, naval_mover);
        if let Some(l) = self.naval.as_mut() {
            let yard = own
                .units
                .iter()
                .find(|s| {
                    !s.is_frame()
                        && (roles.is(s.unit.def, Role::Shipyard)
                            || roles.is(s.unit.def, Role::AdvancedShipyard))
                })
                .map(|s| s.unit.pos)
                .or_else(|| map.shipyard_site_near(home).map(|s| s.center()));
            if let Some(p) = yard {
                l.rehome(l.origin(p));
            }
        }

        // Hover.
        let hover_mover = if wet && map.starts.len() >= 2 {
            roles
                .get(Role::HoverTank)
                .or_else(|| roles.get(Role::HoverAntiAir))
                .or_else(|| {
                    view.unit_defs()
                        .iter()
                        .position(|d| d.mobile && d.move_class.hovers && !d.flies)
                        .and_then(|i| u16::try_from(i).ok())
                        .map(DefId)
                })
        } else {
            None
        };
        refresh(&mut self.hover, view, hover_mover);
        if let Some(l) = self.hover.as_mut() {
            l.rehome(l.origin(home));
        }

        // Flags.
        self.unreachable_ground_exists = self
            .ground
            .as_ref()
            .is_some_and(|g| g.walkable > g.reachable_count() + 64);
        let others: Vec<Pos> = map
            .starts
            .iter()
            .copied()
            .filter(|s| !s.within(home, 256))
            .collect();
        let route = |layer: Option<&Layer>| {
            map.starts.len() < 2 || others.iter().any(|&s| reachable_opt(layer, s))
        };
        self.land_route_to_enemy = route(self.ground.as_ref());
        self.hover_route_to_enemy = route(self.hover.as_ref());
    }

    /// Ground reachability of a point (true while the layer is not built).
    pub fn ground_reachable(&self, p: Pos) -> bool {
        reachable_opt(self.ground.as_ref(), p)
    }

    /// Commander-layer reachability of a point.
    pub fn commander_reachable(&self, p: Pos) -> bool {
        reachable_opt(self.commander.as_ref(), p)
    }

    /// Naval reachability of a point.
    pub fn naval_reachable(&self, p: Pos) -> bool {
        reachable_opt(self.naval.as_ref(), p)
    }

    /// Hover reachability of a point.
    pub fn hover_reachable(&self, p: Pos) -> bool {
        reachable_opt(self.hover.as_ref(), p)
    }
}

/// Keeps `layer` if its mover's class and footprint are unchanged, else
/// rebuilds it (or clears it when there is no mover).
fn refresh(layer: &mut Option<Layer>, view: &dyn GameView, mover: Option<DefId>) {
    let Some(m) = mover else {
        *layer = None;
        return;
    };
    let Some(d) = view.unit_def(m) else {
        *layer = None;
        return;
    };
    let fp = (d.footprint.0.max(1), d.footprint.1.max(1));
    let same = layer
        .as_ref()
        .is_some_and(|l| l.class == d.move_class && l.footprint == fp);
    if !same {
        *layer = Layer::build(view, m);
    } else if let Some(l) = layer.as_mut() {
        l.mover = m;
    }
}
