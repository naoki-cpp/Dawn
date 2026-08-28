//! Shared station-operation vocabulary (ADR-0034 / ADR-0037 foundation).
//!
//! The implementation lives in two deepened sibling modules:
//! - `station_lifecycle.rs` owns dock / undock / active-ship selection / disembark
//! - `station_materialization.rs` owns validation and plan creation for build
//!   / assemble / disassemble
//!
//! Station inventory cache + SQLite write-through lives in
//! `station_inventory.rs`; durable storage details live in
//! `repositories/station_inventory.rs`.
//! - `station_operation_execution.rs` owns accepted-operation side effects and
//!   the final Event append ordering.

use dawn_core::ShipId;

use super::repositories::ProjectionReadError;

/// The station command seam: callers learn whether the operation was accepted
/// and which ship should have its fitting/state resent to the client.
///
/// Callers no longer extract `ship_id` for a `RefreshPlayerLoadout` followup --
/// `apply_client_request` already has `player_id` in scope and uses that
/// directly (a `ship_id` can't always be resolved back to a player, e.g.
/// after Disassemble removes it; see `docs/architecture/ownership.md` §8).
/// `ship_id` is kept because callers (tests, other station ops) still match
/// on which ship an operation targeted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StationOperationOutcome {
    Accepted {
        ship_id: ShipId,
    },
    Rejected {
        ship_id: ShipId,
        reason: StationOperationRejection,
    },
}

/// Why a station operation was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StationOperationRejection {
    NotOwned,
    AlreadyDocked,
    OutOfDockRange,
    ShipNotDocked,
    MissingDockedStationContext,
    WrongDockedStation,
    UnknownShipType,
    MissingStationItem,
    InsufficientStationItem,
    ShipNotFound,
    ShipIsFitted,
    ShipIsDamaged,
    /// `SelectActiveShipCommand` targeted the already-active ship (ADR-0037).
    AlreadyActive,
    /// `SelectActiveShipCommand` targeted a ship not docked at the caller's
    /// current docked station (ADR-0037; station-local switch only).
    ShipNotDockedHere,
    /// The Station projection could not be read safely.
    ProjectionRead(ProjectionReadError),
}

impl StationOperationRejection {
    pub(super) fn projection_read(error: ProjectionReadError) -> Self {
        Self::ProjectionRead(error)
    }
}

impl StationOperationOutcome {
    pub(super) fn fail_on_projection_read(self) -> Result<Self, ProjectionReadError> {
        match self {
            Self::Rejected {
                reason: StationOperationRejection::ProjectionRead(error),
                ..
            } => Err(error),
            other => Ok(other),
        }
    }
}
