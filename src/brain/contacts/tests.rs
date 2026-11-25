#![allow(clippy::unwrap_used)]

use super::*;
use crate::view::{
    Economy, Feature, MapInfo, MoveClass, OwnUnit, PlayerInfo, RadarDot, Tile, VisCell, WeaponDef,
};

/// A hand-driven view: what is visible and where we are looking.
struct Fake {
    tick: Tick,
    full: bool,
    defs: Vec<UnitDef>,
    visible: Vec<EnemyUnit>,
    /// Discs we are looking at (centre, radius).
    eyes: Vec<(Pos, i64)>,
}

impl GameView for Fake {
    fn tick(&self) -> Tick {
        self.tick
    }
    fn me(&self) -> PlayerId {
        PlayerId(0)
    }
    fn players(&self) -> Vec<PlayerInfo> {
        Vec::new()
    }
    fn full_vision(&self) -> bool {
        self.full
    }
    fn own_units(&self) -> Vec<OwnUnit> {
        Vec::new()
    }
    fn economy(&self) -> Economy {
        Economy::default()
    }
    fn unit_defs(&self) -> &[UnitDef] {
        &self.defs
    }
    fn map(&self) -> MapInfo {
        MapInfo::default()
    }
    fn height(&self, _: Tile) -> i32 {
        0
    }
    fn metal(&self, _: Tile) -> i32 {
        0
    }
    fn metal_spots(&self) -> Vec<(Tile, i32)> {
        Vec::new()
    }
    fn standable(&self, _: Tile, _: MoveClass) -> bool {
        true
    }
    fn vents(&self) -> Vec<Pos> {
        Vec::new()
    }
    fn start_positions(&self) -> Vec<Pos> {
        Vec::new()
    }
    fn visible_enemies(&self) -> Vec<EnemyUnit> {
        self.visible.clone()
    }
    fn enemy(&self, id: UnitId) -> Option<EnemyUnit> {
        self.visible.iter().copied().find(|e| e.id == id)
    }
    fn in_los(&self, p: Pos) -> bool {
        self.full || self.eyes.iter().any(|&(c, r)| c.within(p, r))
    }
    fn in_radar(&self, _: Pos) -> bool {
        false
    }
    fn radar_dots(&self) -> Vec<RadarDot> {
        Vec::new()
    }
    fn cell_visible(&self, c: VisCell) -> bool {
        self.in_los(c.center())
    }
    fn cell_explored(&self, _: VisCell) -> bool {
        true
    }
    fn visible_features(&self) -> Vec<Feature> {
        Vec::new()
    }
    fn can_hit(&self, _: UnitId, _: UnitId) -> bool {
        false
    }
}

fn defs() -> Vec<UnitDef> {
    let raider = UnitDef {
        name: "RAIDER".into(),
        mobile: true,
        cost_metal: Amount::units(50),
        weapons: vec![WeaponDef {
            range: 200,
            damage: 10,
            reload: 30,
            ..WeaponDef::default()
        }],
        ..UnitDef::default()
    };
    let mex = UnitDef {
        name: "MEX".into(),
        cost_metal: Amount::units(50),
        ..UnitDef::default()
    };
    vec![raider, mex]
}

fn enemy(id: u32, def: u16, x: i32) -> EnemyUnit {
    EnemyUnit {
        id: UnitId(id),
        owner: PlayerId(1),
        def: DefId(def),
        pos: Pos::new(x, 0),
        health: 100,
        is_frame: false,
        immune: false,
    }
}

#[test]
fn forget_only_when_looking_and_absent() {
    let mut v = Fake {
        tick: 0,
        full: false,
        defs: defs(),
        visible: vec![enemy(1, 0, 100), enemy(2, 1, 500)],
        eyes: vec![(Pos::new(300, 0), 400)],
    };
    let mut m = ContactMemory::default();
    let enemies = [PlayerId(1)];
    let ch = m.update(&v, &enemies, 0);
    assert_eq!(ch.new, vec![UnitId(1), UnitId(2)]);
    assert!(m.get(UnitId(1)).is_some_and(|c| c.armed && !c.building));
    assert!(m.get(UnitId(2)).is_some_and(|c| c.building && !c.armed));

    // We look away; both vanish from view (one moved, one died out of sight).
    // Both stay on the map, standing on remembered fields only.
    v.tick = 1;
    v.eyes = vec![(Pos::new(5000, 0), 100)];
    v.visible.clear();
    let ch = m.update(&v, &enemies, 1);
    assert!(ch.forgotten.is_empty());
    assert!(matches!(
        m.standing(&v, UnitId(1)),
        Some(Standing::Remembered(_))
    ));
    assert!(!m.seen_now(UnitId(1)));
    assert_eq!(m.target(&v, UnitId(1)), None);
    assert_eq!(
        m.get(UnitId(1)).map(|c| (c.first_seen, c.last_seen)),
        Some((0, 0))
    );

    // Looking at the mex's spot again: it is gone, so it is forgotten; the
    // raider's spot is still unwatched.
    v.tick = 2;
    v.eyes = vec![(Pos::new(500, 0), 50)];
    let ch = m.update(&v, &enemies, 2);
    assert_eq!(ch.forgotten, vec![UnitId(2)]);
    assert!(m.get(UnitId(1)).is_some());
    assert!(m.standing(&v, UnitId(2)).is_none());

    // The raider reappears elsewhere in sight: refreshed, first_seen kept.
    v.tick = 3;
    v.eyes = vec![(Pos::new(100, 0), 50), (Pos::new(900, 0), 50)];
    v.visible = vec![enemy(1, 0, 900)];
    let ch = m.update(&v, &enemies, 3);
    assert!(ch.forgotten.is_empty() && ch.new.is_empty());
    let c = m.get(UnitId(1)).copied().unwrap();
    assert_eq!((c.pos, c.first_seen, c.last_seen), (Pos::new(900, 0), 0, 3));
    assert!(matches!(m.standing(&v, UnitId(1)), Some(Standing::Live(_))));
    assert!(m.target(&v, UnitId(1)).is_some());

    // Looking at its spot while it is invisible there: not standing.
    v.tick = 4;
    v.visible.clear();
    let ch = m.update(&v, &enemies, 4);
    assert_eq!(ch.forgotten, vec![UnitId(1)]);
}

