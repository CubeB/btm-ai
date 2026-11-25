//! One seat's window on the sim: [`GameView`], [`FogLeaks`] and [`Orders`].

use super::{FeatureState, Sim, Unit, SUB};
use crate::view::{
    footprint_origin, Amount, BuildingSighting, DefId, Economy, EnemyUnit, FactoryState, Feature,
    FogLeaks, GameView, MapInfo, MoveClass, Order, Orders, OwnUnit, PlayerId, PlayerInfo, Pos,
    Progress, RadarDot, Resource, Tick, Tile, UnitDef, UnitId, VisCell,
};

/// A read-only, fog-filtered view of the sim for one seat.
#[derive(Clone, Copy, Debug)]
pub struct SeatView<'a> {
    sim: &'a Sim,
    me: PlayerId,
}

impl<'a> SeatView<'a> {
    pub(super) fn new(sim: &'a Sim, me: PlayerId) -> Self {
        Self { sim, me }
    }

    fn hostile(&self, u: &Unit) -> bool {
        !self.sim.allied(self.me, u.owner)
    }

    fn radar_covers(&self, p: Pos) -> bool {
        let Some(pl) = self.sim.players.get(usize::from(self.me.0)) else {
            return false;
        };
        pl.full_vision
            || self
                .sim
                .cell_index(p.vis_cell())
                .is_some_and(|i| pl.radar[i])
    }

    fn explored(&self, c: VisCell) -> bool {
        let Some(pl) = self.sim.players.get(usize::from(self.me.0)) else {
            return false;
        };
        pl.full_vision || self.sim.cell_index(c).is_some_and(|i| pl.explored[i])
    }

    /// A radar dot: a hostile unit under radar, out of sight, on the map.
    fn is_dot(&self, u: &Unit) -> bool {
        u.carried_by.is_none()
            && u.hp > 0
            && !self.sim.sees_unit(self.me, u)
            && self.radar_covers(u.pos())
    }
}

fn feature(f: &FeatureState) -> Feature {
    Feature {
        id: f.id,
        kind: f.kind,
        pos: f.pos,
        footprint: f.footprint,
        metal: Amount(f.metal / SUB),
        energy: Amount(f.energy / SUB),
        reclaimable: f.reclaimable,
    }
}

fn resource(p: &super::Pool) -> Resource {
    Resource {
        stock: Amount(p.stock / SUB),
        storage: Amount(p.storage),
        income: Amount(p.income),
        demand: Amount(p.demand),
        stalled: p.stalled,
    }
}

impl GameView for SeatView<'_> {
    fn tick(&self) -> Tick {
        self.sim.tick
    }

    fn me(&self) -> PlayerId {
        self.me
    }

    fn players(&self) -> Vec<PlayerInfo> {
        self.sim
            .players
            .iter()
            .enumerate()
            .map(|(i, p)| PlayerInfo {
                id: PlayerId(u8::try_from(i).unwrap_or(0)),
                team: p.team,
                alive: p.alive,
                side: p.side.name().to_string(),
            })
            .collect()
    }

    fn full_vision(&self) -> bool {
        self.sim
            .players
            .get(usize::from(self.me.0))
            .is_some_and(|p| p.full_vision)
    }

    fn own_units(&self) -> Vec<OwnUnit> {
        self.sim
            .units
            .values()
            .filter(|u| u.owner == self.me)
            .map(|u| {
                let d = self.sim.def(u.def);
                OwnUnit {
                    id: u.id,
                    def: u.def,
                    pos: u.pos(),
                    health: u.health(),
                    frame: u.frame.map(|f| Progress {
                        done: f.done,
                        total: f.total,
                    }),
                    commands: u.cmds.iter().copied().collect(),
                    factory: d.is_factory().then(|| FactoryState {
                        queue: u.fqueue.iter().copied().collect(),
                        building: u.building,
                    }),
                    carrying: u.carrying.clone(),
                    carried_by: u.carried_by,
                    on: u.on,
                    working_on: u.building,
                    scripted: false,
                }
            })
            .collect()
    }

    fn economy(&self) -> Economy {
        match self.sim.players.get(usize::from(self.me.0)) {
            Some(p) => Economy {
                metal: resource(&p.metal),
                energy: resource(&p.energy),
            },
            None => Economy::default(),
        }
    }

    fn unit_defs(&self) -> &[UnitDef] {
        &self.sim.defs
    }

    fn map(&self) -> MapInfo {
        self.sim.map.info()
    }

    fn height(&self, t: Tile) -> i32 {
        self.sim.map.height(t)
    }

    fn metal(&self, t: Tile) -> i32 {
        self.sim.map.metal(t)
    }

    fn metal_spots(&self) -> Vec<(Tile, i32)> {
        self.sim.map.metal_spots()
    }

    fn standable(&self, t: Tile, class: MoveClass) -> bool {
        self.sim.map.standable(t, class)
    }

    fn vents(&self) -> Vec<Pos> {
        self.sim.map.vents().to_vec()
    }

    fn start_positions(&self) -> Vec<Pos> {
        self.sim.starts.clone()
    }

    fn visible_enemies(&self) -> Vec<EnemyUnit> {
        self.sim
            .units
            .values()
            .filter(|u| u.hp > 0 && self.hostile(u) && self.sim.sees_unit(self.me, u))
            .map(|u| EnemyUnit {
                id: u.id,
                owner: u.owner,
                def: u.def,
                pos: u.pos(),
                health: u.health(),
                is_frame: u.frame.is_some(),
                immune: false,
            })
            .collect()
    }

    fn enemy(&self, id: UnitId) -> Option<EnemyUnit> {
        let u = self.sim.units.get(&id)?;
        (u.hp > 0 && self.hostile(u) && self.sim.sees_unit(self.me, u)).then(|| EnemyUnit {
            id: u.id,
            owner: u.owner,
            def: u.def,
            pos: u.pos(),
            health: u.health(),
            is_frame: u.frame.is_some(),
            immune: false,
        })
    }

    fn in_los(&self, p: Pos) -> bool {
        self.sim.sees_cell(self.me, p.vis_cell())
    }

    fn in_radar(&self, p: Pos) -> bool {
        self.radar_covers(p)
    }

    fn radar_dots(&self) -> Vec<RadarDot> {
        self.sim
            .units
            .values()
            .filter(|u| self.hostile(u) && self.is_dot(u))
            .map(|u| RadarDot {
                id: u.id,
                pos: u.pos(),
            })
            .collect()
    }

    fn cell_visible(&self, c: VisCell) -> bool {
        self.sim.sees_cell(self.me, c)
    }

    fn cell_explored(&self, c: VisCell) -> bool {
        self.explored(c)
    }

    fn visible_features(&self) -> Vec<Feature> {
        self.sim
            .features
            .values()
            .filter(|f| self.sim.sees_cell(self.me, f.pos.vis_cell()))
            .map(feature)
            .collect()
    }

    fn can_hit(&self, shooter: UnitId, target: UnitId) -> bool {
        let (Some(s), Some(t)) = (self.sim.units.get(&shooter), self.sim.units.get(&target)) else {
            return false;
        };
        s.owner == self.me
            && self.hostile(t)
            && self.sim.sees_unit(self.me, t)
            && self.sim.can_hit_unit(s, t)
    }
}

