//! Current SQLite persistence seam for Station rows plus admission/identity
//! protocol rows.
//!
//! # ADR-0049 / #277 authority split
//!
//! This module predates the final #277 repository split and currently stores
//! several domains in one `rusqlite::Connection`:
//!
//! - `station_inventory`: **not** independent Sector-world authority under
//!   ADR-0049. The target model is an idempotent SQLite/read-model projection of
//!   journal-owned Station state, with a global contiguous applied-through
//!   watermark.
//! - `client_admission_prepared`, `client_admission_grants`, and
//!   `client_ship_ownership`: durable admission/identity **protocol authority**
//!   may live in #277 repositories because reservations and resume-ticket state
//!   can exist before a Ship is materialized in Sector world state. Those rows
//!   require explicit reconciliation/catch-up with committed Sector transitions.
//!
//! The current catch-all type and mutation ordering below are migration baseline,
//! not the final repository API. #277 splits the domains and #278 owns runtime
//! reconciliation/ack ordering. Do not infer from this file that all SQLite rows
//! are Station authority or that the current SQLite-first paths are normative.
//!
//! The existing flat SQLite item columns are preserved by the current
//! implementation, but their meaning is owned by `dawn_core::ItemId` rather than
//! duplicated here. Pre-release storage compatibility is not required by the
//! destructive refactor.

use std::collections::BTreeMap;

use dawn_core::{ItemId, PlayerId, Position, ResumeTicket, ShipId, StationId};
#[cfg(test)]
use dawn_core::{ModuleId, ShipTypeId};
use rusqlite::{params, Connection, OptionalExtension};

use super::station::StationOperationRejection;

fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {
    item_id.storage_columns().into_tuple()
}

fn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {
    ItemId::from_storage_columns(item_type, module_id, ship_type_id).ok()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PreparedClientAdmission {
    pub ship_id: ShipId,
    pub player_id: PlayerId,
    pub spawn_position: Position,
    pub resume_ticket: ResumeTicket,
}

fn ticket_from_blob(bytes: Vec<u8>) -> rusqlite::Result<ResumeTicket> {
    let bytes: [u8; ResumeTicket::BYTE_LEN] = bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "resume ticket must contain exactly 32 bytes",
            )),
        )
    })?;
    Ok(ResumeTicket::from_bytes(bytes))
}

