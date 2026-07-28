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
    fitting::FittingSnapshot, AbsolutePosition, NodeId, Position, SectorBounds, SectorId, ShipId,
    ShipTypeId, Tick, Velocity,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Ship-level snapshot ───────────────────────────────────────────────────────

/// State of a single Ship at the time of the snapshot.
///
/// Captures everything needed to reconstruct the Ship's ECS components
/// (`PositionComp`, `VelocityComp`, `ShipStatsComp`, `HullComp`,
/// `CapacitorComp`, `FittingComp`) without replaying events from the
/// beginning of the log (INV-002).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipSnapshot {
    pub ship_id: ShipId,
    pub ship_type_id: ShipTypeId,
    /// Authoritative Sector-frame position (ADR-0044). `None` for a
    /// transient/in-memory state that has not acquired a Sector-frame
    /// projection yet.
    pub absolute_position: Option<AbsolutePosition>,
    /// Anchor-relative offset retained as the local simulation representation.
    pub position: Position,
    /// Coordinate anchor the `position` offset is relative to (ADR-0029).
    pub anchor: dawn_core::AnchorId,
    pub velocity: Velocity,
    /// `HullComp` at the time of the snapshot (Shield / Armor / Hull layers).
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    /// `CapacitorComp.current`, if the ship has a capacitor.
    pub capacitor: Option<f32>,
    /// Fitted modules (High/Mid/Low/Rig) and their on/off state.
    pub fitting: FittingSnapshot,
    /// Ships currently tackling this ship (ADR-0024). Persisted so tackle
    /// state is not lost on restart (which would allow escape).
    pub tackled_by: Vec<dawn_core::ShipId>,
    /// Unfitted / unassembled items the pilot owns (ADR-0034).
    pub inventory: std::collections::BTreeMap<dawn_core::ItemId, u64>,
}

// ── Node-level snapshot ───────────────────────────────────────────────────────

/// Complete state of a `SimulationNode` at a specific `log_index`.
///
/// Stores enough information to reconstruct the ECS World without replaying
/// events from the beginning of time.
///
/// # Format compatibility (ADR-0017)
///
/// The on-disk format is **version-locked to the binary**: postcard is not
/// self-describing, so fields are read positionally and a snapshot written by
/// a different field list fails to load with `DeserializeUnexpectedEnd`.
/// `#[serde(default)]` does **not** grant field-level back-compat here the way
/// it would for a self-describing format like JSON — it is a no-op on this
/// path, so do not add it to imply a compatibility guarantee that does not
/// exist. Changing this struct means operators regenerate snapshots.
///
/// Which fields belong here is enforced from both sides:
/// `SimulationNode::take_snapshot` destructures the node exhaustively, and
/// `SimulationNode::apply_snapshot` destructures this struct exhaustively, so
/// adding a field to either is a compile error until it is handled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The node that produced this snapshot.
    pub node_id: NodeId,
    /// The Sector this node manages.
    pub sector_id: SectorId,
    /// Spatial bounds of the Sector.
    pub bounds: SectorBounds,
    /// All events with index < `log_index` are covered by this snapshot.
    /// Events at `log_index` and beyond must be replayed.
    pub log_index: u64,
    /// Logical tick at the time of the snapshot.
    pub tick: Tick,
    /// Next value for `SimulationNode::id_counter`.
    /// Must be restored to prevent EntityId reuse (INV-004).
    pub id_counter: u64,
    /// Next value for `SimulationNode::player_id_counter`.
    ///
    /// Must be restored for the same reason as `id_counter`: ownership is not
    /// carried in the snapshot (a returning client re-asserts it via
    /// `adopt_player_ship`, ADR-0007 §2-A resume), so a counter that restarted
    /// at zero would hand a freshly-admitted client a `PlayerId` that a
    /// restored, still-owned ship already belongs to.
    pub player_id_counter: u64,
    /// State of every Ship in the Sector at the snapshot instant.
    pub ships: Vec<ShipSnapshot>,
    /// Current docked station per ship.
    pub docked_ships: BTreeMap<dawn_core::ShipId, dawn_core::StationId>,
    /// Current docked station context per player.
    pub docked_players: BTreeMap<dawn_core::PlayerId, dawn_core::StationId>,
}

