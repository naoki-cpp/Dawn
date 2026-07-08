//! SQLite-backed durable storage for per-station Station inventory (ADR-0038).
//!
//! Station inventory used to live entirely in a `BTreeMap` that was cloned
//! whole into every periodic `StateSnapshot` and reloaded whole on restart
//! (ADR-0034 9B's intentionally-simple MVP). This module is the "storage
//! seam" ADR-0034 §2 anticipated: SQLite is now the durable authority, and
//! `SimulationNode` only keeps a bounded in-memory cache of recently-touched
//! players on top of it (`node/station.rs`).
//!
//! Column encoding mirrors the flat `item_type`/`module_id`/`ship_type_id`
//! shape `serialization.rs::item_id_to_row_json` already uses for the wire
//! format, rather than inventing a new one -- easier to eyeball with a
//! sqlite3 CLI, and one fewer encoding to keep in sync.

use std::collections::BTreeMap;

use dawn_core::{ItemId, ModuleId, PlayerId, ShipTypeId, StationId};
use rusqlite::{params, Connection, OptionalExtension};

use super::station::StationOperationRejection;

/// One row's flat encoding: `(item_type, module_id, ship_type_id)`.
/// `module_id`/`ship_type_id` are `0` for whichever doesn't apply, matching
/// `item_id_to_row_json`'s convention.
fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {
    match item_id {
        ItemId::Module(module_id) => ("Module", module_id.0, 0),
        ItemId::PackagedShip(ship_type_id) => ("PackagedShip", 0, ship_type_id.0),
        ItemId::ScrapMetal => ("ScrapMetal", 0, 0),
    }
}

fn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {
    match item_type {
        "Module" => Some(ItemId::Module(ModuleId(module_id))),
        "PackagedShip" => Some(ItemId::PackagedShip(ShipTypeId(ship_type_id))),
        "ScrapMetal" => Some(ItemId::ScrapMetal),
        _ => None,
    }
}

/// Durable Station inventory store for one Sector node. Wraps a single
/// `rusqlite::Connection` -- either a real file (production) or `:memory:`
/// (tests/demos, matching `InMemoryEventStore`'s role for the event log).
pub(super) struct StationInventoryDb {
    conn: Connection,
}

