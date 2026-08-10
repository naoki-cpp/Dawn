//! Explicit state owners for one Sector engine.
//!
//! These owners are intentionally crate-private. They are not new aggregate
//! APIs; they make mutation authority visible inside the existing
//! `SimulationNode` composition root while domain operations use the owner
//! that is responsible for their state.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use dawn_core::{
    DomainEvent, JumpGateId, LockOnCommand, ModuleDefinition, ModuleId, NodeId, PlayerId, Position,
    ResumeTicket, ShipId, ShipTypeDefinition, ShipTypeId, StationId, Tick,
};
use dawn_ecs::{components::ShipStatsComp, SimWorld};

use super::repositories::{
    AdmissionRepository, IdentityRepository, SectorRepository, SectorTransaction,
    StationInventoryRepository,
};
use super::sector_map::SectorMap;
use super::ship_registry::ShipRegistry;
use super::TransitJournal;

/// ECS and authoritative simulation counters.
pub(super) struct SimulationState {
    pub(super) world: SimWorld,
    pub(super) current_tick: Tick,
    pub(super) id_counter: u64,
    pub(super) ships: ShipRegistry,
    pub(super) base_stats: HashMap<ShipId, ShipStatsComp>,
    pub(super) pending_bot_lock_commands: Vec<LockOnCommand>,
    /// Market settlement identities already applied to authoritative cargo.
    /// The set is checkpointed so a lost ACK cannot make a retry duplicate an
    /// inventory mutation after restart.
    pub(super) applied_market_settlements: HashSet<u64>,
}

/// Player ownership, active-ship admission, and allocation state.
pub(super) struct PlayerState {
    pub(super) player_id_counter: u64,
    pub(super) active_ship: HashMap<PlayerId, ShipId>,
    pub(super) owners: HashMap<ShipId, PlayerId>,
    pub(super) pending_fresh_admissions: HashSet<ShipId>,
    pub(super) pending_resume_admissions: HashMap<ShipId, PlayerId>,
    pub(super) population_cap: usize,
}

/// Station-domain projection and authoritative docking context.
pub(super) struct StationState {
    docked_ships: BTreeMap<ShipId, StationId>,
    docked_players: BTreeMap<PlayerId, StationId>,
}

impl StationState {
    pub(super) fn empty() -> Self {
        Self {
            docked_ships: BTreeMap::new(),
            docked_players: BTreeMap::new(),
        }
    }

    pub(super) fn docked_station_for_ship(&self, ship_id: ShipId) -> Option<StationId> {
        self.docked_ships.get(&ship_id).copied()
    }

    pub(super) fn docked_station_for_player(&self, player_id: PlayerId) -> Option<StationId> {
        self.docked_players.get(&player_id).copied()
    }

    pub(super) fn is_ship_docked(&self, ship_id: ShipId) -> bool {
        self.docked_ships.contains_key(&ship_id)
    }

    pub(super) fn dock_ship(&mut self, ship_id: ShipId, station_id: StationId) {
        self.docked_ships.insert(ship_id, station_id);
    }

    pub(super) fn undock_ship(&mut self, ship_id: ShipId) {
        self.docked_ships.remove(&ship_id);
    }

    pub(super) fn dock_player(&mut self, player_id: PlayerId, station_id: StationId) {
        self.docked_players.insert(player_id, station_id);
    }

    pub(super) fn undock_player(&mut self, player_id: PlayerId) {
        self.docked_players.remove(&player_id);
    }

    pub(super) fn docked_player_ids(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.docked_players.keys().copied()
    }

    pub(super) fn snapshot_docked_ships(&self) -> BTreeMap<ShipId, StationId> {
        self.docked_ships.clone()
    }

    pub(super) fn snapshot_docked_players(&self) -> BTreeMap<PlayerId, StationId> {
        self.docked_players.clone()
    }

    pub(super) fn restore(
        &mut self,
        docked_ships: BTreeMap<ShipId, StationId>,
        docked_players: BTreeMap<PlayerId, StationId>,
    ) {
        self.docked_ships = docked_ships;
        self.docked_players = docked_players;
    }
}

