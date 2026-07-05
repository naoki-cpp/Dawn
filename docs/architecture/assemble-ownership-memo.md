---
title: Assemble Ownership Memo
status: draft
updated: 2026-07-05
related:
  - ../adr/ADR-0034-economy-foundations.md
  - ../architecture/ownership.md
  - ../process/roadmap.md
---

# Assemble Ownership Memo

## Why this memo exists

Phase 9B's `BuildPackagedShip` fits the current server model cleanly:

- the player stays in their current ship
- the station inventory changes
- no new live ship entity is created

`Assemble` is different. It turns a station inventory item into a live `Ship`
entity, and that collides with the current ownership model.

Today `dawn-sector` treats player ship ownership as effectively **1 player -> 1
active ship**:

- `ShipRegistry.by_player: HashMap<PlayerId, ShipId>`
- command routing asks "does this player own this ship?"
- serve/session code assumes one session ship per player

So before implementing `Assemble`, we need one explicit answer to:

> When a docked player assembles a packaged ship, what happens to the ship they
> arrived in?

## Current code reality

The codebase already supports:

- many player-owned ships existing in a sector in principle (`owners` is
  `ShipId -> PlayerId`)
- one *active* ship per player in practice (`by_player` is `PlayerId -> ShipId`)
- docking as authoritative state (`docked_ships`)
- player-level station access context (`docked_players`)
- station inventory per player

The missing concept is not "another ship exists". The missing concept is:

> a player can own more than one ship while exactly one of them is the current
> controllable ship.

That distinction does not exist yet.

## Options considered

### Option A. Assemble replaces the current ship immediately

Flow:

1. dock in ship A
2. assemble packaged ship B
3. session ownership switches from A to B immediately

Pros:

- minimal change to the `PlayerId -> ShipId` model
- no need to represent multiple owned ships yet

Cons:

- the arrival ship A has to go somewhere
- if A is not disassembled, it becomes an unowned or hidden live ship, which is
  wrong
- if A is auto-disassembled, `Assemble` silently performs a second operation the
  user did not ask for
- makes station actions feel magical rather than explicit

Conclusion: reject for now.

### Option B. Distinguish "owned ships" from "active ship"

Flow:

1. dock in ship A
2. assemble packaged ship B
3. player still has one active docked ship context, but station now contains a
   live owned ship B alongside A
4. a separate future action selects which assembled ship to undock in

Pros:

- matches the game fantasy better
- keeps `Assemble` narrow: item -> ship entity
- avoids hidden side effects
- scales naturally to "player owns several ships in one station"

Cons:

- requires an ownership model extension
- session and command routing must distinguish:
  - owned ships
  - active ship
  - docked station context

Conclusion: recommended direction.

### Option C. Delay live ship creation and treat assembled ships as a second item form

Flow:

1. packaged ship becomes some "assembled but docked" record
2. live entity appears only on undock

Pros:

- avoids touching current live ownership immediately

Cons:

- introduces a third ship form not in ADR-0034
- pushes complexity into a bespoke hidden state
- weakens the meaning of "Assemble"

Conclusion: reject.

## Recommended model

Adopt **Option B** in two steps.

### Step 1. Introduce a docked station roster

While docked, the player's station context may contain:

- the arrival ship they are currently docked in
- additional assembled ships in the same station
- packaged ships in station inventory

This requires a new server-side distinction:

- **Owned ship**: `PlayerId` has title to this ship
- **Active ship**: the ship current player commands route to outside station
- **Docked station context**: the station the player is currently inside

`Assemble` should add a new owned ship to the docked station roster without
changing the active ship automatically.

### Step 2. Add a later "switch active docked ship / undock with ship" action

`Assemble` alone should not decide which ship becomes active. A later explicit
station action should do that.

This keeps responsibilities clean:

- `BuildPackagedShip`: resources -> packaged item
- `Assemble`: packaged item -> docked owned ship
- `Disassemble`: docked owned ship -> packaged item
- `Undock` or `SelectDockedShip`: choose which ship leaves as the active ship

## Implementation consequence for 9B

For 9B, the safe order is:

1. `BuildPackagedShip` (done)
2. preserve station access as player-level docked context even when ship-specific
   state changes (done)
3. `Disassemble` if scoped to the currently active docked ship only
4. ownership-model extension for "owned ships vs active ship"
5. `Assemble`
6. client station roster UI

In other words: **`Assemble` is not blocked by Station work anymore; it is
blocked by the ownership vocabulary being too shallow.**

## Suggested follow-up

Before implementing `Assemble`, record one more decision:

> Should the first ownership extension live as a station-local roster only, or
> should Dawn introduce a general `PlayerId -> Vec<ShipId>` ownership model
> immediately?

My recommendation is to start station-local first. It is the smallest change
that explains `Assemble` correctly without prematurely redesigning all flight
session code.
