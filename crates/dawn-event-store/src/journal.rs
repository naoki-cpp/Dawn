//! Storage-independent contract for one durable logical transition.
//!
//! `DurableJournal` stores encoded records rather than `DomainEvent`s. This
//! keeps the persistence boundary able to carry ADR-0049 recovery data,
//! public facts, and reliable effect records without making any one of them
//! the recovery format.

use std::fmt;

use dawn_core::SectorId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Zero-based position of a record in the authoritative journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct JournalIndex(pub u64);

impl JournalIndex {
    pub const ZERO: Self = Self(0);

    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

impl fmt::Display for JournalIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Stable identity of one logical transition, independent of its records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransitionId(pub u128);

/// Immutable context bound to a durable write receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DurabilityContext {
    pub sector_id: SectorId,
    pub owner_epoch: u64,
}

/// How strongly the journal acknowledges a locally completed append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DurabilityMode {
    /// The batch was flushed to the operating-system file buffer only.
    Buffered,
    /// The batch was flushed and synced to the local storage durability point.
    Synced,
}

/// One logical append operation. The batch must not be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalBatch {
    pub transition_id: TransitionId,
    pub context: DurabilityContext,
    pub records: Vec<Vec<u8>>,
    pub durability: DurabilityMode,
}

impl JournalBatch {
    pub fn new(
        transition_id: TransitionId,
        context: DurabilityContext,
        records: Vec<Vec<u8>>,
        durability: DurabilityMode,
    ) -> Self {
        Self {
            transition_id,
            context,
            records,
            durability,
        }
    }
}

/// Inclusive/exclusive range assigned to one batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JournalRange {
    pub first: JournalIndex,
    pub len: u32,
}

impl JournalRange {
    pub const fn checked_last_exclusive(self) -> Option<JournalIndex> {
        match self.first.0.checked_add(self.len as u64) {
            Some(value) => Some(JournalIndex(value)),
            None => None,
        }
    }

    pub const fn contains(self, index: JournalIndex) -> bool {
        match self.checked_last_exclusive() {
            Some(last_exclusive) => index.0 >= self.first.0 && index.0 < last_exclusive.0,
            None => false,
        }
    }
}

/// Evidence returned after a batch becomes visible at the requested durability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AppendReceipt {
    pub transition_id: TransitionId,
    pub context: DurabilityContext,
    pub range: JournalRange,
    /// Deterministic content binding for the transition metadata and payloads.
    /// This is an identity/checking digest, not an authenticity signature.
    pub content_hash: u64,
    pub durability: DurabilityMode,
}

impl AppendReceipt {
    pub fn matches(
        &self,
        transition_id: TransitionId,
        context: DurabilityContext,
        range: JournalRange,
        content_hash: u64,
    ) -> bool {
        self.transition_id == transition_id
            && self.context == context
            && self.range == range
            && self.content_hash == content_hash
    }
}

/// A decoded journal record returned by the read side of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub index: JournalIndex,
    pub transition_id: TransitionId,
    pub context: DurabilityContext,
    pub ordinal: u32,
    pub payload: Vec<u8>,
}

/// Errors that must be handled by the runtime instead of becoming panics.
#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal batch must contain at least one record")]
    EmptyBatch,
    #[error("journal batch contains too many records: {actual} > {max}")]
    TooManyRecords { actual: usize, max: usize },
    #[error("journal record is too large: {actual} bytes > {max}")]
    RecordTooLarge { actual: usize, max: usize },
    #[error("journal batch payload is too large: {actual} bytes > {max}")]
    BatchTooLarge { actual: usize, max: usize },
    #[error("journal index overflow")]
    IndexOverflow,
    #[error("journal format is invalid: {0}")]
    InvalidFormat(String),
    #[error("journal format version is unsupported")]
    UnsupportedFormat,
    #[error("journal trailing batch is incomplete")]
    IncompleteBatch,
    #[error("journal content digest mismatch")]
    ContentHashMismatch,
    #[error("journal commit marker is invalid")]
    InvalidCommitMarker,
    #[error("journal batch starts at {actual}, expected {expected}")]
    NonContiguousBatch {
        expected: JournalIndex,
        actual: JournalIndex,
    },
    #[error("journal is unusable after a failed rollback; reopen it")]
    Poisoned,
    #[error("journal I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Maximum record count accepted by one logical transition.
pub const MAX_RECORDS_PER_BATCH: usize = 4096;
/// Maximum encoded record size accepted by the storage boundary.
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// Maximum combined encoded payload size accepted by one logical batch.
pub const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

/// Journal abstraction with atomic logical batches and explicit durability.
pub trait DurableJournal {
    fn append_batch(&mut self, batch: JournalBatch) -> Result<AppendReceipt, JournalError>;

    fn read_from(
        &self,
        index: JournalIndex,
    ) -> Result<Box<dyn Iterator<Item = Result<JournalRecord, JournalError>> + '_>, JournalError>;

    fn next_index(&self) -> Result<JournalIndex, JournalError>;
}

pub(crate) fn validate_batch(batch: &JournalBatch) -> Result<(), JournalError> {
    if batch.records.is_empty() {
        return Err(JournalError::EmptyBatch);
    }
    if batch.records.len() > MAX_RECORDS_PER_BATCH {
        return Err(JournalError::TooManyRecords {
            actual: batch.records.len(),
            max: MAX_RECORDS_PER_BATCH,
        });
    }
    let mut total_bytes = 0usize;
    for record in &batch.records {
        if record.len() > MAX_RECORD_BYTES {
            return Err(JournalError::RecordTooLarge {
                actual: record.len(),
                max: MAX_RECORD_BYTES,
            });
        }
        total_bytes = total_bytes
            .checked_add(record.len())
            .ok_or(JournalError::BatchTooLarge {
                actual: usize::MAX,
                max: MAX_BATCH_BYTES,
            })?;
    }
    if total_bytes > MAX_BATCH_BYTES {
        return Err(JournalError::BatchTooLarge {
            actual: total_bytes,
            max: MAX_BATCH_BYTES,
        });
    }
    Ok(())
}

/// Stable non-cryptographic digest used to bind receipt evidence to content.
pub(crate) fn content_hash(first: JournalIndex, batch: &JournalBatch) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    fn mix(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }

    mix(&mut hash, &batch.transition_id.0.to_le_bytes());
    mix(&mut hash, &first.0.to_le_bytes());
    mix(&mut hash, &[batch.context.sector_id.0]);
    mix(&mut hash, &batch.context.owner_epoch.to_le_bytes());
    mix(
        &mut hash,
        &[match batch.durability {
            DurabilityMode::Buffered => 0,
            DurabilityMode::Synced => 1,
        }],
    );
    for (ordinal, record) in batch.records.iter().enumerate() {
        mix(&mut hash, &(ordinal as u32).to_le_bytes());
        mix(&mut hash, &(record.len() as u64).to_le_bytes());
        mix(&mut hash, record);
    }
    hash
}
