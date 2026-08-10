//! `EventRecord` — a single entry in the append-only Event Log.

use dawn_core::DomainEvent;

/// One immutable entry in the Event Log.
///
/// `log_index` is a monotonically increasing sequence number scoped to a
/// single Sector's log.  It serves as the canonical address for the entry and
/// the replay cursor.
#[derive(Debug, Clone)]
pub struct EventRecord {
    /// Zero-based, monotonically increasing position in this Sector's log.
    pub log_index: u64,
    /// The domain event that occurred at this position.
    pub event: DomainEvent,
}

impl EventRecord {
    pub(crate) fn new(log_index: u64, event: DomainEvent) -> Self {
        Self { log_index, event }
    }
}