#[test]
fn allies_immune_and_full_vision() {
    let mut imm = enemy(3, 0, 10);
    imm.immune = true;
    let mut ally = enemy(4, 0, 10);
    ally.owner = PlayerId(2);
    let mut v = Fake {
        tick: 0,
        full: true,
        defs: defs(),
        visible: vec![enemy(1, 0, 100), imm, ally],
        eyes: Vec::new(),
    };
    let mut m = ContactMemory::default();
    m.update(&v, &[PlayerId(1)], 0);
    assert_eq!(m.len(), 1);
    // With full vision a contact is forgotten the moment it dies.
    v.visible.clear();
    v.tick = 1;
    let ch = m.update(&v, &[PlayerId(1)], 1);
    assert_eq!(ch.forgotten, vec![UnitId(1)]);
    assert!(m.is_empty());
}

fn building(id: u32, owner: u8, x: i32) -> Contact {
    Contact {
        id: UnitId(id),
        def: DefId(1),
        owner: PlayerId(owner),
        pos: Pos::new(x, 0),
        health: 1,
        last_seen: 0,
        first_seen: 0,
        building: true,
        armed: false,
        air: false,
        commander: false,
        builder: false,
        frame: false,
        metal: Amount::units(50),
    }
}

#[test]
fn focus_switches_only_past_the_margin() {
    let v = Fake {
        tick: 0,
        full: false,
        defs: defs(),
        visible: Vec::new(),
        eyes: Vec::new(),
    };
    let mut pic = EnemyPicture::default();
    let mut m = ContactMemory::default();
    m.contacts.insert(UnitId(1), building(1, 1, 2000));
    let alive = [PlayerId(1), PlayerId(2)];
    let inputs = |tick| PictureInputs {
        tick,
        home: Some(Pos::new(0, 0)),
        alive_enemies: &alive,
        one_enemy: true,
        switch_margin: 600,
        mix_memory: true,
        mix_halflife: 900,
        our_army_metal: Amount::ZERO,
        factories: &[],
        frame_deaths: &[],
        harass_radius: 400,
    };
    pic.update(&m, &v, inputs(0));
    assert_eq!(pic.focus, Some(PlayerId(1)));
    // Player 2 is nearer, but by less than the margin.
    m.contacts.insert(UnitId(2), building(2, 2, 1500));
    pic.update(&m, &v, inputs(1));
    assert_eq!(pic.focus, Some(PlayerId(1)));
    // Exactly the margin keeps the focus; more than it switches.
    m.contacts.insert(UnitId(3), building(3, 2, 1300));
    pic.update(&m, &v, inputs(2));
    assert_eq!(pic.focus, Some(PlayerId(1)));
    m.contacts.insert(UnitId(3), building(3, 2, 1200));
    pic.update(&m, &v, inputs(2));
    assert_eq!(pic.bases.get(&PlayerId(2)), Some(&Pos::new(1350, 0)));
    assert_eq!(pic.focus, Some(PlayerId(2)));
    assert_eq!(
        pic.enemy_base(Some(Pos::new(0, 0))),
        Some(Pos::new(1350, 0))
    );
    assert_eq!(pic.roles[ContactRole::Other.index()], 3);
}

#[test]
fn role_histogram_rises_and_halves() {
    let v = Fake {
        tick: 0,
        full: false,
        defs: defs(),
        visible: Vec::new(),
        eyes: Vec::new(),
    };
    let mut pic = EnemyPicture::default();
    let mut m = ContactMemory::default();
    for i in 0..4 {
        m.contacts.insert(UnitId(i), building(i, 1, 100));
    }
    let alive = [PlayerId(1)];
    let inputs = |tick| PictureInputs {
        tick,
        home: Some(Pos::new(0, 0)),
        alive_enemies: &alive,
        one_enemy: true,
        switch_margin: 600,
        mix_memory: true,
        mix_halflife: 900,
        our_army_metal: Amount::ZERO,
        factories: &[],
        frame_deaths: &[],
        harass_radius: 400,
    };
    pic.update(&m, &v, inputs(0));
    m.contacts.clear();
    pic.update(&m, &v, inputs(899));
    assert_eq!(pic.roles[ContactRole::Other.index()], 4);
    pic.update(&m, &v, inputs(900));
    assert_eq!(pic.roles[ContactRole::Other.index()], 2);
    assert_eq!(pic.role_share_permille(ContactRole::Other), 1000);
}
