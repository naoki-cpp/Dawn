//! SQLite repositories for admission/identity protocol state and the Station
//! projection.
//!
//! # ADR-0049 / #277 authority split
//!
//! This module keeps one node-local SQLite connection as the local atomic
//! boundary while exposing explicit #277 repository views:
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
//! One connection remains the local atomic boundary, but callers use explicit
//! admission, identity, and Station-projection views. This module owns schema,
//! typed codecs, allocator/reconciliation invariants, and transaction-local
//! atomicity. #278 owns the runtime ordering that feeds committed transitions
//! into the Station projection and gates acknowledgement on its freshness.
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

fn ship_id_from_text(raw: String, column: usize) -> rusqlite::Result<ShipId> {
    raw.parse::<u64>()
        .map(|raw| ShipId(dawn_core::EntityId::from_raw(raw)))
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

fn non_negative_id(value: i64, column: usize, kind: &'static str) -> rusqlite::Result<u64> {
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

fn invariant_error(message: &'static str) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message,
    )))
}

fn player_id_as_sql(player_id: PlayerId) -> rusqlite::Result<i64> {
    i64::try_from(player_id.0).map_err(|_| invariant_error("PlayerId exceeds SQLite INTEGER"))
}

fn next_player_id_as_sql(player_id: PlayerId) -> rusqlite::Result<i64> {
    player_id_as_sql(player_id)?
        .checked_add(1)
        .ok_or(rusqlite::Error::InvalidQuery)
}

fn next_ship_counter_as_sql(ship_id: ShipId) -> rusqlite::Result<i64> {
    let next_counter = ship_id
        .0
        .counter()
        .checked_add(1)
        .ok_or(rusqlite::Error::InvalidQuery)?;
    i64::try_from(next_counter)
        .map_err(|_| invariant_error("ShipId counter exceeds SQLite INTEGER"))
}

/// One local SQLite repository boundary for a Sector node.
///
/// The connection is shared only so a [`SectorTransaction`] can atomically
/// update multiple bounded-context repositories. Callers must use the narrow
/// views returned by [`Self::admissions`], [`Self::identities`], and
/// [`Self::station_inventory`].
pub(super) struct SectorRepository {
    conn: Connection,
}

/// Read/write API for prepared admission protocol state.
pub(super) struct AdmissionRepository<'a> {
    repository: &'a SectorRepository,
}

/// Read/write API for player/ship ownership and resume-ticket identity state.
pub(super) struct IdentityRepository<'a> {
    repository: &'a SectorRepository,
}

/// Read/write API for the idempotent Station inventory projection.
pub(super) struct StationInventoryRepository<'a> {
    repository: &'a SectorRepository,
}

/// Explicit local transaction boundary for updates spanning repository views.
pub(super) struct SectorTransaction<'a> {
    transaction: rusqlite::Transaction<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StationProjectionMutation {
    /// Player owning the Station inventory row.
    pub player_id: PlayerId,
    /// Station receiving or losing the item.
    pub station_id: StationId,
    /// Typed item identity being projected.
    pub item_id: ItemId,
    /// Signed stack delta. Zero is invalid; negative values must not underflow.
    pub delta: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Result of applying one Station projection transition.
pub enum ProjectionApplyResult {
    /// The transition was applied and the cursor now covers this position.
    Applied { projection_applied_through: u64 },
    /// The same transition identity and journal position were already applied.
    Duplicate,
}

/// Failure while applying a committed Station projection transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionApplyError {
    /// The caller skipped a global journal position.
    #[error("station projection is out of order: expected journal index {expected}, got {actual}")]
    OutOfOrder { expected: u64, actual: u64 },
    /// A stable transition identity was reused at another journal position.
    #[error(
        "station projection transition {transition_id} was already recorded at journal index {existing}, not {actual}"
    )]
    DuplicateTransitionAtDifferentIndex {
        transition_id: String,
        existing: u64,
        actual: u64,
    },
    /// The mutation or stored projection data violates its invariants.
    #[error("station projection invariant failed: {reason}")]
    InvalidDelta { reason: &'static str },
}

impl ProjectionApplyError {
    fn invalid(reason: &'static str) -> Self {
        Self::InvalidDelta { reason }
    }
}

/// Failure while reading the persisted Station projection cursor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionReadError {
    #[error("station projection storage read failed: {message}")]
    Storage { message: String },
}

