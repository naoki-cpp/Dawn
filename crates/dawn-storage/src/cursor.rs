//! Explicit cursor types for the two durable streams owned by Dawn.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Position in the authoritative recovery journal.
///
/// This is a checkpoint/recovery boundary. It is not a public-event
/// replication cursor, even when both streams happen to have the same value.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct RecoveryIndex(pub u64);

impl RecoveryIndex {
    pub const ZERO: Self = Self(0);
}

/// Position in the append-only public-event stream.
///
/// Catch-up uses this cursor to identify the next public event that a replica
/// needs. It must not be populated from a checkpoint's recovery coverage.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct PublicEventIndex(pub u64);

impl PublicEventIndex {
    pub const ZERO: Self = Self(0);
}

impl From<u64> for RecoveryIndex {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<RecoveryIndex> for u64 {
    fn from(value: RecoveryIndex) -> Self {
        value.0
    }
}

impl From<u64> for PublicEventIndex {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<PublicEventIndex> for u64 {
    fn from(value: PublicEventIndex) -> Self {
        value.0
    }
}

impl PartialEq<u64> for PublicEventIndex {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}

impl PartialEq<PublicEventIndex> for u64 {
    fn eq(&self, other: &PublicEventIndex) -> bool {
        *self == other.0
    }
}

impl PartialEq<u64> for RecoveryIndex {
    fn eq(&self, other: &u64) -> bool {
        self.0 == *other
    }
}

impl PartialEq<RecoveryIndex> for u64 {
    fn eq(&self, other: &RecoveryIndex) -> bool {
        *self == other.0
    }
}

impl fmt::Display for RecoveryIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for PublicEventIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
