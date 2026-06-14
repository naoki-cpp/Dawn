//! `StateSnapshot` — point-in-time capture of a `SimulationNode`'s ECS state.
//!
//! # Recovery procedure (INV-002)
//!
//! 1. Load `StateSnapshot` from disk.
//! 2. Open the `FileEventStore` for the same Sector.
//! 3. Call `SimulationNode::restore_from(store, &snapshot, &modules, &ship_types)`
//!    with the same module / ship-type definitions the node was configured with.
//!    - The snapshot reconstructs the ECS World up to `log_index`
//!      (position, velocity, hull layers, capacitor, fitting).
//!    - Events at `log_index` and beyond are replayed on top.
//! 4. The restored node is equivalent to the node at shutdown.
//!
//! # Snapshot is the authoritative durable checkpoint (ADR-0017 / INV-002)
//!
//! The Event Log stays append-only (INV-001) and is the history / propagation /
//! snapshot-source. But the snapshot — not genesis replay — is what operational
//! recovery and failover (ADR-0014) rely on: load the latest snapshot, then
//! catch up the tail of events.
//!
//! Derived / transient state (position, capacitor, lock countdowns) is persisted
//! in the snapshot. It is a per-tick pure function (position = velocity integral,
//! cap = recharge) and is NOT event-sourced, so it cannot be rebuilt from events
//! alone — it is restored from the snapshot and recomputed as the sim runs forward.
//!
//! Genesis (index 0) reconstruction is off-path (audit / disaster only): apply
//! events to rebuild authoritative state, then let transient state recompute. No
//! operational path depends on it.

use std::{fs, io, path::Path};

use dawn_core::{
    fitting::FittingSnapshot, NodeId, Position, SectorBounds, SectorId, ShipId, ShipTypeId,
    Tick, Velocity,
};
use serde::{Deserialize, Serialize};

// ── Ship-level snapshot ───────────────────────────────────────────────────────

/// State of a single Ship at the time of the snapshot.
///
/// Captures everything needed to reconstruct the Ship's ECS components
/// (`PositionComp`, `VelocityComp`, `ShipStatsComp`, `HullComp`,
/// `CapacitorComp`, `FittingComp`) without replaying events from the
/// beginning of the log (INV-002).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipSnapshot {
    pub ship_id     : ShipId,
    pub ship_type_id: ShipTypeId,
    pub position    : Position,
    pub velocity    : Velocity,
    /// `HullComp` at the time of the snapshot (Shield / Armor / Hull layers).
    pub current_shield: f32,
    pub current_armor : f32,
    pub current_hull  : f32,
    pub is_destroyed  : bool,
    /// `CapacitorComp.current`, if the ship has a capacitor.
    pub capacitor   : Option<f32>,
    /// Fitted modules (High/Mid/Low/Rig) and their on/off state.
    pub fitting     : FittingSnapshot,
}

// ── Node-level snapshot ───────────────────────────────────────────────────────

/// Complete state of a `SimulationNode` at a specific `log_index`.
///
/// Stores enough information to reconstruct the ECS World without replaying
/// events from the beginning of time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The node that produced this snapshot.
    pub node_id    : NodeId,
    /// The Sector this node manages.
    pub sector_id  : SectorId,
    /// Spatial bounds of the Sector.
    pub bounds     : SectorBounds,
    /// All events with index < `log_index` are covered by this snapshot.
    /// Events at `log_index` and beyond must be replayed.
    pub log_index  : u64,
    /// Logical tick at the time of the snapshot.
    pub tick       : Tick,
    /// Next value for `SimulationNode::id_counter`.
    /// Must be restored to prevent EntityId reuse (INV-004).
    pub id_counter : u64,
    /// State of every Ship in the Sector at the snapshot instant.
    pub ships      : Vec<ShipSnapshot>,
}

impl StateSnapshot {
    /// Serialise with `postcard` and write to `path`.
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let bytes = postcard::to_stdvec(self)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        fs::write(path, bytes)
    }

    /// Read from `path` and deserialise.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        postcard::from_bytes(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Position;

    fn sample_snapshot() -> StateSnapshot {
        StateSnapshot {
            node_id   : NodeId(0),
            sector_id : dawn_core::SectorId(0),
            bounds    : SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            log_index : 42,
            tick      : Tick(10),
            id_counter: 5,
            ships     : vec![
                ShipSnapshot {
                    ship_id       : ShipId::new(NodeId(0), 0),
                    ship_type_id  : ShipTypeId(1),
                    position      : Position::new(100.0, 200.0, 300.0),
                    velocity      : dawn_core::Velocity::new(1.0, 0.0, 0.0),
                    current_shield: 50.0,
                    current_armor : 60.0,
                    current_hull  : 70.0,
                    is_destroyed  : false,
                    capacitor     : Some(250.0),
                    fitting       : FittingSnapshot::empty(),
                },
            ],
        }
    }

    #[test]
    fn snapshot_round_trips_through_postcard_without_data_loss() {
        let original = sample_snapshot();
        let bytes    = postcard::to_stdvec(&original).unwrap();
        let restored: StateSnapshot = postcard::from_bytes(&bytes).unwrap();

        assert_eq!(restored.log_index,  original.log_index);
        assert_eq!(restored.tick,       original.tick);
        assert_eq!(restored.id_counter, original.id_counter);
        assert_eq!(restored.ships.len(), 1);
        assert_eq!(restored.ships[0].position, original.ships[0].position);
    }

    #[test]
    fn snapshot_survives_save_and_load_from_disk() {
        let dir  = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let original = sample_snapshot();
        original.save(&path).unwrap();

        let restored = StateSnapshot::load(&path).unwrap();
        assert_eq!(restored.log_index,  original.log_index);
        assert_eq!(restored.tick,       original.tick);
        assert_eq!(restored.id_counter, original.id_counter);
    }
}
