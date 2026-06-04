//! # dawn-core
//!
//! Pure domain model for the dawn distributed simulation platform.
//!
//! ## Invariants (see CLAUDE.md §2)
//!
//! - Zero network or I/O dependencies.
//! - All types are `Clone + Copy + serde::{Serialize, Deserialize}` where possible.
//! - No mutable global state.

pub mod commands;
pub mod entity;
pub mod error;
pub mod events;
pub mod position;
pub mod sector;
pub mod tick;

// Re-export the most commonly used types at crate root for ergonomics.
pub use commands::MoveCommand;
pub use entity::{EntityId, NodeId, ShipId};
pub use error::DawnError;
pub use events::DomainEvent;
pub use position::{Position, Velocity};
pub use sector::{SectorBounds, SectorId};
pub use tick::Tick;
