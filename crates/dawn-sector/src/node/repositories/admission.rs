//! Prepared client admission and post-projection grant finalization.

use dawn_core::{ItemId, PlayerId, Position, ResumeTicket, ShipId, StationId};
use rusqlite::{params, OptionalExtension};

use super::{
    invariant_error, next_player_id_as_sql, next_ship_counter_as_sql, non_negative_id,
    player_id_as_sql, ship_id_from_text, ticket_from_blob, SectorRepository, SectorTransaction,
};

pub struct AdmissionRepository<'a> {
    pub(super) repository: &'a SectorRepository,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::node) struct PreparedClientAdmission {
    pub(in crate::node) ship_id: ShipId,
    pub(in crate::node) player_id: PlayerId,
    pub(in crate::node) spawn_position: Position,
    pub(in crate::node) resume_ticket: ResumeTicket,
}

impl AdmissionRepository<'_> {
    pub(in crate::node) fn prepared_client_admission(
        &self,
        ship_id: ShipId,
    ) -> rusqlite::Result<Option<PreparedClientAdmission>> {
        self.repository
            .conn
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

    pub(in crate::node) fn prepared_client_admission_by_ticket(
        &self,
        resume_ticket: ResumeTicket,
    ) -> rusqlite::Result<Option<PreparedClientAdmission>> {
        self.repository
            .conn
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
}

impl SectorTransaction<'_> {
    /// Finalize an admission grant after its starter Station mutation has
    /// projected through the durable runtime frame.
    pub(in crate::node) fn ensure_client_admission_grant(
        self,
        ship_id: ShipId,
        player_id: PlayerId,
        resume_ticket: ResumeTicket,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> rusqlite::Result<bool> {
        let (item_type, module_id, ship_type_id) = super::item_id_to_columns(item_id);
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
    use crate::node::repositories::SectorRepository;
    use dawn_core::ShipTypeId;

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
    fn prepared_admission_and_ownership_round_trip() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let spawn = Position::new(1.0, 2.0, 3.0);
        let resume_ticket = ResumeTicket::from_bytes([4; ResumeTicket::BYTE_LEN]);
        let (player_id, ship_id) = db
            .identities()
            .reserve_fresh_admission_identity(dawn_core::NodeId(2), spawn, resume_ticket)
            .unwrap();
        assert_eq!(
            db.admissions().prepared_client_admission(ship_id).unwrap(),
            Some(PreparedClientAdmission {
                ship_id,
                player_id,
                spawn_position: spawn,
                resume_ticket
            })
        );
        let item = ItemId::PackagedShip(ShipTypeId(7));
        ensure_grant(
            &mut db,
            ship_id,
            player_id,
            resume_ticket,
            StationId(7),
            item,
            1,
        );
        assert!(db
            .admissions()
            .prepared_client_admission(ship_id)
            .unwrap()
            .is_none());
        assert_eq!(
            db.identities().client_owner(ship_id).unwrap(),
            Some(player_id)
        );
    }

    #[test]
    fn client_admission_grant_is_idempotent() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 7);
        let item = ItemId::PackagedShip(ShipTypeId(7));
        let ticket = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        assert!(ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            ticket,
            StationId(7),
            item,
            1
        ));
        assert!(!ensure_grant(
            &mut db,
            ship_id,
            PlayerId(1),
            ticket,
            StationId(7),
            item,
            1
        ));
        assert!(db
            .station_inventory()
            .get_all(PlayerId(1), StationId(7))
            .unwrap()
            .is_empty());
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
            1
        ));
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), item, 1);
        assert!(db
            .station_inventory()
            .try_debit(PlayerId(1), StationId(7), item, 1)
            .is_ok());
        db.identities()
            .reconcile_admission_identity_watermarks()
            .unwrap();
        assert!(db
            .station_inventory()
            .get_all(PlayerId(1), StationId(7))
            .unwrap()
            .is_empty());
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
            1
        ));
        db.conn
            .execute(
                "DELETE FROM client_ship_ownership WHERE ship_id = ?1",
                params![ship_id.raw().to_string()],
            )
            .unwrap();
        assert!(db
            .identities()
            .reconcile_admission_identity_watermarks()
            .is_err());
    }

    #[test]
    fn admission_grant_finalization_preserves_rotated_resume_tickets() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 9);
        let player_id = PlayerId(1);
        let original = ResumeTicket::from_bytes([7; ResumeTicket::BYTE_LEN]);
        let current = ResumeTicket::from_bytes([8; ResumeTicket::BYTE_LEN]);
        let pending = ResumeTicket::from_bytes([9; ResumeTicket::BYTE_LEN]);
        db.identities()
            .record_client_ownership(ship_id, player_id, current)
            .unwrap();
        db.identities()
            .stage_client_resume_ticket(ship_id, player_id, current, pending)
            .unwrap();
        ensure_grant(
            &mut db,
            ship_id,
            player_id,
            original,
            StationId(7),
            ItemId::PackagedShip(ShipTypeId(7)),
            1,
        );
        assert_eq!(
            db.identities().client_resume_tickets(ship_id).unwrap(),
            Some((current, Some(pending)))
        );
        assert_eq!(
            db.identities()
                .client_ownership_by_ticket(original)
                .unwrap(),
            None
        );
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
            1
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
                2
            )
            .is_err());
        assert!(db
            .station_inventory()
            .get_all(PlayerId(1), StationId(7))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn admission_grant_rejects_a_different_existing_owner() {
        let mut db = SectorRepository::open_in_memory().unwrap();
        let ship_id = ShipId::new(dawn_core::NodeId(2), 15);
        db.identities()
            .record_client_ownership(
                ship_id,
                PlayerId(2),
                ResumeTicket::from_bytes([30; ResumeTicket::BYTE_LEN]),
            )
            .unwrap();
        assert!(db
            .transaction()
            .unwrap()
            .ensure_client_admission_grant(
                ship_id,
                PlayerId(1),
                ResumeTicket::from_bytes([31; ResumeTicket::BYTE_LEN]),
                StationId(7),
                ItemId::ScrapMetal,
                1
            )
            .is_err());
        assert!(db
            .station_inventory()
            .get_all(PlayerId(1), StationId(7))
            .unwrap()
            .is_empty());
        assert_eq!(
            db.identities().client_owner(ship_id).unwrap(),
            Some(PlayerId(2))
        );
    }
}
