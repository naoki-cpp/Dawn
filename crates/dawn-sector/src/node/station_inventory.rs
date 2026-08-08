//! Current Station inventory cache + SQLite write-through seam.
//!
//! This is the ADR-0038 implementation baseline: `station_inventory_db.rs`
//! currently owns the durable SQLite representation, while this module owns the
//! bounded cache and the `SimulationNode` helpers. ADR-0049 changes the target
//! authority split: the Sector recovery journal owns the Station aggregate and
//! SQLite becomes an idempotent projection/read model under #277. Do not infer
//! the final recovery ordering from this legacy write-through path.

use std::collections::{BTreeMap, VecDeque};

use dawn_core::{events::ClientAdmissionCommitted, ItemId, PlayerId, StationId};

use super::{station::StationOperationRejection, SimulationNode};

/// Bounded in-memory cache of recently-touched players' Station inventory.
///
/// In the current implementation SQLite is the write-through authority for
/// this cache, so eviction only causes the next read to query SQLite again.
/// Under ADR-0049/#277, the same cache is a read optimization over the
/// journal-owned Station aggregate and SQLite projection; that target contract
/// is not implemented by this module yet.
pub(super) struct StationInventoryCache {
    entries: std::collections::HashMap<(PlayerId, StationId), BTreeMap<ItemId, u64>>,
    /// Recency order, oldest first. A player can appear more than once (the
    /// stale occurrence is just skipped on eviction); capacity is small
    /// enough that this doesn't need a proper LRU data structure.
    recency: VecDeque<(PlayerId, StationId)>,
}

/// How many players' Station inventory to keep cached at once. Arbitrary but
/// generous relative to `POPULATION_CAP` (500) -- most docked/recently-active
/// players fit comfortably; the point is bounding memory, not this exact
/// number.
const STATION_INVENTORY_CACHE_CAPACITY: usize = 256;

