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

impl Default for JournalIndex {
    fn default() -> Self {
        Self::ZERO
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

/// The independently retained material carried by one logical transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JournalStream {
    /// Versioned authoritative state-delta bytes selected by ADR-0049.
    RecoveryDelta,
    /// Append-only public facts for clients, replication, and audit consumers.
    PublicEvent,
    /// Reliable post-commit work and retry/idempotency state.
    ReliableEffect,
}

/// One encoded item inside an atomic logical transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub stream: JournalStream,
    pub payload: Vec<u8>,
}

impl JournalEntry {
    pub fn new(stream: JournalStream, payload: Vec<u8>) -> Self {
        Self { stream, payload }
    }
}

/// One logical append operation. The batch must not be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalBatch {
    pub transition_id: TransitionId,
    pub context: DurabilityContext,
    pub entries: Vec<JournalEntry>,
    pub durability: DurabilityMode,
}

impl JournalBatch {
    pub fn new(
        transition_id: TransitionId,
        context: DurabilityContext,
        records: Vec<Vec<u8>>,
        durability: DurabilityMode,
    ) -> Result<Self, JournalError> {
        Self::with_entries(
            transition_id,
            context,
            records
                .into_iter()
                .map(|payload| JournalEntry::new(JournalStream::RecoveryDelta, payload))
                .collect(),
            durability,
        )
    }

    pub fn with_entries(
        transition_id: TransitionId,
        context: DurabilityContext,
        entries: Vec<JournalEntry>,
        durability: DurabilityMode,
    ) -> Result<Self, JournalError> {
        let batch = Self {
            transition_id,
            context,
            entries,
            durability,
        };
        validate_batch(&batch)?;
        Ok(batch)
    }

    pub fn entries_for(&self, stream: JournalStream) -> impl Iterator<Item = &JournalEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.stream == stream)
    }
}

/// Inclusive/exclusive range assigned to one batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JournalRange {
    pub first: JournalIndex,
    pub len: u32,
}

/// Evidence returned after a journal prefix is durably archived behind a
/// verified checkpoint boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CompactionReceipt {
    pub archived: JournalRange,
    pub retained_from: JournalIndex,
    pub next_index: JournalIndex,
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

/// Serializable proof that a local or remote durable write covers one exact
/// transition range. Replica membership and quorum policy remain outside this
/// crate; this type only carries immutable content-bound evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DurabilityEvidence {
    pub receipt: AppendReceipt,
    pub source: DurabilityEvidenceSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DurabilityEvidenceSource {
    Local,
    Remote,
}

impl DurabilityEvidence {
    pub fn validate_batch(
        &self,
        batch: &JournalBatch,
        expected_range: JournalRange,
    ) -> Result<(), JournalError> {
        self.receipt.validate_batch(batch, expected_range)
    }
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

    /// Validate immutable evidence against the exact batch and assigned range.
    ///
    /// This is intentionally stricter than comparing a transition ID: a stale
    /// owner epoch, wrong Sector, range, payload, or durability claim must not
    /// be accepted as proof of the requested write.
    pub fn validate_batch(
        &self,
        batch: &JournalBatch,
        expected_range: JournalRange,
    ) -> Result<(), JournalError> {
        if self.transition_id != batch.transition_id {
            return Err(JournalError::ReceiptMismatch("transition identity"));
        }
        if self.context != batch.context {
            return Err(JournalError::ReceiptMismatch("durability context"));
        }
        if self.range != expected_range {
            return Err(JournalError::ReceiptMismatch("journal range"));
        }
        if self.durability != batch.durability {
            return Err(JournalError::ReceiptMismatch("durability mode"));
        }
        if self.content_hash != content_hash(expected_range.first, batch) {
            return Err(JournalError::ReceiptMismatch("content hash"));
        }
        Ok(())
    }
}

/// A decoded journal record returned by the read side of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub index: JournalIndex,
    pub transition_id: TransitionId,
    pub context: DurabilityContext,
    pub stream: JournalStream,
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
    #[error("journal receipt does not match {0}")]
    ReceiptMismatch(&'static str),
    #[error("journal payload encoding failed: {0}")]
    Encode(String),
    #[error("journal read starts before compacted range: requested {requested}, available from {available_from}")]
    CompactedRange {
        requested: JournalIndex,
        available_from: JournalIndex,
    },
    #[error("journal compaction boundary {boundary} is not a complete batch boundary")]
    InvalidCompactionBoundary { boundary: JournalIndex },
    #[error("journal archive does not continue at {expected}; found {actual}")]
    ArchiveMismatch {
        expected: JournalIndex,
        actual: JournalIndex,
    },
    #[error("journal archive path aliases the hot journal")]
    ArchivePathAlias,
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

/// Encode a versioned recovery/public/output payload at the storage boundary.
/// Domain crates choose the value type; the journal only stores the resulting
/// bytes and reports serialization failure to the caller.
pub fn encode_payload<T: Serialize>(value: &T) -> Result<Vec<u8>, JournalError> {
    postcard::to_stdvec(value).map_err(|error| JournalError::Encode(error.to_string()))
}

pub(crate) fn validate_batch(batch: &JournalBatch) -> Result<(), JournalError> {
    if batch.entries.is_empty() {
        return Err(JournalError::EmptyBatch);
    }
    if batch.entries.len() > MAX_RECORDS_PER_BATCH {
        return Err(JournalError::TooManyRecords {
            actual: batch.entries.len(),
            max: MAX_RECORDS_PER_BATCH,
        });
    }
    let mut total_bytes = 0usize;
    for entry in &batch.entries {
        if entry.payload.len() > MAX_RECORD_BYTES {
            return Err(JournalError::RecordTooLarge {
                actual: entry.payload.len(),
                max: MAX_RECORD_BYTES,
            });
        }
        total_bytes =
            total_bytes
                .checked_add(entry.payload.len())
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
    for (ordinal, entry) in batch.entries.iter().enumerate() {
        mix(&mut hash, &(ordinal as u32).to_le_bytes());
        mix(
            &mut hash,
            &[match entry.stream {
                JournalStream::RecoveryDelta => 0,
                JournalStream::PublicEvent => 1,
                JournalStream::ReliableEffect => 2,
            }],
        );
        mix(&mut hash, &(entry.payload.len() as u64).to_le_bytes());
        mix(&mut hash, &entry.payload);
    }
    hash
}
