//! # dawn-sector
//!
//! Server-side Sector simulation: owns the authoritative `SimulationNode`
//! (ECS world + tick loop + event log) for one Sector, plus the static
//! galaxy map (star systems, jump gates, celestial bodies) and Area-of-
//! Interest delivery used to decide what each client is sent.
//!
//! ## Example
//!
//! ```
//! use dawn_core::{SectorBounds, SectorId};
//! use dawn_sector::node::SimulationNode;
//!
//! let node = SimulationNode::new(
//!     dawn_core::NodeId(0),
//!     SectorId(0),
//!     SectorBounds::centered(SectorBounds::DEFAULT_HALF),
//!     std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
//! );
//! assert_eq!(node.sector_id(), SectorId(0));
//! ```

// Rust API Guidelines C-DEBUG: catch new pub types that forget to derive
// Debug at compile time instead of relying on periodic audits (see #83).
#![warn(missing_debug_implementations)]
// Admission types keep the explicit `ClientAdmission` prefix so imports into
// runtime adapters remain unambiguous (`Intent` and `Attempt` are too generic).
#![allow(clippy::module_name_repetitions)]

pub mod anchor;
pub mod aoi;
pub mod aoi_frame;
pub mod client_admission;
pub mod dilation;
pub mod galaxy;
pub mod game_data;
pub mod modules;
pub mod node;
pub mod persistence;
pub mod ship_types;
pub mod spawner;
pub mod transit;