impl StateSnapshot {
    /// Serialise with `postcard` and write to `path`.
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let bytes = postcard::to_stdvec(self).map_err(|e| io::Error::other(e.to_string()))?;
        fs::write(path, bytes)
    }

    /// Read from `path` and deserialise.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        postcard::from_bytes(&bytes).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("current snapshot: {e}"))
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Position;

    fn sample_snapshot() -> StateSnapshot {
        StateSnapshot {
            node_id: NodeId(0),
            sector_id: dawn_core::SectorId(0),
            bounds: SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            log_index: 42,
            tick: Tick(10),
            id_counter: 5,
            player_id_counter: 3,
            ships: vec![ShipSnapshot {
                ship_id: ShipId::new(NodeId(0), 0),
                ship_type_id: ShipTypeId(1),
                absolute_position: Some(AbsolutePosition::new(100.0, 200.0, 300.0)),
                position: Position::new(100.0, 200.0, 300.0),
                anchor: dawn_core::AnchorId(0),
                velocity: dawn_core::Velocity::new(1.0, 0.0, 0.0),
                current_shield: 50.0,
                current_armor: 60.0,
                current_hull: 70.0,
                is_destroyed: false,
                capacitor: Some(250.0),
                fitting: FittingSnapshot::empty(),
                tackled_by: vec![],
                inventory: std::collections::BTreeMap::from([(
                    dawn_core::ItemId::Module(dawn_core::ModuleId(7)),
                    1,
                )]),
            }],
            docked_ships: BTreeMap::from([(ShipId::new(NodeId(0), 0), dawn_core::StationId(0))]),
            docked_players: BTreeMap::from([(dawn_core::PlayerId(9), dawn_core::StationId(0))]),
        }
    }

    /// Re-encoding what postcard decoded must reproduce the original bytes.
    ///
    /// Asserted on the encoded form rather than field by field on purpose: a
    /// field-by-field list is itself hand-maintained, so it goes stale in
    /// exactly the way this whole seam exists to prevent. `sample_snapshot`
    /// gives every field a non-default value, and its struct literal stops
    /// compiling when `StateSnapshot` grows one.
    #[test]
    fn snapshot_round_trips_through_postcard_without_data_loss() {
        let original = sample_snapshot();
        let bytes = postcard::to_stdvec(&original).unwrap();
        let restored: StateSnapshot = postcard::from_bytes(&bytes).unwrap();

        assert_eq!(postcard::to_stdvec(&restored).unwrap(), bytes);
    }

    /// postcard is not self-describing: struct fields are read positionally,
    /// so a buffer written from a different field list cannot be decoded and
    /// `#[serde(default)]` does not rescue it. This pins the behaviour the
    /// format-compatibility note on `StateSnapshot` depends on — if it ever
    /// stops holding, that note (and ADR-0017) needs revisiting.
    #[test]
    fn a_snapshot_written_with_fewer_fields_fails_to_load() {
        #[derive(Serialize)]
        struct Truncated {
            node_id: NodeId,
        }

        let bytes = postcard::to_stdvec(&Truncated { node_id: NodeId(0) }).unwrap();
        assert!(postcard::from_bytes::<StateSnapshot>(&bytes).is_err());
    }

    #[test]
    fn snapshot_survives_save_and_load_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let original = sample_snapshot();
        original.save(&path).unwrap();

        let restored = StateSnapshot::load(&path).unwrap();
        assert_eq!(restored.log_index, original.log_index);
        assert_eq!(restored.tick, original.tick);
        assert_eq!(restored.id_counter, original.id_counter);
    }
}
