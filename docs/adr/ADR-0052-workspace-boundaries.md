---
id      : ADR-0052
title   : Final Workspace Boundaries
status  : accepted
date    : 2026-08-11
deciders: [human, ai-agent]
related : ADR-0027, ADR-0041, ADR-0042, ADR-0049, ADR-0050, ADR-0051, #282
---

# ADR-0052: Final Workspace Boundaries

## Context

The workspace reached the end of several ownership migrations while retaining
package names from those implementation phases. Wire schemas, event storage,
Raft, peer transport, and replication were split across packages even though
their stable dependency and deployment boundaries had already converged.
Those names made the dependency graph harder to read and allowed obsolete
packages to reappear through compatibility-only forwarding crates.

Issue #282 is pre-release work. Existing package names and public paths may
change; compatibility shims are not part of this decision.

## Decision

Use the following package boundaries:

- `dawn-protocol` is the single client/server schema authority. It contains
  typed requests, facts, handshakes, and postcard envelopes, and has no
  transport or runtime dependency.
- `dawn-storage` is the single storage authority. It contains the public-fact
  store and the fallible `DurableJournal` mechanics used by the recovery
  contract. Sector snapshot and recovery application code remains next to the
  Sector state it restores; it does not create a second journal authority.
- `dawn-distributed` is the single cross-Sector coordination boundary. Its
  Raft, replication, and peer-transport modules are separate policy layers over
  one opaque transport implementation. The transport does not depend on
  Sector, server, client, protocol, or market crates.
- `dawn-server` is the only executable composition root. Both `simulate` and
  `sector-node` select storage, transport, repository, and client adapters
  around the shared Sector runtime.
- `dawn-sector` remains the authoritative Sector runtime package. Its
  `SimulationNode` and transition engine are storage-side-effect-free; server
  orchestration owns journal append and acknowledgement ordering. Protocol,
  recovery, and Transit adapters remain explicit modules of this runtime until
  a future split creates a meaningful dependency rule rather than another
  forwarding crate.

The old `dawn-wire`, `dawn-event-store`, `dawn-consensus`,
`dawn-peer-transport`, `dawn-replication`, `dawn-simulation`, and
`dawn-sector-node` packages are deleted. No compatibility-only package or
re-export shim is retained.

The production dependency checker rejects deleted package names, missing final
boundaries, reverse dependencies from client/core packages, and cycles. It
also protects the one-server-composition-root rule.

## Consequences

The crate graph now describes ownership and deployment rather than historical
milestones. A contributor can find wire schema, persistence, distributed
coordination, and executable wiring in one place each. The remaining internal
modules in `dawn-sector` are documented as adapters around the authoritative
engine, so moving a future adapter does not require inventing a new package
unless it enforces a real dependency restriction.

The migration intentionally changes package names and import paths. Existing
pre-release binaries and local storage layouts are not compatibility targets.

## Implementation checklist

- [x] Rename the protocol and storage authorities.
- [x] Consolidate Raft, replication, and peer transport into `dawn-distributed`.
- [x] Keep `dawn-server` as the sole production/local composition root.
- [x] Delete obsolete packages and update imports, CI, scripts, examples, and docs.
- [x] Add automated package and dependency-DAG checks.
- [x] Verify workspace formatting, tests, and clippy.
