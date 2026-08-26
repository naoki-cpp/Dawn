//! Station inventory SQLite projection and its contiguous journal cursor.

use std::collections::BTreeMap;

use dawn_core::{ItemId, PlayerId, StationId};
use dawn_storage::JournalRange;
use rusqlite::{params, OptionalExtension};

use super::{columns_to_item_id, item_id_to_columns, non_negative_id, SectorRepository};
use crate::transition::StationProjectionMutation;

pub struct StationInventoryRepository<'a> {
    pub(super) repository: &'a SectorRepository,
}

/// Result of applying one Station projection transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionApplyResult {
    /// The transition was applied and the cursor now covers this position.
    Applied { projection_applied_through: u64 },
    /// The same transition identity and journal position were already applied.
    Duplicate,
}

/// Failure while applying a committed Station projection transition.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionApplyError {
    /// The caller skipped a global journal boundary.
    #[error(
        "station projection is out of order: expected journal boundary {expected}, got {actual}"
    )]
    OutOfOrder { expected: u64, actual: u64 },
    /// A stable transition identity was reused with another journal range.
    #[error("station projection transition {transition_id} was already recorded at journal range starting at {existing} (length {existing_len}), not {actual} (length {actual_len})")]
    DuplicateTransitionAtDifferentIndex {
        transition_id: String,
        existing: u64,
        actual: u64,
        existing_len: u32,
        actual_len: u32,
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

impl StationInventoryRepository<'_> {
    pub(in crate::node) fn get_all(
        &self,
        player_id: PlayerId,
        station_id: StationId,
    ) -> BTreeMap<ItemId, u64> {
        let mut stmt = self
            .repository
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

    #[cfg(test)]
    pub(in crate::node) fn credit(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        self.repository.conn
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

    #[cfg(test)]
    pub(in crate::node) fn try_debit(
        &self,
        player_id: PlayerId,
        station_id: StationId,
        item_id: ItemId,
        count: u64,
    ) -> Result<(), crate::node::station::StationOperationRejection> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let current: Option<i64> = self
            .repository.conn
            .query_row(
                "SELECT count FROM station_inventory
                 WHERE player_id = ?1 AND station_id = ?2 AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                params![player_id.0 as i64, station_id.0 as i64, item_type, module_id, ship_type_id],
                |row| row.get(0),
            )
            .optional()
            .expect("query is well-formed");

        let Some(current) = current else {
            return Err(crate::node::station::StationOperationRejection::MissingStationItem);
        };
        if current < count as i64 {
            return Err(crate::node::station::StationOperationRejection::InsufficientStationItem);
        }
        let remaining = current - count as i64;
        if remaining == 0 {
            self.repository.conn
                .execute(
                    "DELETE FROM station_inventory
                     WHERE player_id = ?1 AND station_id = ?2 AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                    params![player_id.0 as i64, station_id.0 as i64, item_type, module_id, ship_type_id],
                )
                .expect("station_inventory delete");
        } else {
            self.repository.conn
                .execute(
                    "UPDATE station_inventory SET count = ?6
                     WHERE player_id = ?1 AND station_id = ?2 AND item_type = ?3 AND module_id = ?4 AND ship_type_id = ?5",
                    params![player_id.0 as i64, station_id.0 as i64, item_type, module_id, ship_type_id, remaining],
                )
                .expect("station_inventory update");
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::node) fn projection_applied_through(&self) -> rusqlite::Result<u64> {
        self.repository.conn.query_row(
            "SELECT projection_applied_through
             FROM station_projection_cursor WHERE singleton = 1",
            [],
            |row| non_negative_id(row.get(0)?, 0, "projection cursor"),
        )
    }

    pub(in crate::node) fn apply_projection(
        &self,
        transition_id: &str,
        range: JournalRange,
        mutations: &[StationProjectionMutation],
    ) -> Result<ProjectionApplyResult, ProjectionApplyError> {
        if transition_id.is_empty() {
            return Err(ProjectionApplyError::invalid(
                "projection transition identity must not be empty",
            ));
        }
        if range.len == 0 {
            return Err(ProjectionApplyError::invalid(
                "projection journal range must not be empty",
            ));
        }
        let journal_index = range.first.0;
        let journal_index_sql = i64::try_from(journal_index)
            .map_err(|_| ProjectionApplyError::invalid("journal index exceeds SQLite INTEGER"))?;
        let journal_len_sql = i64::from(range.len);
        let tx =
            self.repository.conn.unchecked_transaction().map_err(|_| {
                ProjectionApplyError::invalid("could not begin projection transaction")
            })?;
        let existing: Option<(i64, i64)> = tx
            .query_row(
                "SELECT journal_index, journal_len FROM station_projection_transitions
                 WHERE transition_id = ?1",
                params![transition_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| {
                ProjectionApplyError::invalid("could not read transition deduplication state")
            })?;
        if let Some((existing, existing_len)) = existing {
            let existing = u64::try_from(existing).map_err(|_| {
                ProjectionApplyError::invalid("stored transition index is negative")
            })?;
            let existing_len = u32::try_from(existing_len).map_err(|_| {
                ProjectionApplyError::invalid("stored transition range length is invalid")
            })?;
            if existing == journal_index && existing_len == range.len {
                tx.rollback().ok();
                return Ok(ProjectionApplyResult::Duplicate);
            }
            tx.rollback().ok();
            return Err(ProjectionApplyError::DuplicateTransitionAtDifferentIndex {
                transition_id: transition_id.to_owned(),
                existing,
                actual: journal_index,
                existing_len,
                actual_len: range.len,
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
        for mutation in mutations {
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
                        mutation.delta
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
                            ship_type_id
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
                            ship_type_id
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
                            remaining
                        ],
                    )
                    .map_err(|_| {
                        ProjectionApplyError::invalid("could not update station inventory stack")
                    })?;
                }
            }
        }
        tx.execute(
            "INSERT INTO station_projection_transitions
             (transition_id, journal_index, journal_len) VALUES (?1, ?2, ?3)",
            params![transition_id, journal_index_sql, journal_len_sql],
        )
        .map_err(|_| ProjectionApplyError::invalid("could not record projection transition"))?;
        let next = range
            .checked_last_exclusive()
            .map(|index| index.0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::repositories::SectorRepository;

    #[test]
    fn credit_then_get_all_round_trips() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 5);
        db.station_inventory().credit(
            PlayerId(1),
            StationId(7),
            ItemId::Module(dawn_core::ModuleId(3)),
            2,
        );
        db.station_inventory().credit(
            PlayerId(1),
            StationId(7),
            ItemId::PackagedShip(dawn_core::ShipTypeId(4)),
            1,
        );
        let inventory = db.station_inventory().get_all(PlayerId(1), StationId(7));
        assert_eq!(inventory.get(&ItemId::ScrapMetal), Some(&5));
        assert_eq!(
            inventory.get(&ItemId::Module(dawn_core::ModuleId(3))),
            Some(&2)
        );
        assert_eq!(
            inventory.get(&ItemId::PackagedShip(dawn_core::ShipTypeId(4))),
            Some(&1)
        );
    }

    #[test]
    fn credit_accumulates_across_calls() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 3);
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert_eq!(
            db.station_inventory()
                .get_all(PlayerId(1), StationId(7))
                .get(&ItemId::ScrapMetal),
            Some(&5)
        );
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
            db.station_inventory().apply_projection(
                "transition-0",
                JournalRange {
                    first: dawn_storage::JournalIndex(0),
                    len: 2
                },
                &[mutation]
            ),
            Ok(ProjectionApplyResult::Applied {
                projection_applied_through: 2
            })
        );
        assert_eq!(
            db.station_inventory().apply_projection(
                "transition-0",
                JournalRange {
                    first: dawn_storage::JournalIndex(0),
                    len: 2
                },
                &[mutation]
            ),
            Ok(ProjectionApplyResult::Duplicate)
        );
        assert_eq!(
            db.station_inventory().projection_applied_through().unwrap(),
            2
        );
        assert_eq!(
            db.station_inventory().apply_projection(
                "transition-2",
                JournalRange {
                    first: dawn_storage::JournalIndex(3),
                    len: 1
                },
                &[]
            ),
            Err(ProjectionApplyError::OutOfOrder {
                expected: 2,
                actual: 3
            })
        );
        assert_eq!(
            db.station_inventory().apply_projection(
                "transition-1",
                JournalRange {
                    first: dawn_storage::JournalIndex(2),
                    len: 1
                },
                &[]
            ),
            Ok(ProjectionApplyResult::Applied {
                projection_applied_through: 3
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
    fn repository_init_migrates_legacy_station_projection_ranges() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE station_projection_transitions (transition_id TEXT PRIMARY KEY, journal_index INTEGER NOT NULL UNIQUE CHECK (journal_index >= 0))", []).unwrap();
        conn.execute("INSERT INTO station_projection_transitions (transition_id, journal_index) VALUES ('legacy', 0)", []).unwrap();
        conn.execute("CREATE TABLE station_projection_cursor (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), projection_applied_through INTEGER NOT NULL CHECK (projection_applied_through >= 0))", []).unwrap();
        conn.execute("INSERT INTO station_projection_cursor (singleton, projection_applied_through) VALUES (1, 1)", []).unwrap();
        let db = SectorRepository::init(conn).unwrap();
        assert_eq!(db.conn.query_row("SELECT journal_len FROM station_projection_transitions WHERE transition_id = 'legacy'", [], |row| row.get::<_, u32>(0)).unwrap(), 1);
        assert_eq!(
            db.station_inventory().apply_projection(
                "production-range",
                JournalRange {
                    first: dawn_storage::JournalIndex(1),
                    len: 2
                },
                &[]
            ),
            Ok(ProjectionApplyResult::Applied {
                projection_applied_through: 3
            })
        );
    }

    #[test]
    fn station_projection_rejects_journal_indices_outside_sqlite_integer_range() {
        let db = SectorRepository::open_in_memory().unwrap();
        assert!(matches!(
            db.station_inventory().apply_projection(
                "",
                JournalRange {
                    first: dawn_storage::JournalIndex(0),
                    len: 1
                },
                &[]
            ),
            Err(ProjectionApplyError::InvalidDelta { .. })
        ));
        assert!(matches!(
            db.station_inventory().apply_projection(
                "too-large",
                JournalRange {
                    first: dawn_storage::JournalIndex(u64::MAX),
                    len: 1
                },
                &[]
            ),
            Err(ProjectionApplyError::InvalidDelta { .. })
        ));
    }

    #[test]
    fn station_projection_failure_keeps_rows_and_cursor_atomic() {
        let db = SectorRepository::open_in_memory().unwrap();
        let item = ItemId::ScrapMetal;
        let credit = StationProjectionMutation {
            player_id: PlayerId(1),
            station_id: StationId(7),
            item_id: item,
            delta: 1,
        };
        db.station_inventory()
            .apply_projection(
                "credit",
                JournalRange {
                    first: dawn_storage::JournalIndex(0),
                    len: 1,
                },
                &[credit],
            )
            .unwrap();
        let debit = StationProjectionMutation {
            delta: -2,
            ..credit
        };
        assert!(matches!(
            db.station_inventory().apply_projection(
                "invalid-debit",
                JournalRange {
                    first: dawn_storage::JournalIndex(1),
                    len: 1
                },
                &[debit]
            ),
            Err(ProjectionApplyError::InvalidDelta { .. })
        ));
        assert_eq!(
            db.station_inventory().projection_applied_through().unwrap(),
            1
        );
        assert_eq!(
            db.station_inventory()
                .get_all(PlayerId(1), StationId(7))
                .get(&item),
            Some(&1)
        );
    }

    #[test]
    fn try_debit_rejects_a_missing_stack() {
        let db = SectorRepository::open_in_memory().unwrap();
        assert_eq!(
            db.station_inventory()
                .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 1),
            Err(crate::node::station::StationOperationRejection::MissingStationItem)
        );
    }

    #[test]
    fn try_debit_rejects_an_insufficient_stack() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert_eq!(
            db.station_inventory()
                .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 3),
            Err(crate::node::station::StationOperationRejection::InsufficientStationItem)
        );
    }

    #[test]
    fn try_debit_removes_the_row_once_it_hits_zero() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2);
        assert!(db
            .station_inventory()
            .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2)
            .is_ok());
        assert!(db
            .station_inventory()
            .get_all(PlayerId(1), StationId(7))
            .is_empty());
    }

    #[test]
    fn try_debit_partial_leaves_the_remainder() {
        let db = SectorRepository::open_in_memory().unwrap();
        db.station_inventory()
            .credit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 5);
        assert!(db
            .station_inventory()
            .try_debit(PlayerId(1), StationId(7), ItemId::ScrapMetal, 2)
            .is_ok());
        assert_eq!(
            db.station_inventory()
                .get_all(PlayerId(1), StationId(7))
                .get(&ItemId::ScrapMetal),
            Some(&3)
        );
    }
}
