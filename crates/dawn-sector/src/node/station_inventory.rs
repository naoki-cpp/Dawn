//! Station authority and projection helpers for `SimulationNode`.
//!
//! `StationState` is the Sector authority. The repository is the bounded
//! SQLite read-model boundary and is advanced only by a committed recovery
//! transition. The engine intentionally keeps no interior-mutability cache:
//! a read must not mutate state through `&self`.

use std::collections::BTreeMap;

use dawn_core::{events::ClientAdmissionCommitted, ItemId, PlayerId, StationId};

use super::{
    repositories::ProjectionReadError, station::StationOperationRejection, SimulationNode,
};

impl SimulationNode {
    /// Read the player's Station projection at one station.
    pub(crate) fn station_inventory(
        &self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> Result<Option<BTreeMap<ItemId, u64>>, ProjectionReadError> {
        let mut inventory = self
            .persistence
            .station_inventory()
            .get_all(player_id, station_id)?;
        for (item_id, count) in self.stations.overlay_inventory(player_id, station_id) {
            if count == 0 {
                inventory.remove(&item_id);
            } else {
                inventory.insert(item_id, count);
            }
        }
        Ok((!inventory.is_empty()).then_some(inventory))
    }

    /// Count one item stack inside the player's Station projection.
    pub(crate) fn station_item_count(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
    ) -> Result<u64, ProjectionReadError> {
        self.station_inventory(player_id, station_id)
            .map(|inventory| {
                inventory
                    .and_then(|inventory| inventory.get(&item_id).copied())
                    .unwrap_or(0)
            })
    }

    fn ensure_client_admission_grant(
        &mut self,
        event: &ClientAdmissionCommitted,
    ) -> Result<(), String> {
        self.persistence
            .transaction()
            .map_err(|error| error.to_string())?
            .ensure_client_admission_grant(
                event.ship_id,
                event.player_id,
                event.resume_ticket,
                event.starter_station_id,
                event.starter_item_id,
                event.starter_item_count,
            )
            .map_err(|error| error.to_string())?;
        self.players.pending_fresh_admissions.remove(&event.ship_id);
        Ok(())
    }

    /// Finalize repository-owned admission protocol state only after the
    /// transition carrying the materialized Ship and starter Station grant is
    /// durable and its Station projection has applied.
    pub(crate) fn reconcile_committed_admission_events(
        &mut self,
        events: &[dawn_core::DomainEvent],
    ) -> Result<(), String> {
        for event in events {
            if let dawn_core::DomainEvent::ClientAdmissionCommitted(admission) = event {
                self.ensure_client_admission_grant(admission)?;
            }
        }
        Ok(())
    }

    pub(super) fn reconcile_client_admission_identities(&mut self) -> Result<(), String> {
        self.persistence
            .reconcile_admission_identity_watermarks()
            .map_err(|error| error.to_string())
    }

    /// Apply an authoritative Station credit during command preparation.
    ///
    /// This changes only the Sector authority and records a pending mutation.
    /// The pending mutation is carried by the next RecoveryDelta; SQLite is
    /// updated only after that delta is durable and live-applied.
    pub(crate) fn credit_station_item(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), ProjectionReadError> {
        let current = self.station_item_count(player_id, station_id, item_id)?;
        self.credit_station_item_from_current(player_id, station_id, item_id, current, count);
        Ok(())
    }

    pub(super) fn credit_station_item_from_current(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        current: u64,
        count: u64,
    ) {
        self.stations
            .credit(player_id, station_id, item_id, current, count);
    }

    pub(super) fn try_debit_station_item_from_current(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
        current: u64,
    ) -> Result<(), StationOperationRejection> {
        self.stations
            .try_debit(player_id, station_id, item_id, count, current)
    }

    pub(super) fn try_debit_station_item(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), StationOperationRejection> {
        let current = self
            .station_item_count(player_id, station_id, item_id)
            .map_err(StationOperationRejection::projection_read)?;
        self.try_debit_station_item_from_current(player_id, station_id, item_id, count, current)
    }

    pub(crate) fn apply_station_projection(
        &self,
        transition_id: &str,
        range: dawn_storage::JournalRange,
        mutations: &[crate::transition::StationProjectionMutation],
    ) -> Result<super::repositories::ProjectionApplyResult, super::repositories::ProjectionApplyError>
    {
        self.persistence
            .station_inventory()
            .apply_projection(transition_id, range, mutations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds, SectorId};

    const TEST_STATION_ID: StationId = StationId(0);

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn station_inventory_tracks_items_per_station() {
        let mut node = node();
        let player_a = PlayerId(1);
        let player_b = PlayerId(2);

        node.credit_station_item(player_a, TEST_STATION_ID, ItemId::ScrapMetal, 3)
            .unwrap();
        node.credit_station_item(player_a, TEST_STATION_ID, ItemId::ScrapMetal, 2)
            .unwrap();
        node.credit_station_item(
            player_b,
            TEST_STATION_ID,
            ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            1,
        )
        .unwrap();

        assert_eq!(
            node.station_item_count(player_a, TEST_STATION_ID, ItemId::ScrapMetal)
                .unwrap(),
            5
        );
        assert_eq!(
            node.station_item_count(
                player_b,
                TEST_STATION_ID,
                ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn station_inventory_is_isolated_per_station_for_the_same_player() {
        let mut node = node();
        let player_id = PlayerId(1);

        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 3)
            .unwrap();
        node.credit_station_item(player_id, StationId(1), ItemId::ScrapMetal, 7)
            .unwrap();

        assert_eq!(
            node.station_item_count(player_id, StationId(0), ItemId::ScrapMetal)
                .unwrap(),
            3
        );
        assert_eq!(
            node.station_item_count(player_id, StationId(1), ItemId::ScrapMetal)
                .unwrap(),
            7
        );
    }

    #[test]
    fn station_projection_reads_merge_only_the_frame_local_overlay() {
        let mut node = node();
        node.credit_station_item(PlayerId(1), TEST_STATION_ID, ItemId::ScrapMetal, 2)
            .unwrap();

        assert_eq!(
            node.station_inventory(PlayerId(1), TEST_STATION_ID)
                .unwrap()
                .unwrap()
                .get(&ItemId::ScrapMetal),
            Some(&2)
        );
    }

    #[test]
    fn station_projection_read_failure_is_not_converted_to_an_empty_inventory() {
        let mut node = node();
        node.persistence.drop_station_inventory_for_test();

        assert!(matches!(
            node.station_inventory(PlayerId(1), TEST_STATION_ID),
            Err(ProjectionReadError::Storage { .. })
        ));
        assert!(matches!(
            node.station_item_count(PlayerId(1), TEST_STATION_ID, ItemId::ScrapMetal),
            Err(ProjectionReadError::Storage { .. })
        ));
    }

    #[test]
    fn station_mutation_reaches_sqlite_only_after_the_durable_tick() {
        let mut node = node();
        node.credit_station_item(PlayerId(1), TEST_STATION_ID, ItemId::ScrapMetal, 2)
            .unwrap();
        assert!(node
            .persistence
            .station_inventory()
            .get_all(PlayerId(1), TEST_STATION_ID)
            .unwrap()
            .is_empty());

        let mut journal = dawn_storage::InMemoryJournal::new();
        crate::transit::commit_tick_state_transition(
            &mut node,
            &mut journal,
            crate::transition::FrameInput::lock_only(&[]),
            crate::transition::SectorTransitionId(1),
            0,
            dawn_storage::DurabilityMode::Synced,
        )
        .expect("durable Tick should apply the required Station projection");

        assert_eq!(
            node.persistence
                .station_inventory()
                .get_all(PlayerId(1), TEST_STATION_ID)
                .unwrap()
                .get(&ItemId::ScrapMetal),
            Some(&2)
        );
    }

    #[test]
    fn try_debit_station_item_rejects_missing_or_insufficient_stacks() {
        let mut node = node();
        let player_id = PlayerId(1);
        node.credit_station_item(player_id, TEST_STATION_ID, ItemId::ScrapMetal, 2)
            .unwrap();

        assert!(matches!(
            node.try_debit_station_item(
                player_id,
                TEST_STATION_ID,
                ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
                1
            ),
            Err(StationOperationRejection::MissingStationItem)
        ));
        assert!(matches!(
            node.try_debit_station_item(player_id, TEST_STATION_ID, ItemId::ScrapMetal, 3),
            Err(StationOperationRejection::InsufficientStationItem)
        ));
        assert!(node
            .try_debit_station_item(player_id, TEST_STATION_ID, ItemId::ScrapMetal, 2)
            .is_ok());
        assert_eq!(
            node.station_item_count(player_id, TEST_STATION_ID, ItemId::ScrapMetal)
                .unwrap(),
            0
        );
    }
}
