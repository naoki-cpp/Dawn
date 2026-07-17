//! Shared station-operation vocabulary (ADR-0034 / ADR-0037 foundation).
//!
//! The implementation lives in two deepened sibling modules:
//! - `station_lifecycle.rs` owns dock / undock / active-ship selection / disembark
//! - `station_materialization.rs` owns validation and plan creation for build
//!   / assemble / disassemble
//!
//! Station inventory cache + SQLite write-through lives in
//! `station_inventory.rs`; durable storage details live in
//! `station_inventory_db.rs`.
//! - `station_operation_execution.rs` owns accepted-operation side effects and
//!   the final Event append ordering.

use dawn_core::ShipId;

/// The station command seam: callers learn whether the operation was accepted
/// and which ship should have its fitting/state resent to the client.
///
/// Callers no longer extract `ship_id` for a `RefreshPlayerLoadout` followup --
/// `apply_client_command` already has `player_id` in scope and uses that
/// directly (a `ship_id` can't always be resolved back to a player, e.g.
/// after Disassemble removes it; see `docs/architecture/ownership.md` §8).
/// `ship_id` is kept because callers (tests, other station ops) still match
/// on which ship an operation targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}
