# Snapshot publication guarantee

`StateSnapshot` is the authoritative durable checkpoint for INV-002. Checkpointing must not compact the covered hot-log prefix until the replacement snapshot has been published successfully.

## Publication protocol

`StateSnapshot::save` owns the complete protocol:

1. Encode the unchanged postcard snapshot schema and decode it once to validate the payload.
2. Open the destination directory before replacement.
3. Create a uniquely named temporary file beside the authoritative snapshot.
4. Write the complete payload, flush it, call `sync_all`, then reread and decode it.
5. If an authoritative snapshot already exists, copy it to a fixed sibling rollback path, sync the rollback file, and sync the directory entry before replacement.
6. Replace the authoritative snapshot with the new sibling file.
7. Make the replacement durable: on Unix, sync the already-open parent directory; on Windows, use the replacement primitive's write-through semantics.
8. Remove the rollback copy after the new authoritative path is durable.

Temporary files and rollback copies are removed after normal success and handled failure. The rollback path has a fixed name, so even a cleanup failure cannot accumulate an unbounded set. A process or machine crash can still leave one stale temporary or rollback file; recovery ignores these artifacts because only the configured authoritative path is loaded. A later publication removes a stale rollback copy only when the authoritative snapshot is readable.

## Platform replacement semantics

- **Unix:** same-directory `rename` atomically replaces an existing destination. The parent directory is opened before replacement and synced afterwards. An existing snapshot remains available through the synced rollback copy until that directory sync succeeds.
- **Windows:** Rust's `std::fs::rename` does not replace an existing destination. Dawn therefore calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`. The sibling files keep every move on the same volume, and the rollback copy remains available until the write-through replacement succeeds.
- **Other platforms:** publication returns `Unsupported` rather than claiming an atomic replacement guarantee that has not been implemented.

The atomicity and durability guarantees ultimately depend on the filesystem honoring the platform's same-volume replacement and flush primitives.

## Failure and checkpoint ordering

Any handled failure before replacement leaves the authoritative path untouched. If replacement succeeds but its durability step fails, publication atomically restores the synced rollback copy and syncs the directory again before returning the error. For a failed first publication, rollback removes the new authoritative path and restores the prior state in which no snapshot existed.

`SimulationNode::checkpoint` propagates every publication error and calls `FileEventStore::compact` only after `StateSnapshot::save` returns success. Therefore a failed publication leaves both the previously published snapshot and the covered hot-log prefix available for recovery.

If restoring the authoritative path encounters an additional filesystem failure, the returned error reports both failures and retains the fixed rollback file for operator recovery rather than deleting the last known readable copy. If the path is restored but the rollback directory sync also fails, the error reports that durability remains uncertain and checkpointing still leaves the hot log untouched.

This preserves INV-001 and INV-002 without changing the postcard format or moving log rewriting outside the existing checkpoint compaction path. See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md).