impl StationInventoryCache {
    pub(super) fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            recency: VecDeque::new(),
        }
    }

    fn touch(&mut self, key: (PlayerId, StationId)) {
        self.recency.push_back(key);
        while self.entries.len() > STATION_INVENTORY_CACHE_CAPACITY {
            let Some(oldest) = self.recency.pop_front() else {
                break;
            };
            // Only actually evict if `oldest` isn't more-recently touched
            // elsewhere in the queue (a stale duplicate).
            if !self.recency.contains(&oldest) {
                self.entries.remove(&oldest);
            }
        }
    }

    /// Populate the cache from a DB read (cache miss) or record a write's
    /// resulting value, and mark `player_id` as just-touched.
    fn insert(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        inventory: BTreeMap<ItemId, u64>,
    ) {
        let key = (player_id, station_id);
        self.entries.insert(key, inventory);
        self.touch(key);
    }

    /// Cache-hit read: clones the entry (cheap -- inventories are small) and
    /// bumps recency. `None` means a cache miss, not "empty inventory" --
    /// callers query SQLite and `insert()` the result.
    fn get_cloned_and_touch(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> Option<BTreeMap<ItemId, u64>> {
        let key = (player_id, station_id);
        let inventory = self.entries.get(&key).cloned();
        if inventory.is_some() {
            self.touch(key);
        }
        inventory
    }

    /// Mutable access for the write path, inserting an empty entry (and
    /// marking it just-touched) if this player isn't cached yet -- the write
    /// path never needs to distinguish "not cached" from "cached and empty".
    fn entry_mut(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> &mut BTreeMap<ItemId, u64> {
        let key = (player_id, station_id);
        if !self.entries.contains_key(&key) {
            self.insert(player_id, station_id, BTreeMap::new());
        } else {
            self.touch(key);
        }
        self.entries
            .get_mut(&key)
            .expect("just inserted or already present")
    }
}

impl SimulationNode {
    /// The player's station inventory in this Sector, owned rather than
    /// borrowed (ADR-0038: a cache hit clones out of the `RefCell`-guarded
    /// cache; a miss queries SQLite and populates the cache before
    /// returning). `None` only when the player has never had a Station
    /// inventory entry at all -- `station_item_count` treats that the same
    /// as an empty one.
    pub fn station_inventory(
        &self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> Option<BTreeMap<ItemId, u64>> {
        let mut cache = self.station_inventory_cache.borrow_mut();
        if let Some(inventory) = cache.get_cloned_and_touch(player_id, station_id) {
            return Some(inventory);
        }
        let inventory = self.station_inventory_db.get_all(player_id, station_id);
        cache.insert(player_id, station_id, inventory.clone());
        if inventory.is_empty() {
            None
        } else {
            Some(inventory)
        }
    }

    /// Count one item stack inside the player's station inventory.
    pub fn station_item_count(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
    ) -> u64 {
        self.station_inventory(player_id, station_id)
            .and_then(|inv| inv.get(&item_id).copied())
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn replace_station_inventory(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        inventory: BTreeMap<ItemId, u64>,
    ) {
        for (item_id, count) in &inventory {
            self.station_inventory_db
                .credit(player_id, station_id, *item_id, *count);
        }
        self.station_inventory_cache
            .get_mut()
            .insert(player_id, station_id, inventory);
    }

    pub(super) fn ensure_client_admission_grant(&mut self, event: &ClientAdmissionCommitted) {
        self.station_inventory_db
            .ensure_client_admission_grant(
                event.ship_id,
                event.player_id,
                event.resume_ticket,
                event.starter_station_id,
                event.starter_item_id,
                event.starter_item_count,
            )
            .expect("client admission Station grant transaction");
        let inventory = self
            .station_inventory_db
            .get_all(event.player_id, event.starter_station_id);
        self.station_inventory_cache.get_mut().insert(
            event.player_id,
            event.starter_station_id,
            inventory,
        );
    }

    pub(super) fn reconcile_client_admission_grants(&mut self) -> rusqlite::Result<()> {
        // Admission reconciliation is a runtime/repository concern. The
        // storage-independent engine cannot reconstruct it from an event log;
        // callers must feed committed admission records through the explicit
        // projection API before opening a client session.
        Ok(())
    }

    /// Add `count` of `item_id` to the player's station inventory. Writes
    /// through to SQLite synchronously (ADR-0038) before updating the cache
    /// -- Station commands are low-frequency and player-triggered, not
    /// per-tick, so this doesn't need a write-behind queue.
    pub fn credit_station_item(
        &mut self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) {
        if count == 0 {
            return;
        }
        // Write through first, always. Then either patch the cache entry
        // with the same delta (it holds pre-write data if it was already
        // cached) or, if this player wasn't cached at all, load the
        // already-updated value straight from SQLite -- patching a freshly
        // DB-loaded value would double-apply the delta.
        self.station_inventory_db
            .credit(player_id, station_id, item_id, count);
        let already_cached = self
            .station_inventory_cache
            .get_mut()
            .get_cloned_and_touch(player_id, station_id)
            .is_some();
        if already_cached {
            *self
                .station_inventory_cache
                .get_mut()
                .entry_mut(player_id, station_id)
                .entry(item_id)
                .or_default() += count;
        } else {
            let inventory = self.station_inventory_db.get_all(player_id, station_id);
            self.station_inventory_cache
                .get_mut()
                .insert(player_id, station_id, inventory);
        }
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
        // The current SQLite-backed path makes the rejection decision here,
        // because a fresh cache miss may still have a stack in SQLite. The
        // ADR-0049 target moves this decision to the journal-owned Station
        // aggregate and keeps SQLite as an idempotent projection.
        self.station_inventory_db
            .try_debit(player_id, station_id, item_id, count)?;
        let already_cached = self
            .station_inventory_cache
            .get_mut()
            .get_cloned_and_touch(player_id, station_id)
            .is_some();
        if already_cached {
            let cached = self
                .station_inventory_cache
                .get_mut()
                .entry_mut(player_id, station_id);
            if let Some(stack) = cached.get_mut(&item_id) {
                *stack = stack.saturating_sub(count);
                if *stack == 0 {
                    cached.remove(&item_id);
                }
            }
        } else {
            let inventory = self.station_inventory_db.get_all(player_id, station_id);
            self.station_inventory_cache
                .get_mut()
                .insert(player_id, station_id, inventory);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{ItemId, NodeId, SectorBounds, SectorId};

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
        assert_eq!(
            node.station_item_count(player_b, TEST_STATION_ID, ItemId::ScrapMetal),
            0
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

    /// ADR-0038: the whole point of the SQLite-backed cache is to never hold
    /// every player in memory at once. Touch more players than
    /// `STATION_INVENTORY_CACHE_CAPACITY`, then read one from early in the
    /// sequence back -- it must still be correct (served from SQLite on the
    /// resulting cache miss), proving eviction doesn't lose data.
    #[test]
    fn station_inventory_survives_cache_eviction() {
        let mut node = node();
        for i in 0..(STATION_INVENTORY_CACHE_CAPACITY as u64 + 10) {
            node.credit_station_item(PlayerId(i), TEST_STATION_ID, ItemId::ScrapMetal, i + 1);
        }

        // PlayerId(0) was touched first and is well outside the capacity
        // window by the time the loop finishes -- it must have been evicted.
        assert_eq!(
            node.station_item_count(PlayerId(0), TEST_STATION_ID, ItemId::ScrapMetal),
            1
        );
        // The most recently touched player is still a guaranteed cache hit.
        assert_eq!(
            node.station_item_count(
                PlayerId(STATION_INVENTORY_CACHE_CAPACITY as u64 + 9),
                TEST_STATION_ID,
                ItemId::ScrapMetal
            ),
            STATION_INVENTORY_CACHE_CAPACITY as u64 + 10
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
        assert_eq!(
            node.station_item_count(player_id, TEST_STATION_ID, ItemId::ScrapMetal),
            2
        );
        assert!(node
            .try_debit_station_item(player_id, TEST_STATION_ID, ItemId::ScrapMetal, 2)
            .is_ok());
        assert_eq!(
            node.station_item_count(player_id, TEST_STATION_ID, ItemId::ScrapMetal),
            0
        );
    }
}
