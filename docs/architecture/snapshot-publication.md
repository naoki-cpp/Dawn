# Snapshot publication guarantee

`StateSnapshot` is the authoritative durable checkpoint for INV-002. Checkpointing must not compact the covered hot-log prefix until the replacement snapshot has been published successfully.

## Publication protocol

`StateSnapshot::save` owns the complete protocol:

1. Encode the unchanged postcard snapshot schema and decode it once to validate the payload.
2. Create a uniquely named temporary file beside the authoritative snapshot.
3. Write the complete payload, flush it, and call `sync_all`.
4. Read and decode the temporary file before publication.
5. Replace the authoritative snapshot with the sibling temporary file.
6. On Unix, sync the parent directory after `rename` so the directory entry is durable.

Temporary files are removed after normal success and after handled failures. A process or machine crash can still leave one stale temporary file; recovery ignores such files because only the configured authoritative path is loaded.

## Platform replacement semantics

- **Unix:** same-directory `rename` atomically replaces an existing destination, followed by a parent-directory sync.
- **Windows:** Rust's `std::fs::rename` does not replace an existing destination. Dawn therefore calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`. The sibling temporary file keeps the move on the same volume.
- **Other platforms:** publication returns `Unsupported` rather than claiming an atomic replacement guarantee that has not been implemented.

The atomicity guarantee ultimately depends on the filesystem honoring the platform's same-volume rename or replacement primitive.

## Failure and checkpoint ordering

Before the replacement primitive succeeds, any handled error leaves the previously published snapshot readable and removes the temporary file. `SimulationNode::checkpoint` propagates publication errors and calls `FileEventStore::compact` only after `StateSnapshot::save` returns success. Therefore a failed publication cannot remove the covered prefix from the hot log.

If the replacement succeeds but a later durability operation reports an error, checkpointing still does not compact the hot log. Recovery remains safe because the hot log retains all events, although the authoritative path may already contain the new snapshot.

This preserves INV-001 and INV-002 without changing the postcard format or moving log rewriting outside the existing checkpoint compaction path. See [ADR-0017](../adr/ADR-0017-snapshot-compaction.md).
