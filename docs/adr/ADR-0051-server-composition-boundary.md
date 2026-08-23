---
id      : ADR-0051
title   : One Server Composition Boundary for Production and Local Runs
status  : accepted
date    : 2026-08-10
deciders: [human, ai-agent]
related : ADR-0026 (Sector owns gameplay rather than runtime composition), ADR-0049 (durable runtime boundary), ADR-0050 (shared peer transport)
---

# ADR-0051: One Server Composition Boundary for Production and Local Runs

> Amendment (#338): the shared client transport is now owned by `dawn-server`.
> `dawn-actor` has been deleted; both server binaries use the library's single
> WebSocket framing, handshake, and connection boundary.

## Context

The workspace currently has two executable packages that assemble the same
Sector runtime:

- `dawn-simulation` owns local, single-node, and cluster server modes;
- `dawn-sector-node` owns the production peer-connected process.

Both packages wire WebSocket admission, Raft, replication, checkpoints, and
the Sector runtime. This leaves two composition roots and makes a change to a
runtime contract easy to apply to one deployment but not the other. The
package name `dawn-simulation` also hides that its `--serve` modes are the
local server implementation used for client playtests.

The consolidation work in issue #282 is explicitly pre-release. Package names
and binary paths may change without compatibility shims.

## Decision

Rename the executable package to `dawn-server` and make it the single server
composition boundary. It contains:

- the `simulate` binary for local benchmarks, demos, and playtest servers;
- the `sector-node` binary for the production peer-connected process;
- one package boundary around the shared `dawn-sector` runtime frame. The
  deployment-specific peer/FileJournal wiring remains local to `sector-node`,
  while local simulation adapters remain local to `simulate`.

The `dawn-sector-node` package is deleted. Its configuration and production
adapter modules move under `dawn-server`. The `dawn-sector` crate remains the
Sector domain/runtime library and continues to own authoritative state
transitions; `dawn-server` only selects concrete transports, journals,
repositories, and deployment configuration.

The client transport is part of the `dawn-server` library. It owns the shared
`ClientConnection`, `WsServer`, handshake, and `PlayerSession` implementation;
the `dawn-protocol` crate remains the schema owner and has no transport
dependency. This keeps both server binaries on one transport implementation
without adding a compatibility facade.

The resulting executable boundary is:

```text
dawn-core / dawn-protocol / dawn-market / dawn-ecs / dawn-storage
        ^
    dawn-sector          dawn-distributed
        ^                         ^
                 dawn-server
          (simulate + sector-node)
```

No server binary may define a second tick ordering or a second authoritative
Sector reducer. The dependency check added with this ADR rejects reintroducing
the deleted package or a reverse dependency from the Sector library to the
server composition root.

## Alternatives considered

1. Keep both packages and document them better. Rejected because documentation
   cannot prevent runtime policy from diverging between two composition roots.
2. Put the production binary directly in `dawn-sector`. Rejected because the
   Sector library must remain independent of deployment I/O, peer transport,
   and process configuration.
3. Create a new `dawn-runtime` crate while retaining both executable packages.
   Rejected for this step because it would add a third boundary before the two
   existing composition roots had been removed.

## Implementation checklist

- [x] Rename the package directory and package name to `dawn-server`.
- [x] Move the production `sector-node` binary, config loader, admission
      adapter, and runtime adapter into `dawn-server`.
- [x] Remove `dawn-sector-node` from the workspace and delete its package.
- [x] Update scripts, CI, docs, examples, and commands to use `dawn-server`.
- [x] Add a dependency-boundary check that rejects the deleted package and
      server dependencies from `dawn-sector`.
- [x] Extract `dawn-server::runtime_frame::RuntimeFrameHost` as the shared
      one-Sector frame owner used by single serve, cluster serve, the production
      `sector-node`, and the runtime driver. Keep network delivery and
      cross-Sector routing outside the Host.
- [x] Absorb the shared client transport into the `dawn-server` library, move
      the schema generator to `dawn-protocol`, and delete `dawn-actor` (#338).
- [x] Verify both binaries with workspace format, test, and clippy checks.
