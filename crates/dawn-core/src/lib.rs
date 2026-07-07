//! # dawn-core
//!
//! Pure domain model for the dawn distributed simulation platform.
//!
//! ## Invariants (see CLAUDE.md §2)
//!
//! - Zero network or I/O dependencies.
//! - All types are `Clone + Copy + serde::{Serialize, Deserialize}` where possible.
//! - No mutable global state.
//!
//! ## Example
//!
//! ```
//! use dawn_core::{MoveCommand, Position};
//!
//! let command = MoveCommand::new(Position::new(10.0, 0.0, -5.0));
//!
//! assert_eq!(command.target_position.x, 10.0);
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]

pub mod commands;
pub mod entity;
pub mod error;
pub mod events;
pub mod fitting;
pub mod item;
pub mod navigation;
pub mod player;
pub mod position;
pub mod sector;
pub mod ship_type;
pub mod tick;

// Re-export the most commonly used types at crate root for ergonomics.
pub use commands::{
    ActivateModuleCommand, ApproachCommand, ApproachTarget, AssembleCommand, AttackCommand,
    BuildPackagedShipCommand, ClientCommand, DeactivateModuleCommand, DisassembleShipCommand,
    DisembarkCommand, DockCommand, FitModuleCommand, JumpCommand, KeepAtRangeCommand,
    LockOnCommand, MoveCommand, OrbitCommand, SelectActiveShipCommand, StopCommand, UndockCommand,
    UnfitModuleCommand, WarpCommand,
};
pub use entity::{EntityId, NodeId, ShipId};
pub use error::DawnError;
pub use events::DomainEvent;
pub use fitting::{
    ActivationMode, FittingSnapshot, ModuleDefinition, ModuleId, ModuleKind, SlotEntry, SlotKind,
    StatDelta,
};
pub use item::ItemId;
pub use navigation::{
    AnchorId, CelestialBodyDef, CelestialBodyId, CelestialBodyKind, JumpGateDef, JumpGateId,
    StarSystemDef, StarSystemId, StationDef, StationId, WarpTarget,
};
pub use player::PlayerId;
pub use position::{Position, Velocity};
pub use sector::{SectorBounds, SectorId};
pub use ship_type::{ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout};
pub use tick::Tick;
