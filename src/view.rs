//! The AI's contract with a game: what it may read and what it may order.
//!
//! This module is engine-neutral. An adapter for a real engine implements
//! [`GameView`] (fog-honest reads, SPEC §1.2), [`FogLeaks`] (the reference
//! AI's named fog leaks, SPEC §1.4) and [`Orders`] (the order stream, SPEC
//! §2.1). The headless test simulation in [`crate::sim`] implements all three.
//!
//! Determinism (SPEC §7): every value that can influence an order is an
//! integer. Positions are integer world units, resources are integer
//! milli-units, and time is counted in simulation ticks. Every list a view
//! returns is in a defined order (unit id, feature id, player id or grid scan
//! order), never hash or pointer order.

mod defs;
mod game;
mod orders;
mod types;

pub use defs::{MoveClass, TransportDef, UnitDef, WeaponDef};
pub use game::{
    footprint_origin, BuildingSighting, Economy, EnemyUnit, FactoryState, Feature, FeatureKind,
    FogLeaks, GameView, MapInfo, OwnUnit, PlayerInfo, Progress, RadarDot, Resource,
};
pub use orders::{Command, Order, OrderRecorder, Orders, Queue};
pub use types::{
    secs, Amount, DefId, FeatureId, PlayerId, Pos, TeamId, Tick, Tile, UnitId, VisCell,
    TICKS_PER_SECOND, TILE_SIZE, VIS_CELL_SIZE,
};
