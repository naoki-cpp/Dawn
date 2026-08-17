//! Station projection helpers for `SimulationNode`.
//!
//! The repository is the bounded read/write boundary. The engine intentionally
//! keeps no interior-mutability cache: a read must not mutate state through
//! `&self`. A future runtime may add a cache beside the repository if it owns
//! invalidation and promotion freshness explicitly.

use std::collections::BTreeMap;

use dawn_core::{events::ClientAdmissionCommitted, ItemId, PlayerId, StationId};

use super::{station::StationOperationRejection, SimulationNode};

impl SimulationNode {
    /// Read the player's Station projection at one station.
    pub(crate) fn station_inventory(
        &self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> Option<BTreeMap<ItemId, u64>> {
        let inventory = self
            .persistence
            .station_inventory()
            .get_all(player_id, station_id);
        (!inventory.is_empty()).then_some(inventory)
    }

    /// Count one item stack inside the player's Station projection.
    pub(crate) fn station_item_count(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
    ) -> u64 {
        self.station_inventory(player_id, station_id)
            .and_then(|inventory| inventory.get(&item_id).copied())
            .unwrap_or(0)
    }

    pub(super) fn ensure_client_admission_grant(&mut self, event: &ClientAdmissionCommitted) {
        self.persistence
            .transaction()
            .expect("client admission Station grant transaction")
            .ensure_client_admission_grant(
                event.ship_id,
                event.player_id,
                event.resume_ticket,
                event.starter_station_id,
                event.starter_item_id,
                event.starter_item_count,
            )
            .expect("client admission Station grant transaction");
    }

    pub(super) fn reconcile_client_admission_identities(&mut self) -> Result<(), String> {
        self.persistence
            .reconcile_admission_identity_watermarks()
            .map_err(|error| error.to_string())
    }

    /// Transitional command adapter. Authoritative Station mutations should
    /// use `StationInventoryRepository::apply_projection` after the enclosing
    /// RecoveryDelta is durable; this method remains for existing command
    /// seams until the runtime projection hook is wired by #278.
    pub(crate) fn credit_station_item(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) {
        if count == 0 {
            return;
        }
        self.persistence
            .station_inventory()
            .credit(player_id, station_id, item_id, count);
    }

    pub(super) fn try_debit_station_item(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), StationOperationRejection> {
        if count == 0 {
            return Ok(());
        }
        self.persistence
            .station_inventory()
            .try_debit(player_id, station_id, item_id, count)
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

        node.credit_station_item(player_a, TEST_STATION_ID, ItemId::ScrapMetal, 3);
        node.credit_station_item(player_a, TEST_STATION_ID, ItemId::ScrapMetal, 2);
        node.credit_station_item(
            player_b,
            TEST_STATION_ID,
            ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            1,
        );

        assert_eq!(
            node.station_item_count(player_a, TEST_STATION_ID, ItemId::ScrapMetal),
            5
        );
        assert_eq!(
            node.station_item_count(
                player_b,
                TEST_STATION_ID,
                ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
            ),
            1
        );
    }

    #[test]
    fn station_inventory_is_isolated_per_station_for_the_same_player() {
        let mut node = node();
        let player_id = PlayerId(1);

        node.credit_station_item(player_id, StationId(0), ItemId::ScrapMetal, 3);
        node.credit_station_item(player_id, StationId(1), ItemId::ScrapMetal, 7);

        assert_eq!(
            node.station_item_count(player_id, StationId(0), ItemId::ScrapMetal),
            3
        );
        assert_eq!(
            node.station_item_count(player_id, StationId(1), ItemId::ScrapMetal),
            7
        );
    }

    #[test]
    fn station_projection_reads_do_not_require_an_in_memory_cache() {
        let mut node = node();
        node.credit_station_item(PlayerId(1), TEST_STATION_ID, ItemId::ScrapMetal, 2);

        assert_eq!(
            node.station_inventory(PlayerId(1), TEST_STATION_ID)
                .unwrap()
                .get(&ItemId::ScrapMetal),
            Some(&2)
        );
    }

    #[test]
    fn try_debit_station_item_rejects_missing_or_insufficient_stacks() {
        let mut node = node();
        let player_id = PlayerId(1);
        node.credit_station_item(player_id, TEST_STATION_ID, ItemId::ScrapMetal, 2);

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
            node.station_item_count(player_id, TEST_STATION_ID, ItemId::ScrapMetal),
            0
        );
    }
}
