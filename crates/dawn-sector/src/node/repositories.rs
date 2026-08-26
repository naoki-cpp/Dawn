//! SQLite repository boundary for Sector protocol state and projections.
//!
//! `SectorRepository` owns exactly one node-local SQLite connection. Its
//! bounded-context views live in `repositories/`, while this root keeps only
//! schema setup, shared codecs, and cross-view transaction coordination.

mod admission;
mod identity;
mod station_inventory;

use dawn_core::ItemId;
use rusqlite::Connection;

pub use station_inventory::{ProjectionApplyError, ProjectionApplyResult, ProjectionReadError};

pub(super) struct SectorRepository {
    conn: Connection,
}

pub(super) use admission::AdmissionRepository;
pub(super) use identity::IdentityRepository;
pub(super) use station_inventory::StationInventoryRepository;

/// Explicit local transaction boundary for updates spanning repository views.
pub(super) struct SectorTransaction<'a> {
    transaction: rusqlite::Transaction<'a>,
}

pub(super) fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {
    item_id.storage_columns().into_tuple()
}

pub(super) fn columns_to_item_id(
    item_type: &str,
    module_id: u32,
    ship_type_id: u32,
) -> Option<ItemId> {
    ItemId::from_storage_columns(item_type, module_id, ship_type_id).ok()
}

pub(super) fn ticket_from_blob(bytes: Vec<u8>) -> rusqlite::Result<dawn_core::ResumeTicket> {
    let bytes: [u8; dawn_core::ResumeTicket::BYTE_LEN] = bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "resume ticket must contain exactly 32 bytes",
            )),
        )
    })?;
    Ok(dawn_core::ResumeTicket::from_bytes(bytes))
}

pub(super) fn ship_id_from_text(raw: String, column: usize) -> rusqlite::Result<dawn_core::ShipId> {
    raw.parse::<u64>()
        .map(|raw| dawn_core::ShipId(dawn_core::EntityId::from_raw(raw)))
        .map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                column,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid ShipId",
                )),
            )
        })
}

pub(super) fn non_negative_id(
    value: i64,
    column: usize,
    kind: &'static str,
) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{kind} must be non-negative"),
            )),
        )
    })
}

pub(super) fn invariant_error(message: &'static str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message,
    )))
}

pub(super) fn player_id_as_sql(player_id: dawn_core::PlayerId) -> rusqlite::Result<i64> {
    i64::try_from(player_id.0).map_err(|_| invariant_error("PlayerId exceeds SQLite INTEGER"))
}

pub(super) fn next_player_id_as_sql(player_id: dawn_core::PlayerId) -> rusqlite::Result<i64> {
    player_id_as_sql(player_id)?
        .checked_add(1)
        .ok_or(rusqlite::Error::InvalidQuery)
}

pub(super) fn next_ship_counter_as_sql(ship_id: dawn_core::ShipId) -> rusqlite::Result<i64> {
    let next_counter = ship_id
        .0
        .counter()
        .checked_add(1)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    i64::try_from(next_counter)
        .map_err(|_| invariant_error("ShipId counter exceeds SQLite INTEGER"))
}

impl SectorRepository {
    pub(super) fn open(path: &str) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub(super) fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS station_inventory (
                player_id     INTEGER NOT NULL,
                station_id    INTEGER NOT NULL,
                item_type     TEXT    NOT NULL,
                module_id     INTEGER NOT NULL DEFAULT 0,
                ship_type_id  INTEGER NOT NULL DEFAULT 0,
                count         INTEGER NOT NULL CHECK (count > 0),
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
                count         INTEGER NOT NULL CHECK (count > 0)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS client_admission_prepared (
                ship_id       TEXT PRIMARY KEY,
                player_id     INTEGER NOT NULL UNIQUE CHECK (player_id >= 0),
                spawn_x       REAL NOT NULL,
                spawn_y       REAL NOT NULL,
                spawn_z       REAL NOT NULL,
                resume_ticket BLOB NOT NULL UNIQUE CHECK (length(resume_ticket) = 32)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS client_ship_ownership (
                ship_id       TEXT PRIMARY KEY,
                player_id     INTEGER NOT NULL CHECK (player_id >= 0),
                resume_ticket BLOB NOT NULL UNIQUE CHECK (length(resume_ticket) = 32),
                pending_resume_ticket BLOB UNIQUE
                    CHECK (pending_resume_ticket IS NULL OR length(pending_resume_ticket) = 32)
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
        conn.execute(
            "CREATE TABLE IF NOT EXISTS consumed_player_ids (
                player_id INTEGER PRIMARY KEY
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS consumed_ship_ids (
                ship_id TEXT PRIMARY KEY
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS player_identity_allocator (
                singleton      INTEGER PRIMARY KEY CHECK (singleton = 1),
                next_player_id INTEGER NOT NULL CHECK (next_player_id >= 0)
            )",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO player_identity_allocator (singleton, next_player_id)
             VALUES (1, 0)",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS ship_identity_allocator (
                node_id           INTEGER PRIMARY KEY,
                next_ship_counter INTEGER NOT NULL CHECK (next_ship_counter >= 0)
            )",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS station_projection_transitions (
                transition_id TEXT PRIMARY KEY,
                journal_index INTEGER NOT NULL UNIQUE CHECK (journal_index >= 0),
                journal_len INTEGER NOT NULL DEFAULT 1 CHECK (journal_len > 0)
            )",
            [],
        )?;
        let has_station_projection_journal_len: bool = conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info('station_projection_transitions')
                WHERE name = 'journal_len'
            )",
            [],
            |row| row.get(0),
        )?;
        if !has_station_projection_journal_len {
            conn.execute(
                "ALTER TABLE station_projection_transitions
                 ADD COLUMN journal_len INTEGER NOT NULL DEFAULT 1
                 CHECK (journal_len > 0)",
                [],
            )?;
        }
        conn.execute(
            "CREATE TABLE IF NOT EXISTS station_projection_cursor (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                projection_applied_through INTEGER NOT NULL
                    CHECK (projection_applied_through >= 0)
            )",
            [],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO station_projection_cursor
             (singleton, projection_applied_through) VALUES (1, 0)",
            [],
        )?;

        let repository = Self { conn };
        repository.identities().rebuild_identity_watermarks()?;
        Ok(repository)
    }

    pub(super) fn admissions(&self) -> AdmissionRepository<'_> {
        AdmissionRepository { repository: self }
    }

    pub(super) fn identities(&self) -> IdentityRepository<'_> {
        IdentityRepository { repository: self }
    }

    pub(super) fn station_inventory(&self) -> StationInventoryRepository<'_> {
        StationInventoryRepository { repository: self }
    }

    pub(super) fn transaction(&mut self) -> rusqlite::Result<SectorTransaction<'_>> {
        Ok(SectorTransaction {
            transaction: self.conn.transaction()?,
        })
    }
}