impl FogLeaks for SeatView<'_> {
    fn leak_can_place(&self, def: DefId, at: Pos) -> bool {
        usize::from(def.0) < self.sim.defs.len()
            && self.sim.map.info().contains(at)
            && self.sim.can_place(def, at)
    }

    fn leak_buildings_near(&self, at: Pos, radius: i32) -> Vec<BuildingSighting> {
        self.sim
            .units
            .values()
            .filter(|u| !self.sim.def(u.def).mobile && u.pos().within(at, i64::from(radius)))
            .map(|u| BuildingSighting {
                id: u.id,
                owner: u.owner,
                def: u.def,
                pos: u.pos(),
                is_frame: u.frame.is_some(),
            })
            .collect()
    }

    fn leak_explored_features(&self) -> Vec<Feature> {
        self.sim
            .features
            .values()
            .filter(|f| self.explored(f.pos.vis_cell()))
            .map(feature)
            .collect()
    }

    fn leak_features_near(&self, at: Pos, radius: i32) -> Vec<Feature> {
        self.sim
            .features
            .values()
            .filter(|f| f.pos.within(at, i64::from(radius)))
            .map(feature)
            .collect()
    }

    fn leak_vent_occupied(&self, vent: Pos) -> bool {
        let t = vent.tile();
        self.sim.units.values().any(|u| {
            let d = self.sim.def(u.def);
            let o = footprint_origin(d.footprint, u.pos());
            !d.mobile
                && t.x >= o.x
                && t.z >= o.z
                && t.x < o.x + d.footprint.0
                && t.z < o.z + d.footprint.1
        })
    }

    fn leak_radar_dot_mobile(&self, id: UnitId) -> Option<bool> {
        let u = self.sim.units.get(&id)?;
        (self.hostile(u) && self.is_dot(u)).then(|| self.sim.def(u.def).mobile)
    }

    fn leak_radar_dots_with_allies(&self) -> Vec<RadarDot> {
        self.sim
            .units
            .values()
            .filter(|u| {
                u.owner != self.me
                    && u.carried_by.is_none()
                    && self.radar_covers(u.pos())
                    && (!self.hostile(u) || self.is_dot(u))
            })
            .map(|u| RadarDot {
                id: u.id,
                pos: u.pos(),
            })
            .collect()
    }
}

/// An [`Orders`] sink that submits straight into the sim for one seat. The
/// sim's order delay applies.
#[derive(Debug)]
pub struct SeatOrders<'a> {
    sim: &'a mut Sim,
    me: PlayerId,
}

impl<'a> SeatOrders<'a> {
    pub(super) fn new(sim: &'a mut Sim, me: PlayerId) -> Self {
        Self { sim, me }
    }
}

impl Orders for SeatOrders<'_> {
    fn issue(&mut self, order: Order) {
        self.sim.submit(self.me, [order]);
    }
}
