//! Player/ship identity, allocator, ownership, and ResumeTicket persistence.

use std::collections::BTreeMap;

use dawn_core::{PlayerId, Position, ResumeTicket, ShipId};
use rusqlite::{params, OptionalExtension};

use super::{
    columns_to_item_id, invariant_error, next_player_id_as_sql, next_ship_counter_as_sql,
    non_negative_id, player_id_as_sql, ship_id_from_text, ticket_from_blob, SectorRepository,
};

pub struct IdentityRepository<'a> {
    pub(super) repository: &'a SectorRepository,
}

impl IdentityRepository<'_> {
    /// Rebuild durable identity indexes after opening an existing repository.
    /// Allocator tables are derived from all materialized protocol rows.
    pub(super) fn rebuild_identity_watermarks(&self) -> rusqlite::Result<()> {
        let tx = self.repository.conn.unchecked_transaction()?;
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
    pub(in crate::node) fn observe_materialized_identities(
        &self,
        ship_ids: impl IntoIterator<Item = ShipId>,
        player_ids: impl IntoIterator<Item = PlayerId>,
    ) -> rusqlite::Result<()> {
        let tx = self.repository.conn.unchecked_transaction()?;
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
    pub(in crate::node) fn reconcile_admission_identity_watermarks(&self) -> rusqlite::Result<()> {
        let tx = self.repository.conn.unchecked_transaction()?;
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
    /// the caller can expose a Welcome frame.
    pub(in crate::node) fn reserve_fresh_admission_identity(
        &self,
        node_id: dawn_core::NodeId,
        spawn_position: Position,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<(PlayerId, ShipId)> {
        let tx = self.repository.conn.unchecked_transaction()?;
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

    pub(in crate::node) fn client_owner(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<PlayerId>> {
        self.repository
            .conn
            .query_row(
                "SELECT player_id FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
                |row| Ok(PlayerId(non_negative_id(row.get(0)?, 0, "PlayerId")?)),
            )
            .optional()
    }

    pub(in crate::node) fn client_ownership_by_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<(PlayerId, ShipId)>> {
        self.repository
            .conn
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

    pub(in crate::node) fn client_resume_tickets(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<(ResumeTicket, Option<ResumeTicket>)>> {
        self.repository
            .conn
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

    pub(in crate::node) fn record_client_ownership(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<()> {
        let tx = self.repository.conn.unchecked_transaction()?;
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

    pub(in crate::node) fn record_client_ownership_with_pending(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        pending_resume_ticket: Option<ResumeTicket>,
    ) -> rusqlite::Result<()> {
        let tx = self.repository.conn.unchecked_transaction()?;
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
}

impl IdentityRepository<'_> {
    pub(in crate::node) fn stage_client_resume_ticket(
        &self,
        ship_id: ShipId,
        player_id: PlayerId,
        presented_ticket: ResumeTicket,
        proposed_next_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<ResumeTicket>> {
        let tx = self.repository.conn.unchecked_transaction()?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::repositories::SectorRepository;

    #[test]
    fn existing_ownership_schema_can_stage_and_promote_a_resume_ticket() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE client_ship_ownership (
                ship_id TEXT PRIMARY KEY,
                player_id INTEGER NOT NULL,
                resume_ticket BLOB NOT NULL UNIQUE
            )",
            [],
        )
        .unwrap();
        let db = SectorRepository::init(conn).unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 8);
        let player_id = PlayerId(1);
        let current_ticket = ResumeTicket::from_bytes([8; ResumeTicket::BYTE_LEN]);
        let next_ticket = ResumeTicket::from_bytes([9; ResumeTicket::BYTE_LEN]);

        db.identities()
            .record_client_ownership(ship_id, player_id, current_ticket)
            .unwrap();
        assert_eq!(
            db.identities()
                .stage_client_resume_ticket(ship_id, player_id, current_ticket, next_ticket)
                .unwrap(),
            Some(next_ticket)
        );
        assert_eq!(
            db.identities()
                .client_ownership_by_ticket(next_ticket)
                .unwrap(),
            Some((player_id, ship_id))
        );

        db.identities()
            .record_client_ownership(ship_id, player_id, next_ticket)
            .unwrap();
        assert_eq!(
            db.identities()
                .client_ownership_by_ticket(current_ticket)
                .unwrap(),
            None
        );
        assert_eq!(
            db.identities()
                .client_ownership_by_ticket(next_ticket)
                .unwrap(),
            Some((player_id, ship_id))
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
            .identities()
            .reserve_fresh_admission_identity(dawn_core::NodeId(4), spawn, first_ticket)
            .unwrap();
        let second_ticket = ResumeTicket::from_bytes([22; ResumeTicket::BYTE_LEN]);
        let (second_player, second_ship) = SectorRepository::open(db_path)
            .unwrap()
            .identities()
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
            .identities()
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
        db.identities()
            .observe_materialized_identities(
                [ShipId::new(dawn_core::NodeId(5), 42)],
                [PlayerId(17)],
            )
            .unwrap();

        let (player_id, ship_id) = db
            .identities()
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
        db.identities()
            .record_client_ownership(
                ship_id,
                PlayerId(1),
                ResumeTicket::from_bytes([26; ResumeTicket::BYTE_LEN]),
            )
            .unwrap();

        assert!(db
            .identities()
            .record_client_ownership(
                ship_id,
                PlayerId(2),
                ResumeTicket::from_bytes([27; ResumeTicket::BYTE_LEN]),
            )
            .is_err());
        assert_eq!(
            db.identities().client_owner(ship_id).unwrap(),
            Some(PlayerId(1))
        );
    }
}
