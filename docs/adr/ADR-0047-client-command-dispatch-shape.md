---
id      : ADR-0047
title   : Client request dispatch shape
status  : superseded
date    : 2026-07-28
superseded_by: issue #273 (2026-08-06)
related : ADR-0037 (active ship / owned ship), ADR-0041/0042 (wire schema),
          docs/architecture/architecture.md
---

# ADR-0047 - Client request dispatch shape

## Historical decision

The original implementation retained one exhaustive outer command match and
then re-encoded requests into private `FlightDispatchCommand`,
`ModuleDispatchCommand`, `LoadoutDispatchCommand`, and
`StationDispatchCommand` enums. Those enums gave each family a closed input
set, but together they repeated the complete external request catalog.

The outer entry point also maintained a separate
`request_requires_active_ship` classification table. A new request could be
added to the exhaustive dispatch match without being added to that table,
leaving a latent panic path at the later `expect`.

## Superseding decision: issue #273

`ClientRequest` is now the single authoritative catalog of externally supported
Sector requests. `SimulationNode::apply_client_request` owns one exhaustive
match and each arm directly:

1. obtains the active ship with `Result` when that specific request needs it;
2. constructs any family-local domain command that adds meaning;
3. calls the family-local policy method; and
4. returns the required server follow-up.

The four `*DispatchCommand` enums and their effect-projection layer are deleted.
Family-local domain types such as `ActivateModuleCommand`, `DockCommand`, and
`TransferToStationCommand` remain because they express policy inputs and are
used independently of the external protocol; they do not recreate a second
complete request catalog.

## Consequences

- Adding a `ClientRequest` variant makes the one admission match fail to compile
  until its authority, policy call, and follow-up are specified.
- There is no independent active-ship classification list to drift from the
  match. Each active-ship arm uses `require_active_ship(active_ship)?`.
- The pure Sector engine still receives typed values and does not depend on
  postcard, JSON Schema, WebSocket, or Godot APIs.
- Domain-specific rejection/result types remain local to their policies rather
  than being flattened into one generic command result.
- The historical closed-family dispatch enums are not to be reintroduced unless
  the external protocol itself is intentionally redesigned as nested family
  enums with an explicit wire-version change.