impl SectorRepository {
    /// Open (creating if absent) the on-disk database at `path`.
    pub(super) fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// A private, non-persistent database -- the default for `SimulationNode::new`/
    /// `restore_from` so tests and demos never touch disk unless
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
                journal_index INTEGER NOT NULL UNIQUE CHECK (journal_index >= 0)
            )",
            [],
        )?;
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
        repository.rebuild_identity_watermarks()?;
        Ok(repository)
    }

    /// Rebuild the durable identity indexes after opening an existing
    /// repository. The allocator tables are intentionally derived from all
    /// materialized protocol rows rather than trusting a row that may have
    /// been introduced after the original database was created.
    fn rebuild_identity_watermarks(&self) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO consumed_player_ids (player_id)
             SELECT player_id FROM client_ship_ownership
             UNION SELECT player_id FROM client_admission_prepared",
            [],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO consumed_ship_ids (ship_id)
             SELECT ship_id FROM client_ship_ownership
             UNION SELECT ship_id FROM client_admission_prepared",
            [],
        )?;

        let max_player_id: Option<i64> = tx.query_row(
            "SELECT MAX(player_id) FROM consumed_player_ids",
            [],
            |row| row.get(0),
        )?;
        if let Some(max_player_id) = max_player_id {
            let next_player_id = max_player_id
                .checked_add(1)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            tx.execute(
                "UPDATE player_identity_allocator
                 SET next_player_id = MAX(next_player_id, ?1)
                 WHERE singleton = 1",
                params![next_player_id],
            )?;
        }

        let mut statement = tx.prepare("SELECT ship_id FROM consumed_ship_ids")?;
        let ship_ids = statement.query_map([], |row| ship_id_from_text(row.get(0)?, 0))?;
        let mut next_by_node = BTreeMap::<u8, u64>::new();
        for ship_id in ship_ids {
            let ship_id = ship_id?;
            let next_counter = ship_id
                .0
                .counter()
                .checked_add(1)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            next_by_node
                .entry(ship_id.0.node_id().0)
                .and_modify(|current| *current = (*current).max(next_counter))
                .or_insert(next_counter);
        }
        drop(statement);
        for (node_id, next_counter) in next_by_node {
            let next_counter = i64::try_from(next_counter)
                .map_err(|_| invariant_error("ShipId counter exceeds SQLite INTEGER"))?;
            tx.execute(
                "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
                 VALUES (?1, ?2)
                 ON CONFLICT (node_id) DO UPDATE SET
                   next_ship_counter = MAX(next_ship_counter, excluded.next_ship_counter)",
                params![node_id as i64, next_counter],
            )?;
        }
        tx.commit()
    }

    /// Observe identities materialized in the authoritative Sector snapshot.
    ///
    /// A repository can be created after a snapshot has already restored
    /// ships, so protocol rows alone cannot determine the next ShipId. This
    /// method makes the allocator watermarks monotonic across that boundary.
    pub(super) fn observe_materialized_identities(
        &self,
        ship_ids: impl IntoIterator<Item = ShipId>,
        player_ids: impl IntoIterator<Item = PlayerId>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut next_by_node = BTreeMap::<u8, u64>::new();
        for ship_id in ship_ids {
            tx.execute(
                "INSERT OR IGNORE INTO consumed_ship_ids (ship_id) VALUES (?1)",
                params![ship_id.raw().to_string()],
            )?;
            let next_counter = ship_id
                .0
                .counter()
                .checked_add(1)
                .ok_or(rusqlite::Error::InvalidQuery)?;
            next_by_node
                .entry(ship_id.0.node_id().0)
                .and_modify(|current| *current = (*current).max(next_counter))
                .or_insert(next_counter);
        }
        for player_id in player_ids {
            let player_id = player_id_as_sql(player_id)?;
            tx.execute(
                "INSERT OR IGNORE INTO consumed_player_ids (player_id) VALUES (?1)",
                params![player_id],
            )?;
            tx.execute(
                "UPDATE player_identity_allocator
                 SET next_player_id = MAX(next_player_id, ?1)
                 WHERE singleton = 1",
                params![player_id
                    .checked_add(1)
                    .ok_or(rusqlite::Error::InvalidQuery)?],
            )?;
        }
        for (node_id, next_counter) in next_by_node {
            let next_counter = i64::try_from(next_counter)
                .map_err(|_| invariant_error("ShipId counter exceeds SQLite INTEGER"))?;
            tx.execute(
                "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
                 VALUES (?1, ?2)
                 ON CONFLICT (node_id) DO UPDATE SET
                   next_ship_counter = MAX(next_ship_counter, excluded.next_ship_counter)",
                params![node_id as i64, next_counter],
            )?;
        }
        tx.commit()
    }

    /// Rebuild identity watermarks from durable admission grant markers.
    ///
    /// Admission finalization writes the grant marker, Station row, ownership,
    /// and consumed IDs atomically. Reconciliation must therefore not replay
    /// the Station grant: a player may have consumed the starter item since
    /// the marker was written, and recreating it would duplicate the item.
    pub(super) fn reconcile_admission_identity_watermarks(&self) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let mut next_by_node = BTreeMap::<u8, u64>::new();
        let mut max_player_id: Option<i64> = None;
        {
            let mut statement = tx.prepare(
                "SELECT g.ship_id, g.player_id, g.item_type, g.module_id,
                        g.ship_type_id, g.count, o.player_id
                 FROM client_admission_grants AS g
                 LEFT JOIN client_ship_ownership AS o ON o.ship_id = g.ship_id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    ship_id_from_text(row.get(0)?, 0)?,
                    PlayerId(non_negative_id(row.get(1)?, 1, "PlayerId")?),
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? as u32,
                    row.get::<_, i64>(4)? as u32,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })?;
            for row in rows {
                let (ship_id, player_id, item_type, module_id, ship_type_id, count, owner) = row?;
                if count <= 0 {
                    return Err(invariant_error("admission grant count must be positive"));
                }
                let player_id_sql = player_id_as_sql(player_id)?;
                columns_to_item_id(&item_type, module_id, ship_type_id)
                    .ok_or_else(|| invariant_error("admission grant contains an unknown ItemId"))?;
                if owner != Some(player_id_sql) {
                    return Err(invariant_error(
                        "admission grant is missing its matching ownership binding",
                    ));
                }
                max_player_id =
                    Some(max_player_id.map_or(player_id_sql, |current| current.max(player_id_sql)));
                let next_counter = ship_id
                    .0
                    .counter()
                    .checked_add(1)
                    .ok_or(rusqlite::Error::InvalidQuery)?;
                next_by_node
                    .entry(ship_id.0.node_id().0)
                    .and_modify(|current| *current = (*current).max(next_counter))
                    .or_insert(next_counter);
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO consumed_player_ids (player_id)
             SELECT player_id FROM client_admission_grants",
            [],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO consumed_ship_ids (ship_id)
             SELECT ship_id FROM client_admission_grants",
            [],
        )?;
        if let Some(max_player_id) = max_player_id {
            tx.execute(
                "UPDATE player_identity_allocator
                 SET next_player_id = MAX(next_player_id, ?1)
                 WHERE singleton = 1",
                params![max_player_id
                    .checked_add(1)
                    .ok_or(rusqlite::Error::InvalidQuery)?],
            )?;
        }
        for (node_id, next_counter) in next_by_node {
            let next_counter = i64::try_from(next_counter)
                .map_err(|_| invariant_error("ShipId counter exceeds SQLite INTEGER"))?;
            tx.execute(
                "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
                 VALUES (?1, ?2)
                 ON CONFLICT (node_id) DO UPDATE SET
                   next_ship_counter = MAX(next_ship_counter, excluded.next_ship_counter)",
                params![node_id as i64, next_counter],
            )?;
        }
        tx.commit()
    }

    /// Allocate and durably consume the next fresh player/ship identity before
    /// the caller can expose a Welcome frame. The allocator watermark and the
    /// prepared admission row are committed in one local transaction.
    pub(super) fn reserve_fresh_admission_identity(
        &self,
        node_id: dawn_core::NodeId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<(PlayerId, ShipId)> {
        let tx = self.conn.unchecked_transaction()?;
        let next_player_id: i64 = tx.query_row(
            "SELECT next_player_id FROM player_identity_allocator WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let next_ship_counter: Option<i64> = tx
            .query_row(
                "SELECT next_ship_counter FROM ship_identity_allocator WHERE node_id = ?1",
                params![node_id.0 as i64],
                |row| row.get(0),
            )
            .optional()?;
        let player_id = PlayerId(non_negative_id(next_player_id, 0, "PlayerId")?);
        let player_id_sql = player_id_as_sql(player_id)?;
        let ship_counter = next_ship_counter
            .map(|value| non_negative_id(value, 0, "ShipId counter"))
            .transpose()?;
        let ship_id = ShipId::new(node_id, ship_counter.unwrap_or(0));
        tx.execute(
            "INSERT INTO consumed_player_ids (player_id) VALUES (?1)",
            params![player_id_sql],
        )?;
        tx.execute(
            "INSERT INTO consumed_ship_ids (ship_id) VALUES (?1)",
            params![ship_id.raw().to_string()],
        )?;
        tx.execute(
            "INSERT INTO client_admission_prepared
             (ship_id, player_id, spawn_x, spawn_y, spawn_z, resume_ticket)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ship_id.raw().to_string(),
                player_id_sql,
                spawn_position.x,
                spawn_position.y,
                spawn_position.z,
                resume_ticket.as_bytes().as_slice(),
            ],
        )?;
        let next_player_id = next_player_id
            .checked_add(1)
            .ok_or_else(|| rusqlite::Error::InvalidQuery)?;
        tx.execute(
            "UPDATE player_identity_allocator SET next_player_id = ?1 WHERE singleton = 1",
            params![next_player_id],
        )?;
        tx.execute(
            "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
             VALUES (?1, ?2)
             ON CONFLICT (node_id) DO UPDATE SET next_ship_counter = excluded.next_ship_counter",
            params![node_id.0 as i64, next_ship_counter_as_sql(ship_id)?],
        )?;
        tx.commit()?;
        Ok((player_id, ship_id))
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
                        player_id: PlayerId(non_negative_id(row.get(0)?, 0, "PlayerId")?),
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
                    let ship_id = ship_id_from_text(row.get(0)?, 0)?;
                    Ok(PreparedClientAdmission {
                        ship_id,
                        player_id: PlayerId(non_negative_id(row.get(1)?, 1, "PlayerId")?),
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
                |row| Ok(PlayerId(non_negative_id(row.get(0)?, 0, "PlayerId")?)),
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
                    let ship_id = ship_id_from_text(row.get(1)?, 1)?;
                    Ok((
                        PlayerId(non_negative_id(row.get(0)?, 0, "PlayerId")?),
                        ship_id,
                    ))
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
        let tx = self.conn.unchecked_transaction()?;
        let player_id_sql = player_id_as_sql(player_id)?;
        let next_player_id_sql = next_player_id_as_sql(player_id)?;
        let existing_player: Option<i64> = tx
            .query_row(
                "SELECT player_id FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if existing_player.is_some_and(|existing| existing != player_id_sql) {
            return Err(invariant_error(
                "a ShipId cannot be rebound to a different PlayerId",
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO consumed_player_ids (player_id) VALUES (?1)",
            params![player_id_sql],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO consumed_ship_ids (ship_id) VALUES (?1)",
            params![ship_id.raw().to_string()],
        )?;
        tx.execute(
            "INSERT INTO client_ship_ownership (ship_id, player_id, resume_ticket)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (ship_id) DO UPDATE SET
               player_id = excluded.player_id,
               resume_ticket = excluded.resume_ticket,
               pending_resume_ticket = NULL",
            params![
                ship_id.raw().to_string(),
                player_id_sql,
                resume_ticket.as_bytes().as_slice()
            ],
        )?;
        tx.execute(
            "UPDATE player_identity_allocator
             SET next_player_id = MAX(next_player_id, ?1)
             WHERE singleton = 1",
            params![next_player_id_sql],
        )?;
        tx.execute(
            "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
             VALUES (?1, ?2)
             ON CONFLICT (node_id) DO UPDATE SET
               next_ship_counter = MAX(next_ship_counter, excluded.next_ship_counter)",
            params![
                ship_id.0.node_id().0 as i64,
                next_ship_counter_as_sql(ship_id)?
            ],
        )?;
        tx.commit()
    }

    pub(super) fn record_client_ownership_with_pending(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        pending_resume_ticket: Option<ResumeTicket>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let player_id_sql = player_id_as_sql(player_id)?;
        let next_player_id_sql = next_player_id_as_sql(player_id)?;
        let existing_player: Option<i64> = tx
            .query_row(
                "SELECT player_id FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if existing_player.is_some_and(|existing| existing != player_id_sql) {
            return Err(invariant_error(
                "a ShipId cannot be rebound to a different PlayerId",
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO consumed_player_ids (player_id) VALUES (?1)",
            params![player_id_sql],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO consumed_ship_ids (ship_id) VALUES (?1)",
            params![ship_id.raw().to_string()],
        )?;
        tx.execute(
            "INSERT INTO client_ship_ownership
             (ship_id, player_id, resume_ticket, pending_resume_ticket)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (ship_id) DO UPDATE SET
               player_id = excluded.player_id,
               resume_ticket = excluded.resume_ticket,
               pending_resume_ticket = excluded.pending_resume_ticket",
            params![
                ship_id.raw().to_string(),
                player_id_sql,
                resume_ticket.as_bytes().as_slice(),
                pending_resume_ticket.map(|ticket| ticket.as_bytes().to_vec()),
            ],
        )?;
        tx.execute(
            "UPDATE player_identity_allocator
             SET next_player_id = MAX(next_player_id, ?1)
             WHERE singleton = 1",
            params![next_player_id_sql],
        )?;
        tx.execute(
            "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
             VALUES (?1, ?2)
             ON CONFLICT (node_id) DO UPDATE SET
               next_ship_counter = MAX(next_ship_counter, excluded.next_ship_counter)",
            params![
                ship_id.0.node_id().0 as i64,
                next_ship_counter_as_sql(ship_id)?
            ],
        )?;
        tx.commit()
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
    /// The public `SimulationNode` read API delegates here directly.
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

    // `migrate_from_snapshot` (one-time import from the old
    // `StateSnapshot.station_inventories` field) is gone along with that field.
    // It was unreachable: postcard reads struct fields positionally, so a
    // snapshot written before the field existed fails to load outright rather
    // than arriving here with a populated map (ADR-0017 format compatibility).
}

impl SectorRepository {
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

    pub(super) fn projection_applied_through(&self) -> rusqlite::Result<u64> {
        self.conn.query_row(
            "SELECT projection_applied_through
             FROM station_projection_cursor WHERE singleton = 1",
            [],
            |row| non_negative_id(row.get(0)?, 0, "projection cursor"),
        )
    }

    /// Apply one Station mutation at a global journal position.
    ///
    /// The transition identity is checked before the mutation. Replaying the
    /// same identity is a no-op, while a gap is rejected so the cursor can only
    /// represent a contiguous global prefix. Non-Station transitions use the
    /// same method with `mutation = None`.
    pub(super) fn apply_station_projection(
        &self,
        transition_id: &str,
        journal_index: u64,
        mutation: Option<StationProjectionMutation>,
    ) -> Result<ProjectionApplyResult, ProjectionApplyError> {
        if transition_id.is_empty() {
            return Err(ProjectionApplyError::invalid(
                "projection transition identity must not be empty",
            ));
        }
        let journal_index_sql = i64::try_from(journal_index)
            .map_err(|_| ProjectionApplyError::invalid("journal index exceeds SQLite INTEGER"))?;
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|_| ProjectionApplyError::invalid("could not begin projection transaction"))?;
        let existing: Option<i64> = tx
            .query_row(
                "SELECT journal_index FROM station_projection_transitions
                 WHERE transition_id = ?1",
                params![transition_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| {
                ProjectionApplyError::invalid("could not read transition deduplication state")
            })?;
        if let Some(existing) = existing {
            let existing = u64::try_from(existing).map_err(|_| {
                ProjectionApplyError::invalid("stored transition index is negative")
            })?;
            if existing == journal_index {
                tx.rollback().ok();
                return Ok(ProjectionApplyResult::Duplicate);
            }
            tx.rollback().ok();
            return Err(ProjectionApplyError::DuplicateTransitionAtDifferentIndex {
                transition_id: transition_id.to_owned(),
                existing,
                actual: journal_index,
            });
        }
        let through: u64 = tx
            .query_row(
                "SELECT projection_applied_through
                 FROM station_projection_cursor WHERE singleton = 1",
                [],
                |row| non_negative_id(row.get(0)?, 0, "projection cursor"),
            )
            .map_err(|_| ProjectionApplyError::invalid("could not read projection cursor"))?;
        if journal_index != through {
            tx.rollback().ok();
            return Err(ProjectionApplyError::OutOfOrder {
                expected: through,
                actual: journal_index,
            });
        }
        if let Some(mutation) = mutation {
            if mutation.delta == 0 {
                tx.rollback().ok();
                return Err(ProjectionApplyError::invalid(
                    "station projection delta must not be zero",
                ));
            }
            let (item_type, module_id, ship_type_id) = item_id_to_columns(mutation.item_id);
            if mutation.delta > 0 {
                tx.execute(
                    "INSERT INTO station_inventory
                     (player_id, station_id, item_type, module_id, ship_type_id, count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (player_id, station_id, item_type, module_id, ship_type_id)
                     DO UPDATE SET count = count + excluded.count",
                    params![
                        mutation.player_id.0 as i64,
                        mutation.station_id.0 as i64,
                        item_type,
                        module_id,
                        ship_type_id,
                        mutation.delta,
                    ],
                )
                .map_err(|_| {
                    ProjectionApplyError::invalid("could not apply positive station delta")
                })?;
            } else {
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT count FROM station_inventory
                         WHERE player_id = ?1 AND station_id = ?2
                           AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                        params![
                            mutation.player_id.0 as i64,
                            mutation.station_id.0 as i64,
                            item_type,
                            module_id,
                            ship_type_id,
                        ],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|_| {
                        ProjectionApplyError::invalid("could not read station inventory stack")
                    })?;
                let Some(current) = current else {
                    tx.rollback().ok();
                    return Err(ProjectionApplyError::invalid(
                        "station inventory stack is missing",
                    ));
                };
                let remaining = current
                    .checked_add(mutation.delta)
                    .filter(|remaining| *remaining >= 0)
                    .ok_or_else(|| {
                        ProjectionApplyError::invalid(
                            "station projection delta underflows the stack",
                        )
                    })?;
                if remaining == 0 {
                    tx.execute(
                        "DELETE FROM station_inventory
                         WHERE player_id = ?1 AND station_id = ?2
                           AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                        params![
                            mutation.player_id.0 as i64,
                            mutation.station_id.0 as i64,
                            item_type,
                            module_id,
                            ship_type_id,
                        ],
                    )
                    .map_err(|_| {
                        ProjectionApplyError::invalid("could not delete station inventory stack")
                    })?;
                } else {
                    tx.execute(
                        "UPDATE station_inventory SET count = ?6
                         WHERE player_id = ?1 AND station_id = ?2
                           AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                        params![
                            mutation.player_id.0 as i64,
                            mutation.station_id.0 as i64,
                            item_type,
                            module_id,
                            ship_type_id,
                            remaining,
                        ],
                    )
                    .map_err(|_| {
                        ProjectionApplyError::invalid("could not update station inventory stack")
                    })?;
                }
            }
        }
        tx.execute(
            "INSERT INTO station_projection_transitions (transition_id, journal_index)
             VALUES (?1, ?2)",
            params![transition_id, journal_index_sql],
        )
        .map_err(|_| ProjectionApplyError::invalid("could not record projection transition"))?;
        let next = journal_index
            .checked_add(1)
            .ok_or_else(|| ProjectionApplyError::invalid("projection cursor overflowed"))?;
        let next_sql = i64::try_from(next).map_err(|_| {
            ProjectionApplyError::invalid("projection cursor exceeds SQLite INTEGER")
        })?;
        tx.execute(
            "UPDATE station_projection_cursor SET projection_applied_through = ?1
             WHERE singleton = 1",
            params![next_sql],
        )
        .map_err(|_| ProjectionApplyError::invalid("could not advance projection cursor"))?;
        tx.commit()
            .map_err(|_| ProjectionApplyError::invalid("could not commit station projection"))?;
        Ok(ProjectionApplyResult::Applied {
            projection_applied_through: next,
        })
    }
}

impl AdmissionRepository<'_> {
    pub(super) fn prepared_client_admission(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<PreparedClientAdmission>> {
        self.repository.prepared_client_admission(ship_id)
    }

    pub(super) fn prepared_client_admission_by_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<PreparedClientAdmission>> {
        self.repository
            .prepared_client_admission_by_ticket(resume_ticket)
    }
}

impl IdentityRepository<'_> {
    pub(super) fn client_owner(&self, ship_id: ShipId) -> rusqlite::Result<Option<PlayerId>> {
        self.repository.client_owner(ship_id)
    }

    pub(super) fn client_ownership_by_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<(PlayerId, ShipId)>> {
        self.repository.client_ownership_by_ticket(resume_ticket)
    }

    pub(super) fn client_resume_tickets(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<(ResumeTicket, Option<ResumeTicket>)>> {
        self.repository.client_resume_tickets(ship_id)
    }
}

impl StationInventoryRepository<'_> {
    pub(super) fn projection_applied_through(&self) -> rusqlite::Result<u64> {
        self.repository.projection_applied_through()
    }

    pub(super) fn apply_projection(
        &self,
        transition_id: &str,
        journal_index: u64,
        mutation: Option<StationProjectionMutation>,
    ) -> Result<ProjectionApplyResult, ProjectionApplyError> {
        self.repository
            .apply_station_projection(transition_id, journal_index, mutation)
    }

    pub(super) fn get_all(
        &self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> BTreeMap<ItemId, u64> {
        self.repository.get_all(player_id, station_id)
    }

    pub(super) fn credit(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) {
        self.repository
            .credit(player_id, station_id, item_id, count);
    }

    pub(super) fn try_debit(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), StationOperationRejection> {
        self.repository
            .try_debit(player_id, station_id, item_id, count)
    }
}

impl SectorTransaction<'_> {
    /// Commit an admission grant across the admission, identity, and Station
    /// repositories as one local transaction.
    pub(super) fn ensure_client_admission_grant(
        self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> rusqlite::Result<bool> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let player_id_sql = player_id_as_sql(player_id)?;
        let next_player_id_sql = next_player_id_as_sql(player_id)?;
        let count = i64::try_from(count).map_err(|_| invariant_error("item count exceeds i64"))?;
        let inserted = self.transaction.execute(
            "INSERT OR IGNORE INTO client_admission_grants
             (ship_id, player_id, station_id, item_type, module_id, ship_type_id, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ship_id.raw().to_string(),
                player_id_sql,
                station_id.0 as i64,
                item_type,
                module_id,
                ship_type_id,
                count,
            ],
        )?;
        if inserted == 0 {
            let existing = self.transaction.query_row(
                "SELECT player_id, station_id, item_type, module_id, ship_type_id, count
                 FROM client_admission_grants WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )?;
            if existing
                != (
                    player_id_sql,
                    station_id.0 as i64,
                    item_type.to_string(),
                    module_id as i64,
                    ship_type_id as i64,
                    count,
                )
            {
                return Err(invariant_error(
                    "admission grant identity does not match the existing grant",
                ));
            }
        } else {
            self.transaction.execute(
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
                    count,
                ],
            )?;
        }
        let existing_owner: Option<i64> = self
            .transaction
            .query_row(
                "SELECT player_id FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if existing_owner.is_some_and(|owner| owner != player_id_sql) {
            return Err(invariant_error(
                "admission grant owner does not match existing ownership",
            ));
        }
        self.transaction.execute(
            "INSERT OR IGNORE INTO consumed_player_ids (player_id) VALUES (?1)",
            params![player_id.0 as i64],
        )?;
        self.transaction.execute(
            "INSERT OR IGNORE INTO consumed_ship_ids (ship_id) VALUES (?1)",
            params![ship_id.raw().to_string()],
        )?;
        self.transaction.execute(
            "UPDATE player_identity_allocator
             SET next_player_id = MAX(next_player_id, ?1)
             WHERE singleton = 1",
            params![next_player_id_sql],
        )?;
        self.transaction.execute(
            "INSERT INTO ship_identity_allocator (node_id, next_ship_counter)
             VALUES (?1, ?2)
             ON CONFLICT (node_id) DO UPDATE SET
               next_ship_counter = MAX(next_ship_counter, excluded.next_ship_counter)",
            params![
                ship_id.0.node_id().0 as i64,
                next_ship_counter_as_sql(ship_id)?
            ],
        )?;
        self.transaction.execute(
            "INSERT INTO client_ship_ownership (ship_id, player_id, resume_ticket)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (ship_id) DO NOTHING",
            params![
                ship_id.raw().to_string(),
                player_id_sql,
                resume_ticket.as_bytes().as_slice()
            ],
        )?;
        self.transaction.execute(
            "DELETE FROM client_admission_prepared WHERE ship_id = ?1",
            params![ship_id.raw().to_string()],
        )?;
        self.transaction.commit()?;
        Ok(inserted == 1)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn ensure_grant(
        db: &mut SectorRepository,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> bool {
        db.transaction()
            .unwrap()
            .ensure_client_admission_grant(
                ship_id,
                player_id,
                resume_ticket,
                station_id,
                item_id,
                count,
            )
            .unwrap()
    }

    #[test]
    fn credit_then_get_all_round_trips() {
        let db = SectorRepository::open_in_memory().unwrap();
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
        let db = SectorRepository::open_in_memory().unwrap();
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
        let mut db = SectorRepository::open_in_memory().unwrap();
        let spawn = Position::new(10.0, 20.0, 30.0);
        let resume_ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        let (player_id, ship_id) = db
            .reserve_fresh_admission_identity(dawn_core::NodeId(2), spawn, resume_ticket)
            .unwrap();
        assert_eq!(
            db.admissions().prepared_client_admission(ship_id).unwrap(),
            Some(PreparedClientAdmission {
                ship_id,
                player_id,
                spawn_position: spawn,
                resume_ticket,
            })
        );

        let item = ItemId::PackagedShip(ShipTypeId(7));
        db.transaction()
            .unwrap()
            .ensure_client_admission_grant(ship_id, player_id, resume_ticket, StationId(7), item, 1)
            .unwrap();
        assert_eq!(
            db.admissions().prepared_client_admission(ship_id).unwrap(),
            None
        );
        assert_eq!(
            db.identities().client_owner(ship_id).unwrap(),
            Some(player_id)
        );
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
        let mut db = SectorRepository::init(conn).unwrap();
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
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 7);
        let item = ItemId::PackagedShip(ShipTypeId(7));
        let resume_ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        assert!(ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            resume_ticket,
            StationId(7),
            item,
            1,
        ));
        assert!(!ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            resume_ticket,
            StationId(7),
            item,
            1,
        ));
        assert_eq!(db.get_all(PlayerId(1), StationId(7)).get(&item), Some(&1));
    }

    #[test]
    fn admission_grant_reconciliation_does_not_regrant_a_consumed_item() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 14);
        let item = ItemId::ScrapMetal;
        let ticket = ResumeTicket::from_bytes([29; ResumeTicket::BYTE_LEN]);
        assert!(ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            ticket,
            StationId(7),
            item,
            1,
        ));
        assert!(db.try_debit(PlayerId(1), StationId(7), item, 1).is_ok());

        db.reconcile_admission_identity_watermarks().unwrap();
        assert!(db.get_all(PlayerId(1), StationId(7)).is_empty());
    }

    #[test]
    fn admission_grant_reconciliation_rejects_a_missing_owner_binding() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 16);
        let ticket = ResumeTicket::from_bytes([32; ResumeTicket::BYTE_LEN]);
        assert!(ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            ticket,
            StationId(7),
            ItemId::ScrapMetal,
            1,
        ));
        db.conn
            .execute(
                "DELETE FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
            )
            .unwrap();

        assert!(db.reconcile_admission_identity_watermarks().is_err());
    }

    #[test]
    fn admission_grant_finalization_preserves_rotated_resume_tickets() {
        let mut db = SectorRepository::open_in_memory().unwrap();
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

        ensure_grant(
            &mut db,
            ship_id,
            player_id,
            original_ticket,
            StationId(7),
            ItemId::PackagedShip(ShipTypeId(7)),
            1,
        );

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
    fn fresh_identity_reservation_advances_allocator_across_restart() {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let db_path = db_file.path().to_str().unwrap();
        let spawn = Position::ORIGIN;
        let first_ticket = ResumeTicket::from_bytes([21; ResumeTicket::BYTE_LEN]);
        let (first_player, first_ship) = SectorRepository::open(db_path)
            .unwrap()
            .reserve_fresh_admission_identity(dawn_core::NodeId(4), spawn, first_ticket)
            .unwrap();
        let second_ticket = ResumeTicket::from_bytes([22; ResumeTicket::BYTE_LEN]);
        let (second_player, second_ship) = SectorRepository::open(db_path)
            .unwrap()
            .reserve_fresh_admission_identity(dawn_core::NodeId(4), spawn, second_ticket)
            .unwrap();

        assert!(second_player.0 > first_player.0);
        assert!(second_ship.0.counter() > first_ship.0.counter());
    }

    #[test]
    fn opening_an_existing_repository_rebuilds_allocator_watermarks() {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let db_path = db_file.path().to_str().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(4), 77);
        let ticket = ResumeTicket::from_bytes([23; ResumeTicket::BYTE_LEN]);
        {
            let db = SectorRepository::open(db_path).unwrap();
            db.conn
                .execute(
                    "INSERT INTO client_ship_ownership
                     (ship_id, player_id, resume_ticket)
                     VALUES (?1, ?2, ?3)",
                    params![ship_id.raw().to_string(), 99_i64, ticket.as_bytes()],
                )
                .unwrap();
        }

        let (player_id, next_ship_id) = SectorRepository::open(db_path)
            .unwrap()
            .reserve_fresh_admission_identity(
                dawn_core::NodeId(4),
                Position::ORIGIN,
                ResumeTicket::from_bytes([24; ResumeTicket::BYTE_LEN]),
            )
            .unwrap();
        assert_eq!(player_id, PlayerId(100));
        assert_eq!(next_ship_id, ShipId::new(dawn_core::NodeId(4), 78));
    }

    #[test]
    fn observing_materialized_ids_keeps_allocators_above_snapshot_state() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.observe_materialized_identities([ShipId::new(dawn_core::NodeId(5), 42)], [PlayerId(17)])
            .unwrap();

        let (player_id, ship_id) = db
            .reserve_fresh_admission_identity(
                dawn_core::NodeId(5),
                Position::ORIGIN,
                ResumeTicket::from_bytes([25; ResumeTicket::BYTE_LEN]),
            )
            .unwrap();
        assert_eq!(player_id, PlayerId(18));
        assert_eq!(ship_id, ShipId::new(dawn_core::NodeId(5), 43));
    }

    #[test]
    fn ownership_cannot_be_rebound_to_another_player() {
        let db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 12);
        db.record_client_ownership(
            ship_id,
            PlayerId(1),
            ResumeTicket::from_bytes([26; ResumeTicket::BYTE_LEN]),
        )
        .unwrap();

        assert!(db
            .record_client_ownership(
                ship_id,
                PlayerId(2),
                ResumeTicket::from_bytes([27; ResumeTicket::BYTE_LEN]),
            )
            .is_err());
        assert_eq!(db.client_owner(ship_id).unwrap(), Some(PlayerId(1)));
    }

    #[test]
    fn admission_grant_rejects_a_different_duplicate_payload() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 13);
        let ticket = ResumeTicket::from_bytes([28; ResumeTicket::BYTE_LEN]);
        assert!(ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            ticket,
            StationId(7),
            ItemId::ScrapMetal,
            1,
        ));
        assert!(db
            .transaction()
            .unwrap()
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                ticket,
                StationId(7),
                ItemId::ScrapMetal,
                2,
            )
            .is_err());
        assert_eq!(
            db.get_all(PlayerId(1), StationId(7))
                .get(&ItemId::ScrapMetal),
            Some(&1)
        );
    }

    #[test]
    fn admission_grant_rejects_a_different_existing_owner() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 15);
        let existing_ticket = ResumeTicket::from_bytes([30; ResumeTicket::BYTE_LEN]);
        let grant_ticket = ResumeTicket::from_bytes([31; ResumeTicket::BYTE_LEN]);
        db.record_client_ownership(ship_id, PlayerId(2), existing_ticket)
            .unwrap();

        assert!(db
            .transaction()
            .unwrap()
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                grant_ticket,
                StationId(7),
                ItemId::ScrapMetal,
                1,
            )
            .is_err());
        assert!(db.get_all(PlayerId(1), StationId(7)).is_empty());
        assert_eq!(db.client_owner(ship_id).unwrap(), Some(PlayerId(2)));
    }

    #[test]
    fn station_projection_is_contiguous_and_idempotent() {
        let db = SectorRepository::open_in_memory().unwrap();
        let mutation = StationProjectionMutation {
            player_id: PlayerId(8),
            station_id: StationId(3),
            item_id: ItemId::ScrapMetal,
            delta: 4,
        };

        assert_eq!(
            db.station_inventory()
                .apply_projection("transition-0", 0, Some(mutation)),
            Ok(ProjectionApplyResult::Applied {
                projection_applied_through: 1
            })
        );
        assert_eq!(
            db.station_inventory()
                .apply_projection("transition-0", 0, Some(mutation)),
            Ok(ProjectionApplyResult::Duplicate)
        );
        assert_eq!(
            db.station_inventory().projection_applied_through().unwrap(),
            1
        );
        assert_eq!(
            db.station_inventory()
                .apply_projection("transition-2", 2, None),
            Err(ProjectionApplyError::OutOfOrder {
                expected: 1,
                actual: 2
            })
        );
        assert_eq!(
            db.station_inventory()
                .apply_projection("transition-1", 1, None),
            Ok(ProjectionApplyResult::Applied {
                projection_applied_through: 2
            })
        );
        assert_eq!(
            db.station_inventory()
                .get_all(PlayerId(8), StationId(3))
                .get(&ItemId::ScrapMetal),
            Some(&4)
        );
    }

    #[test]
    fn station_projection_rejects_journal_indices_outside_sqlite_integer_range() {
        let db = SectorRepository::open_in_memory().unwrap();

        assert!(matches!(
            db.station_inventory().apply_projection("", 0, None),
            Err(ProjectionApplyError::InvalidDelta { .. })
        ));
        assert!(matches!(
            db.station_inventory()
                .apply_projection("too-large", u64::MAX, None),
            Err(ProjectionApplyError::InvalidDelta { .. })
        ));
    }

    #[test]
    fn try_debit_rejects_a_missing_stack() {
        let db = SectorRepository::open_in_memory().unwrap();
        assert_eq!(
            db.try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 1),
            Err(StationOperationRejection::MissingStationItem)
        );
    }

    #[test]
    fn try_debit_rejects_an_insufficient_stack() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert_eq!(
            db.try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 3),
            Err(StationOperationRejection::InsufficientStationItem)
        );
    }

    #[test]
    fn try_debit_removes_the_row_once_it_hits_zero() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert!(db
            .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2)
            .is_ok());
        assert!(db.get_all(PlayerId(1), StationId(7)).is_empty());
    }

    #[test]
    fn try_debit_partial_leaves_the_remainder() {
        let db = SectorRepository::open_in_memory().unwrap();
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
