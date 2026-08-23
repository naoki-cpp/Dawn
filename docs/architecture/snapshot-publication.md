# Snapshot publication guarantee

`StateSnapshot` is the authoritative durable checkpoint for INV-002. Checkpointing must not compact the covered hot-log prefix until the replacement snapshot has been published successfully.

## Publication protocol

`StateSnapshot::save` owns the complete protocol:

1. Encode the unchanged postcard snapshot schema and decode it once to validate the payload.
2. Open the destination directory before replacement and capture the existing snapshot's protection metadata.
3. Create a uniquely named temporary file beside the authoritative snapshot. On Unix it is created with the existing mode, or `0600` for a first publication. On Windows it receives the existing snapshot's DACL before snapshot bytes are written.
4. Write the complete payload, flush it, call `sync_all`, then reread and decode it.
5. If an authoritative snapshot already exists, copy it to a fixed sibling rollback path using the same protection metadata, sync the rollback file, and sync the directory entry before replacement.
6. Replace the authoritative snapshot with the new sibling file.
7. On Unix, sync the already-open parent directory after replacement. On Windows, the replacement file was synced before publication; `ReplaceFileW` preserves the existing destination's metadata during replacement, while first publication uses `MoveFileExW(MOVEFILE_WRITE_THROUGH)`.
8. Remove the rollback copy after the new authoritative path is published successfully.

Temporary files and rollback copies are removed after normal success and handled failure. The rollback path has a fixed name, so even a cleanup failure cannot accumulate an unbounded set. A process or machine crash can still leave one stale temporary or rollback file; recovery ignores these artifacts because only the configured authoritative path is loaded. A later publication removes a stale rollback copy only when the authoritative snapshot is readable.

## Platform replacement and protection semantics

- **Unix:** same-directory `rename` atomically replaces an existing destination. Temporary and rollback files are created with the existing snapshot mode before they become visible; a first publication defaults to owner-only `0600`. The parent directory is opened before replacement and synced afterwards. An existing snapshot remains available through the synced rollback copy until that directory sync succeeds.
- **Windows:** temporary and rollback files receive the existing snapshot's DACL before data is exposed through them. `ReplaceFileW` atomically replaces an existing destination while preserving its DACL and other protection-related file metadata. A first publication has no previous ACL to preserve and inherits the destination directory's normal Windows security policy before `MoveFileExW(MOVEFILE_WRITE_THROUGH)` publishes it.
- **Other platforms:** publication returns `Unsupported` rather than claiming an atomic replacement guarantee that has not been implemented.

The atomicity, durability, and metadata guarantees ultimately depend on the filesystem honoring the platform's same-volume replacement, security, and flush primitives.

## Failure and checkpoint ordering

Any handled failure before replacement leaves the authoritative path untouched. If replacement succeeds but its durability step fails, publication atomically restores the synced rollback copy and syncs the directory again before returning the error. For a failed first publication, rollback removes the new authoritative path and restores the prior state in which no snapshot existed.

`CheckpointScheduler` propagates every publication error and compacts only the
authoritative `FileJournal` after `StateSnapshot::save` returns success.
Public facts remain in the journal's `PublicEvent` stream and are rebuilt into
the bounded `PublicEventTail`; a failed checkpoint never makes the tail a
second durability source.

If restoring the authoritative path encounters an additional filesystem failure, the returned error reports both failures and retains the fixed rollback file for operator recovery rather than deleting the last known readable copy. If the path is restored but the rollback directory sync also fails, the error reports that durability remains uncertain and checkpointing still leaves the hot log untouched.

This preserves INV-001 and INV-002 without changing the postcard format or moving log rewriting outside the existing checkpoint compaction path. See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md).