/// Current catch-all SQLite adapter for one Sector node.
///
/// It wraps one `rusqlite::Connection` for the legacy Station + admission +
/// identity schema. Under ADR-0049/#277, Station rows become an idempotent
/// projection while admission/identity rows may remain durable protocol authority
/// behind separate repository APIs. This type is scheduled for removal by #277.
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
        conn.execute(
            "CREATE TABLE IF NOT EXISTS client_admission_grants (
                ship_id       TEXT PRIMARY KEY,
                player_id     INTEGER NOT NULL,
                station_id    INTEGER NOT NULL,
                item_type     TEXT    NOT NULL,
                module_id     INTEGER NOT NULL DEFAULT 0,
                ship_type_id  INTEGER NOT NULL DEFAULT 0,
                count         INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS client_admission_prepared (
                ship_id       TEXT PRIMARY KEY,
                player_id     INTEGER NOT NULL UNIQUE,
                spawn_x       REAL NOT NULL,
                spawn_y       REAL NOT NULL,
                spawn_z       REAL NOT NULL,
                resume_ticket BLOB NOT NULL UNIQUE
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS client_ship_ownership (
                ship_id       TEXT PRIMARY KEY,
                player_id     INTEGER NOT NULL,
                resume_ticket BLOB NOT NULL UNIQUE,
                pending_resume_ticket BLOB UNIQUE
            )",
            [],
        )?;
        let has_pending_resume_ticket: bool = conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info('client_ship_ownership')
                WHERE name = 'pending_resume_ticket'
            )",
            [],
            |row| row.get(0),
        )?;
        if !has_pending_resume_ticket {
            conn.execute(
                "ALTER TABLE client_ship_ownership
                 ADD COLUMN pending_resume_ticket BLOB",
                [],
            )?;
        }
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS
             idx_client_ship_ownership_pending_resume_ticket
             ON client_ship_ownership(pending_resume_ticket)",
            [],
        )?;
        Ok(Self { conn })
    }

    /// Persist a fresh admission reservation before any handshake frame exposes
    /// its identity to the client.
    ///
    /// Under ADR-0049/#277 this prepared row is durable admission protocol
    /// authority until the corresponding Ship is materialized/aborted. It is not
    /// merely a Station projection row, and restart must reuse/reconcile the same
    /// reservation rather than allocate a conflicting identity.
    pub(super) fn reserve_client_admission(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO client_admission_prepared
             (ship_id, player_id, spawn_x, spawn_y, spawn_z, resume_ticket)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                spawn_position.x,
                spawn_position.y,
                spawn_position.z,
                resume_ticket.as_bytes().as_slice(),
            ],
        )?;
        Ok(())
    }

    pub(super) fn prepared_client_admission(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<PreparedClientAdmission>> {
        self.conn
            .query_row(
                "SELECT player_id, spawn_x, spawn_y, spawn_z, resume_ticket
                 FROM client_admission_prepared WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| {
                    Ok(PreparedClientAdmission {
                        ship_id,
                        player_id: PlayerId(row.get::<_, i64>(0)? as u64),
                        spawn_position: Position::new(row.get(1)?, row.get(2)?, row.get(3)?),
                        resume_ticket: ticket_from_blob(row.get(4)?)?,
                    })
                },
            )
            .optional()
    }

    pub(super) fn prepared_client_admission_by_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<PreparedClientAdmission>> {
        self.conn
            .query_row(
                "SELECT ship_id, player_id, spawn_x, spawn_y, spawn_z
                 FROM client_admission_prepared WHERE resume_ticket = ?1",
                params![resume_ticket.as_bytes().as_slice()],
                |row| {
                    let ship_raw: String = row.get(0)?;
                    let ship_id = ship_raw
                        .parse::<u64>()
                        .map(|raw| ShipId(dawn_core::EntityId::from_raw(raw)))
                        .map_err(|_| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "invalid prepared ShipId",
                                )),
                            )
                        })?;
                    Ok(PreparedClientAdmission {
                        ship_id,
                        player_id: PlayerId(row.get::<_, i64>(1)? as u64),
                        spawn_position: Position::new(row.get(2)?, row.get(3)?, row.get(4)?),
                        resume_ticket,
                    })
                },
            )
            .optional()
    }

    pub(super) fn client_owner(&self, ship_id: ShipId) -> rusqlite::Result<Option<PlayerId>> {
        self.conn
            .query_row(
                "SELECT player_id FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| Ok(PlayerId(row.get::<_, i64>(0)? as u64)),
            )
            .optional()
    }

    pub(super) fn client_ownership_by_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<(PlayerId, ShipId)>> {
        self.conn
            .query_row(
                "SELECT player_id, ship_id FROM client_ship_ownership
                 WHERE resume_ticket = ?1 OR pending_resume_ticket = ?1",
                params![resume_ticket.as_bytes().as_slice()],
                |row| {
                    let ship_raw: String = row.get(1)?;
                    let ship_id = ship_raw
                        .parse::<u64>()
                        .map(|raw| ShipId(dawn_core::EntityId::from_raw(raw)))
                        .map_err(|_| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "invalid owned ShipId",
                                )),
                            )
                        })?;
                    Ok((PlayerId(row.get::<_, i64>(0)? as u64), ship_id))
                },
            )
            .optional()
    }

    pub(super) fn client_resume_tickets(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<(ResumeTicket, Option<ResumeTicket>)>> {
        self.conn
            .query_row(
                "SELECT resume_ticket, pending_resume_ticket
                 FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| {
                    let current = ticket_from_blob(row.get(0)?)?;
                    let pending = row
                        .get::<_, Option<Vec<u8>>>(1)?
                        .map(ticket_from_blob)
                        .transpose()?;
                    Ok((current, pending))
                },
            )
            .optional()
    }

    pub(super) fn record_client_ownership(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO client_ship_ownership (ship_id, player_id, resume_ticket)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (ship_id) DO UPDATE SET
               player_id = excluded.player_id,
               resume_ticket = excluded.resume_ticket,
               pending_resume_ticket = NULL",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                resume_ticket.as_bytes().as_slice()
            ],
        )?;
        Ok(())
    }

    pub(super) fn record_client_ownership_with_pending(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        pending_resume_ticket: Option<ResumeTicket>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO client_ship_ownership
             (ship_id, player_id, resume_ticket, pending_resume_ticket)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (ship_id) DO UPDATE SET
               player_id = excluded.player_id,
               resume_ticket = excluded.resume_ticket,
               pending_resume_ticket = excluded.pending_resume_ticket",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                resume_ticket.as_bytes().as_slice(),
                pending_resume_ticket.map(|ticket| ticket.as_bytes().to_vec()),
            ],
        )?;
        Ok(())
    }

    /// Persist the ticket that may be exposed in Welcome without invalidating
    /// the ticket the client used for this attempt. Retrying the committed ticket
    /// reuses an existing pending ticket. Retrying the pending ticket promotes it
    /// to current before staging its successor, so an abort still leaves the
    /// client-presented ticket valid.
    pub(super) fn stage_client_resume_ticket(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
        presented_ticket: ResumeTicket,
        proposed_next_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<ResumeTicket>> {
        let tx = self.conn.transaction()?;
        let stored = tx
            .query_row(
                "SELECT player_id, resume_ticket, pending_resume_ticket
                 FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| {
                    let stored_player = PlayerId(row.get::<_, i64>(0)? as u64);
                    let current_ticket = ticket_from_blob(row.get(1)?)?;
                    let pending_ticket = row
                        .get::<_, Option<Vec<u8>>>(2)?
                        .map(ticket_from_blob)
                        .transpose()?;
                    Ok((stored_player, current_ticket, pending_ticket))
                },
            )
            .optional()?;
        let Some((stored_player, current_ticket, pending_ticket)) = stored else {
            return Ok(None);
        };
        if stored_player != player_id {
            return Ok(None);
        }

        let (next_current, next_pending, advertised_ticket) = if presented_ticket == current_ticket
        {
            let advertised_ticket = pending_ticket.unwrap_or(proposed_next_ticket);
            (current_ticket, Some(advertised_ticket), advertised_ticket)
        } else if pending_ticket == Some(presented_ticket) {
            (
                presented_ticket,
                Some(proposed_next_ticket),
                proposed_next_ticket,
            )
        } else {
            return Ok(None);
        };

        tx.execute(
            "UPDATE client_ship_ownership
             SET resume_ticket = ?3, pending_resume_ticket = ?4
             WHERE ship_id = ?1 AND player_id = ?2",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                next_current.as_bytes().as_slice(),
                next_pending.map(|ticket| ticket.as_bytes().to_vec()),
            ],
        )?;
        tx.commit()?;
        Ok(Some(advertised_ticket))
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

    /// Apply a starter grant exactly once, keyed by the committed ShipId.
    /// The ledger marker, inventory upsert, ownership binding, and prepared-row
    /// cleanup share one SQLite transaction.
    pub(super) fn ensure_client_admission_grant(
        &mut self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> rusqlite::Result<bool> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let tx = self.conn.transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO client_admission_grants
             (ship_id, player_id, station_id, item_type, module_id, ship_type_id, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                station_id.0 as i64,
                item_type,
                module_id,
                ship_type_id,
                count as i64,
            ],
        )?;
        if inserted == 1 {
            tx.execute(
                "INSERT INTO station_inventory
                 (player_id, station_id, item_type, module_id, ship_type_id, count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (player_id, station_id, item_type, module_id, ship_type_id)
                 DO UPDATE SET count = count + excluded.count",
                params![
                    player_id.0 as i64,
                    station_id.0 as i64,
                    item_type,
                    module_id,
                    ship_type_id,
                    count as i64,
                ],
            )?;
        }
        // The event carries the ticket issued at the original fresh commit,
        // but a later resume may already have rotated the durable binding.
        // Reconciliation must restore a missing ownership row without
        // overwriting the current or staged reconnect ticket.
        tx.execute(
            "INSERT INTO client_ship_ownership (ship_id, player_id, resume_ticket)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (ship_id) DO NOTHING",
            params![
                ship_id.raw().to_string(),
                player_id.0 as i64,
                resume_ticket.as_bytes().as_slice()
            ],
        )?;
        tx.execute(
            "DELETE FROM client_admission_prepared WHERE ship_id = ?1",
            params![ship_id.raw().to_string()],
        )?;
        tx.commit()?;
        Ok(inserted == 1)
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

    // `migrate_from_snapshot` (one-time import from the old
    // `StateSnapshot.station_inventories` field) is gone along with that field.
    // It was unreachable: postcard reads struct fields positionally, so a
    // snapshot written before the field existed fails to load outright rather
    // than arriving here with a populated map (ADR-0017 format compatibility).
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_then_get_all_round_trips() {
        let db = StationInventoryDb::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 5);
        db.credit(PlayerId(1), StationId(7), ItemId::Module(ModuleId(3)), 2);
        db.credit(
            PlayerId(1),
            StationId(7),
            ItemId::PackagedShip(ShipTypeId(7)),
            1,
        );

        let inv = db.get_all(PlayerId(1), StationId(7));
        assert_eq!(inv.get(&ItemId::ScrapMetal), Some(&5));
        assert_eq!(inv.get(&ItemId::Module(ModuleId(3))), Some(&2));
        assert_eq!(inv.get(&ItemId::PackagedShip(ShipTypeId(7))), Some(&1));
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
    fn prepared_admission_and_ownership_round_trip() {
        let mut db = StationInventoryDb::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 7);
        let player_id = PlayerId(1);
        let spawn = Position::new(10.0, 20.0, 30.0);
        let resume_ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        db.reserve_client_admission(ship_id, player_id, spawn, resume_ticket)
            .unwrap();
        assert_eq!(
            db.prepared_client_admission(ship_id).unwrap(),
            Some(PreparedClientAdmission {
                ship_id,
                player_id,
                spawn_position: spawn,
                resume_ticket,
            })
        );

        let item = ItemId::PackagedShip(ShipTypeId(7));
        db.ensure_client_admission_grant(ship_id, player_id, resume_ticket, StationId(7), item, 1)
            .unwrap();
        assert_eq!(db.prepared_client_admission(ship_id).unwrap(), None);
        assert_eq!(db.client_owner(ship_id).unwrap(), Some(player_id));
    }

    #[test]
    fn existing_ownership_schema_can_stage_and_promote_a_resume_ticket() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE client_ship_ownership (
                ship_id TEXT PRIMARY KEY,
                player_id INTEGER NOT NULL,
                resume_ticket BLOB NOT NULL UNIQUE
            )",
            [],
        )
        .unwrap();
        let mut db = StationInventoryDb::init(conn).unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 8);
        let player_id = PlayerId(1);
        let current_ticket = ResumeTicket::from_bytes([8; ResumeTicket::BYTE_LEN]);
        let next_ticket = ResumeTicket::from_bytes([9; ResumeTicket::BYTE_LEN]);

        db.record_client_ownership(ship_id, player_id, current_ticket)
            .unwrap();
        assert_eq!(
            db.stage_client_resume_ticket(ship_id, player_id, current_ticket, next_ticket)
                .unwrap(),
            Some(next_ticket)
        );
        assert_eq!(
            db.client_ownership_by_ticket(next_ticket).unwrap(),
            Some((player_id, ship_id))
        );

        db.record_client_ownership(ship_id, player_id, next_ticket)
            .unwrap();
        assert_eq!(db.client_ownership_by_ticket(current_ticket).unwrap(), None);
        assert_eq!(
            db.client_ownership_by_ticket(next_ticket).unwrap(),
            Some((player_id, ship_id))
        );
    }

    #[test]
    fn client_admission_grant_is_idempotent() {
        let mut db = StationInventoryDb::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 7);
        let item = ItemId::PackagedShip(ShipTypeId(7));
        let resume_ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        assert!(db
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                resume_ticket,
                StationId(7),
                item,
                1,
            )
            .unwrap());
        assert!(!db
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                resume_ticket,
                StationId(7),
                item,
                1,
            )
            .unwrap());
        assert_eq!(db.get_all(PlayerId(1), StationId(7)).get(&item), Some(&1));
    }

    #[test]
    fn admission_grant_reconciliation_preserves_rotated_resume_tickets() {
        let mut db = StationInventoryDb::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 9);
        let player_id = PlayerId(1);
        let original_ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        let current_ticket = ResumeTicket::from_bytes([8; ResumeTicket::BYTE_LEN]);
        let pending_ticket = ResumeTicket::from_bytes([9; ResumeTicket::BYTE_LEN]);

        db.record_client_ownership(ship_id, player_id, current_ticket)
            .unwrap();
        assert_eq!(
            db.stage_client_resume_ticket(ship_id, player_id, current_ticket, pending_ticket,)
                .unwrap(),
            Some(pending_ticket)
        );

        db.ensure_client_admission_grant(
            ship_id,
            player_id,
            original_ticket,
            StationId(7),
            ItemId::PackagedShip(ShipTypeId(7)),
            1,
        )
        .unwrap();

        assert_eq!(
            db.client_resume_tickets(ship_id).unwrap(),
            Some((current_ticket, Some(pending_ticket)))
        );
        assert_eq!(
            db.client_ownership_by_ticket(original_ticket).unwrap(),
            None
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
}