/// Composition-wired persistence boundary.
///
/// SQL is deliberately not part of the Station state owner. Station and
/// admission code may use the narrow repository views exposed here, while the
/// composition root remains the only place that wires a concrete adapter.
pub(super) struct PersistenceBoundary {
    repositories: SectorRepository,
}

impl PersistenceBoundary {
    pub(super) fn in_memory() -> Self {
        Self {
            repositories: SectorRepository::open_in_memory()
                .expect("in-memory repository never fails to open"),
        }
    }

    pub(super) fn open(path: &str) -> Result<Self, String> {
        Ok(Self {
            repositories: SectorRepository::open(path).map_err(|error| error.to_string())?,
        })
    }

    pub(super) fn admissions(&self) -> AdmissionRepository<'_> {
        self.repositories.admissions()
    }

    pub(super) fn identities(&self) -> IdentityRepository<'_> {
        self.repositories.identities()
    }

    pub(super) fn station_inventory(&self) -> StationInventoryRepository<'_> {
        self.repositories.station_inventory()
    }

    pub(super) fn transaction(&mut self) -> rusqlite::Result<SectorTransaction<'_>> {
        self.repositories.transaction()
    }

    pub(super) fn observe_materialized_identities(
        &self,
        ship_ids: impl IntoIterator<Item = ShipId>,
        player_ids: impl IntoIterator<Item = PlayerId>,
    ) -> rusqlite::Result<()> {
        self.repositories
            .observe_materialized_identities(ship_ids, player_ids)
    }

    pub(super) fn reconcile_admission_identity_watermarks(&self) -> rusqlite::Result<()> {
        self.repositories.reconcile_admission_identity_watermarks()
    }

    pub(super) fn record_client_ownership_with_pending(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        pending_resume_ticket: Option<ResumeTicket>,
    ) -> rusqlite::Result<()> {
        self.repositories.record_client_ownership_with_pending(
            ship_id,
            player_id,
            resume_ticket,
            pending_resume_ticket,
        )
    }

    pub(super) fn reserve_fresh_admission_identity(
        &self,
        node_id: NodeId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<(PlayerId, ShipId)> {
        self.repositories
            .reserve_fresh_admission_identity(node_id, spawn_position, resume_ticket)
    }

    pub(super) fn record_client_ownership(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<()> {
        self.repositories
            .record_client_ownership(ship_id, player_id, resume_ticket)
    }

    pub(super) fn stage_client_resume_ticket(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
        presented_ticket: ResumeTicket,
        proposed_next_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<ResumeTicket>> {
        self.repositories.stage_client_resume_ticket(
            ship_id,
            player_id,
            presented_ticket,
            proposed_next_ticket,
        )
    }
}

/// Durable Transit attempt state and its source-local allocator.
pub(super) struct TransitState {
    pub(super) transit_attempt_counter: u64,
    pub(super) transit_journal: TransitJournal,
}

/// Outputs that live only until the runtime drains the current frame.
pub(super) struct FrameOutputs {
    pub(super) pending_events: Vec<DomainEvent>,
    pub(super) pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    pub(super) completed_warps: Vec<ShipId>,
}

/// Immutable navigation and coordinate dependencies for this Sector.
pub(super) struct SectorTopology {
    pub(super) sector_map: SectorMap,
    pub(super) anchor_table: crate::anchor::AnchorTable,
}

/// Validated immutable game-data dependency and its content identity.
pub(super) struct GameData {
    pub(super) module_registry: Arc<std::collections::BTreeMap<ModuleId, ModuleDefinition>>,
    pub(super) ship_type_registry: Arc<std::collections::BTreeMap<ShipTypeId, ShipTypeDefinition>>,
    pub(super) catalog_fingerprint: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_state_can_be_constructed_without_a_repository() {
        let station = StationState::empty();

        assert!(station.docked_ships.is_empty());
        assert!(station.docked_players.is_empty());
    }

    #[test]
    fn persistence_boundary_exposes_typed_projection_views() {
        let persistence = PersistenceBoundary::in_memory();

        assert_eq!(
            persistence
                .station_inventory()
                .projection_applied_through()
                .expect("in-memory projection cursor"),
            0
        );
    }
}
