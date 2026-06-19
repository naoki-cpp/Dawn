use dawn_ecs::components::{CapacitorComp, FittingComp, HullComp, PositionComp, TackledComp, VelocityComp};
use dawn_event_store::store::EventStore;

use crate::persistence::{ShipSnapshot, StateSnapshot};

use super::SimulationNode;

impl<S: EventStore> SimulationNode<S> {
    /// Capture the current ECS state as a `StateSnapshot`.
    ///
    /// The snapshot covers all events with `log_index < event_store.len()`.
    /// Pair with the event log to reconstruct this exact state on restart.
    pub fn take_snapshot(&self) -> StateSnapshot {
        let mut ships: Vec<ShipSnapshot> = self
            .ships.index
            .iter()
            .filter_map(|(&ship_id, &entity)| {
                let pos  = self.world.inner().get::<&PositionComp>(entity).ok()?.0;
                let vel  = self.world.inner().get::<&VelocityComp>(entity).ok()?.0;
                let hull = self.world.inner().get::<&HullComp>(entity).ok()?;
                let capacitor = self.world.inner().get::<&CapacitorComp>(entity).ok().map(|c| c.current);
                let fitting = self.world.inner().get::<&FittingComp>(entity)
                    .map(|f| f.to_snapshot())
                    .unwrap_or_else(|_| dawn_core::fitting::FittingSnapshot::empty());
                let tackled_by = self.world.inner().get::<&TackledComp>(entity)
                    .map(|t| t.tacklers.clone())
                    .unwrap_or_default();
                let ship_type_id = self.ships.type_ids.get(&ship_id).copied().unwrap_or(dawn_core::ShipTypeId(0));
                Some(ShipSnapshot {
                    ship_id,
                    ship_type_id,
                    position      : pos,
                    velocity      : vel,
                    current_shield: hull.current_shield,
                    current_armor : hull.current_armor,
                    current_hull  : hull.current_hull,
                    is_destroyed  : hull.is_destroyed,
                    capacitor,
                    fitting,
                    tackled_by,
                })
            })
            .collect();

        // Canonical ordering: a snapshot of a given state must serialise
        // identically regardless of HashMap iteration order, so it can be
        // byte-compared (INV-002: verifiable snapshot / round-trip).
        // `ShipId: Ord` is canonical id order (node_id then counter).
        ships.sort_by_key(|s| s.ship_id);

        StateSnapshot {
            node_id   : self.node_id,
            sector_id : self.sector_id,
            bounds    : self.bounds,
            log_index : self.event_store.len() as u64,
            tick      : self.current_tick,
            id_counter: self.id_counter,
            ships,
        }
    }
}

// ── Checkpointing (ADR-0017 8A-7) ─────────────────────────────────────────────

impl SimulationNode<dawn_event_store::FileEventStore> {
    /// Take a snapshot, persist it durably, then compact the hot log behind it.
    ///
    /// This is the operational checkpoint of ADR-0017: after it returns, recovery
    /// only needs `snapshot_path` + the post-snapshot tail of the hot log; the
    /// prefix it covers lives in the append-only cold archive at `cold_path`.
    ///
    /// Ordering is load-bearing for crash safety: the snapshot is saved **before**
    /// the hot log is compacted. A crash between the two leaves the snapshot
    /// written and the hot log untouched (a redundant but safe state). Compacting
    /// first could strand a snapshot whose `log_index` is older than the new
    /// `base_index`, which would make `iter_from` silently skip events.
    pub fn checkpoint(
        &mut self,
        snapshot_path: impl AsRef<std::path::Path>,
        cold_path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<StateSnapshot> {
        let snapshot = self.take_snapshot();
        snapshot.save(&snapshot_path)?;
        self.event_store.compact(snapshot.log_index, cold_path)?;
        Ok(snapshot)
    }
}