impl StationInventoryDb {
    /// Open (creating if absent) the on-disk database at `path`.
    pub(super) fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// A private, non-persistent database -- the default for `SimulationNode::new`/
    /// `with_store`/`restore_from` so tests and demos never touch disk unless
    /// a caller explicitly opts in via `open`.
    pub(super) fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS station_inventory (
                player_id     INTEGER NOT NULL,
                station_id    INTEGER NOT NULL,
                item_type     TEXT    NOT NULL,
                module_id     INTEGER NOT NULL DEFAULT 0,
                ship_type_id  INTEGER NOT NULL DEFAULT 0,
                count         INTEGER NOT NULL,
                PRIMARY KEY (player_id, station_id, item_type, module_id, ship_type_id)
            )",
            [],
        )?;
        Ok(Self { conn })
    }

    /// Every item stack this player currently owns at `station_id`.
    /// Used on a cache miss (`node/station.rs`).
    pub(super) fn get_all(
        &self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> BTreeMap<ItemId, u64> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_type, module_id, ship_type_id, count
                 FROM station_inventory WHERE player_id = ?1 AND station_id = ?2",
            )
            .expect("station_inventory table exists");
        let rows = stmt
            .query_map(params![player_id.0 as i64, station_id.0 as i64], |row| {
                let item_type: String = row.get(0)?;
                let module_id: i64 = row.get(1)?;
                let ship_type_id: i64 = row.get(2)?;
                let count: i64 = row.get(3)?;
                Ok((
                    item_type,
                    module_id as u32,
                    ship_type_id as u32,
                    count as u64,
                ))
            })
            .expect("query is well-formed");

        rows.filter_map(|r| r.ok())
            .filter_map(|(item_type, module_id, ship_type_id, count)| {
                columns_to_item_id(&item_type, module_id, ship_type_id).map(|id| (id, count))
            })
            .collect()
    }

    /// Add `count` of `item_id` to `player_id`'s stack at `station_id` (upsert).
    pub(super) fn credit(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        self.conn
            .execute(
                "INSERT INTO station_inventory (player_id, station_id, item_type, module_id, ship_type_id, count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (player_id, station_id, item_type, module_id, ship_type_id)
                 DO UPDATE SET count = count + excluded.count",
                params![
                    player_id.0 as i64,
                    station_id.0 as i64,
                    item_type,
                    module_id,
                    ship_type_id,
                    count as i64
                ],
            )
            .expect("station_inventory upsert");
    }

    /// Subtract `count` from `player_id`'s stack at `station_id`, rejecting rather than going
    /// negative (mirrors the rejection reasons `station.rs` already surfaces
    /// to callers). Deletes the row once it hits zero.
    pub(super) fn try_debit(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), StationOperationRejection> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let current: Option<i64> = self
            .conn
            .query_row(
                "SELECT count FROM station_inventory
                 WHERE player_id = ?1 AND station_id = ?2 AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                params![
                    player_id.0 as i64,
                    station_id.0 as i64,
                    item_type,
                    module_id,
                    ship_type_id
                ],
                |row| row.get(0),
            )
            .optional()
            .expect("query is well-formed");

        let Some(current) = current else {
            return Err(StationOperationRejection::MissingStationItem);
        };
        if current < count as i64 {
            return Err(StationOperationRejection::InsufficientStationItem);
        }

        let remaining = current - count as i64;
        if remaining == 0 {
            self.conn
                .execute(
                    "DELETE FROM station_inventory
                     WHERE player_id = ?1 AND station_id = ?2 AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                    params![
                        player_id.0 as i64,
                        station_id.0 as i64,
                        item_type,
                        module_id,
                        ship_type_id
                    ],
                )
                .expect("station_inventory delete");
        } else {
            self.conn
                .execute(
                    "UPDATE station_inventory SET count = ?6
                     WHERE player_id = ?1 AND station_id = ?2 AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                    params![
                        player_id.0 as i64,
                        station_id.0 as i64,
                        item_type,
                        module_id,
                        ship_type_id,
                        remaining
                    ],
                )
                .expect("station_inventory update");
        }
        Ok(())
    }

    /// One-time bulk import from an old-format `StateSnapshot.station_inventories`
    /// (ADR-0038 back-compat: snapshots taken before this change still carry
    /// the full map). Idempotent to call again (upserts), but callers only
    /// need to call it once, right after restore, when the field is non-empty.
    pub(super) fn migrate_from_snapshot(&self, old: &BTreeMap<PlayerId, BTreeMap<ItemId, u64>>) {
        for (&player_id, inventory) in old {
            for (&item_id, &count) in inventory {
                self.credit(player_id, StationId(0), item_id, count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_then_get_all_round_trips() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 5);
        db.credit(PlayerId(1), StationId(7), ItemId::Module(ModuleId(3)), 2);

        let inv = db.get_all(PlayerId(1), StationId(7));
        assert_eq!(inv.get(&ItemId::ScrapMetal), Some(&5));
        assert_eq!(inv.get(&ItemId::Module(ModuleId(3))), Some(&2));
    }

    #[test]
    fn credit_accumulates_across_calls() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 3);
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);

        assert_eq!(
            db.get_all(PlayerId(1), StationId(7))
                .get(&ItemId::ScrapMetal),
            Some(&5)
        );
    }

    #[test]
    fn try_debit_rejects_a_missing_stack() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        assert_eq!(
            db.try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 1),
            Err(StationOperationRejection::MissingStationItem)
        );
    }

    #[test]
    fn try_debit_rejects_an_insufficient_stack() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert_eq!(
            db.try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 3),
            Err(StationOperationRejection::InsufficientStationItem)
        );
    }

    #[test]
    fn try_debit_removes_the_row_once_it_hits_zero() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert!(db
            .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2)
            .is_ok());
        assert!(db.get_all(PlayerId(1), StationId(7)).is_empty());
    }

    #[test]
    fn try_debit_partial_leaves_the_remainder() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 5);
        assert!(db
            .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2)
            .is_ok());
        assert_eq!(
            db.get_all(PlayerId(1), StationId(7))
                .get(&ItemId::ScrapMetal),
            Some(&3)
        );
    }

    #[test]
    fn migrate_from_snapshot_imports_every_player_and_item() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        let old = BTreeMap::from([
            (
                PlayerId(1),
                BTreeMap::from([(ItemId::ScrapMetal, 4), (ItemId::Module(ModuleId(1)), 1)]),
            ),
            (
                PlayerId(2),
                BTreeMap::from([(ItemId::PackagedShip(ShipTypeId(7)), 1)]),
            ),
        ]);

        db.migrate_from_snapshot(&old);

        assert_eq!(
            db.get_all(PlayerId(1), StationId(0))
                .get(&ItemId::ScrapMetal),
            Some(&4)
        );
        assert_eq!(
            db.get_all(PlayerId(1), StationId(0))
                .get(&ItemId::Module(ModuleId(1))),
            Some(&1)
        );
        assert_eq!(
            db.get_all(PlayerId(2), StationId(0))
                .get(&ItemId::PackagedShip(ShipTypeId(7))),
            Some(&1)
        );
    }
}
