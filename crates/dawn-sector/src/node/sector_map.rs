//! Navigation topology for one Sector: star map, jump gates, celestial bodies.
//!
//! `SectorMap` is an immutable snapshot of the static geography loaded at node
//! startup. All fields are `pub(super)` so every `node::*` submodule can
//! access them directly without an extra indirection layer.

use std::collections::HashMap;
use std::sync::Arc;

use dawn_core::{
    CelestialBodyDef, CelestialBodyId, JumpGateDef, JumpGateId, StationDef, StationId,
};

use crate::galaxy::Galaxy;

/// Static navigation topology for this node's Sector.
///
/// Loaded once at startup from `Galaxy`; never mutated during a run.
pub(super) struct SectorMap {
    /// Full star-map reference (used for cross-sector topology queries).
    pub(super) galaxy: Arc<Galaxy>,
    /// Jump Gates whose `from_sector` is this node's Sector (ADR-0009).
    pub(super) gates: HashMap<JumpGateId, JumpGateDef>,
    /// Celestial bodies (stars, planets) in this node's Sector (ADR-0025).
    pub(super) bodies: HashMap<CelestialBodyId, CelestialBodyDef>,
    /// NPC stations in this node's Sector (ADR-0034 9B foundation).
    pub(super) stations: HashMap<StationId, StationDef>,
}
