---
scope    : Durable journal mechanics owned by issue #271
audience : AI Agent / Human Developer
update   : When the journal framing, receipt, compaction, or failure policy changes
related  : recovery-contract.md, ../adr/ADR-0049-sector-recovery-state-delta-wal.md
---

# Durable Journal

`dawn-storage::DurableJournal` is the storage boundary for one logical
Sector transition. It stores encoded bytes, not a particular domain event type,
so ADR-0049's versioned recovery representation can evolve independently of
the physical journal.

## Logical transition

`JournalBatch` is the atomic visibility unit. Its entries may contain three
independently retained streams:

- `RecoveryDelta`: authoritative state-delta bytes used for exact recovery;
- `PublicEvent`: append-only facts for presentation, replication, and audit;
- `ReliableEffect`: durable retry/idempotency intent for post-commit work.

All entries in one batch share a `TransitionId`, `SectorId`, and owner epoch.
The journal does not publish any entry from a batch until its commit marker,
content hash, and requested durability boundary have completed.

The journal does not decide the authoritative payload, replica set, quorum, or
ack gating. Those decisions belong to ADR-0049/#284 and #278. A remote
`DurabilityEvidence` value carries the same immutable receipt context so the
runtime can reject stale epochs, wrong Sectors, ranges, transitions, or
content.

The recovery journal and the public-event store also have independent cursors.
`RecoveryIndex` addresses authoritative checkpoint/RecoveryDelta coverage;
`PublicEventIndex` addresses the next public fact to replicate. They may
diverge on eventless Ticks. Checkpoint and catch-up code must pass both values
explicitly rather than treating a recovery position as a public-event tail.

## File format and recovery

`FileJournal` uses the versioned `DAWNJNL3` format. The file header contains the
format magic, the global `base_index`, and a checksum covering both header
fields. Each batch contains its record count,
first index, transition metadata, per-entry stream tag and length-prefixed
payload, content hash, and commit marker.

On open:

- an unsupported header, header checksum mismatch, invalid committed batch, bad
  hash, bad marker, or non-contiguous first index is rejected;
- a torn trailing batch is truncated to the last committed boundary and synced;
- a complete batch is never silently reindexed or skipped.

`Buffered` means the bytes reached the file after flush. `Synced` means the
file was also synced through the local storage durability point. Neither mode
claims remote durability; #278 aggregates remote evidence separately.

## Compaction and archive

`FileJournal::compact(boundary, archive_path)` accepts only a complete batch
boundary at or before the current next index. It first appends the immutable
prefix to the archive and syncs it, then writes and syncs a new hot suffix and
atomically replaces the hot file. On Unix, the parent directory is synced after
the replacement so the rename is durable across a power loss; platforms that
do not support directory fsync retain atomic replacement but do not claim that
stronger directory-entry guarantee. A crash before replacement leaves the old
hot file; a crash after replacement leaves the archived prefix and the new
suffix. The global journal indices and receipt ranges remain unchanged.

If archive append succeeds but hot replacement does not, a retry is allowed.
The retry verifies the archive records that overlap the retained hot range and
resumes from the archive's current `next_index`; it never appends an unverified
or duplicate prefix. Initial creation of both hot and archive files syncs the
file and, on Unix, their parent directory before a durable receipt is exposed.
After the hot rename itself succeeds, any failure reopening the writer or
syncing the parent directory poisons the in-memory handle; callers must reopen
the journal before appending again.

The archive is append-only across repeated compactions. Each subsequent
compaction accepts an archive whose `next_index` is between the current hot
`base_index` and the requested boundary: an already archived overlap is
verified byte-for-byte, and only the missing complete prefix is appended.
This allows a retry after archive sync succeeded but hot replacement failed
without changing the global index space. The archive path must not alias the
hot journal or its compaction temporary path; such requests are rejected
before any mutation.

After compaction, `read_from` serves the retained hot range and returns
`CompactedRange` for an archived prefix. The archive is itself a versioned
`FileJournal` and can be opened independently for audit or catch-up work.
Compaction cannot split a logical transition. The checkpoint/coverage proof
that authorizes the boundary belongs to #284; the physical move belongs here.

## Failure policy

Validation occurs before mutation. Write, flush, and sync failures roll back
the uncommitted suffix and return a typed `JournalError`. If rollback itself
fails, the journal becomes `Poisoned` and must be reopened rather than reused.
The in-memory implementation and file implementation share the batch/range/
receipt invariants; the file tests inject write, flush, and sync failures and
verify that no partial batch is visible after reopen.

`DurableJournal` is now the sole persistent source for committed transitions
and public facts. `JournalStream::PublicEvent` is projected after successful
live apply into the rebuildable `dawn_distributed::PublicEventTail`; that tail
does not add a second durable file or cursor authority. #272 establishes the
storage-independent `SectorEngine::prepare_stop` boundary, the bounded full-Tick
write set, and runtime-owned Stop/Tick adapters around `SimulationNode`.
