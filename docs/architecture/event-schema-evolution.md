# Event Schema Evolution Rules

> Canonical detail behind the AI_DEVELOPMENT_GUIDE.md event-schema reference (ADR-0030).
> The guide itself keeps only a note that we are currently pre-release
> (breaking changes allowed) and a link here.

## Scope by phase

This document applies in two phases:

```
Pre-release (Phase 1 through release):
  No external user holds a persisted event log.
  -> Breaking changes (remove a field, change a type, remove an Event) are allowed directly.
  -> No Upcaster, V2 naming, or Deprecated marking required.
  -> docs/architecture/event-catalog.md and AI_DEVELOPMENT_GUIDE.md must always match the code.

Post-release (once production logs exist):
  External users hold event logs.
  -> Existing fields may not be changed or removed without an Upcaster.
  -> The "Post-release constraints" below apply in full.
```

**We are currently in Phase 6 (pre-release). Breaking changes are permitted.**

---

## Post-release basic principle

**Existing Event fields must not be changed or removed. Only adding new fields is allowed.**

### Allowed post-release

```rust
// Before
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
    pub damage   : f32,
    pub tick     : Tick,
}

// After: adding a new field is allowed (must be Option)
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
    pub damage   : f32,
    pub tick     : Tick,
    pub hit_chance: Option<f32>,  // new field added as Option<T>
}
```

### Forbidden post-release

```rust
// Forbidden 1: removing a field
pub struct WeaponFired {
    pub ship_id  : ShipId,
    // target_id removed <- forbidden. Replay of past Events fails to deserialize.
    pub damage   : f32,
    pub tick     : Tick,
}

// Forbidden 2: changing a field's type
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: u64,   // ShipId -> u64 <- forbidden
    pub damage   : f32,
    pub tick     : Tick,
}

// Forbidden 3: renaming a field (changes the serialization key)
pub struct WeaponFired {
    pub attacker_id: ShipId,  // ship_id -> attacker_id <- forbidden
    pub target_id  : ShipId,
    pub damage     : f32,
    pub tick       : Tick,
}
```

### Procedure when a breaking change is unavoidable post-release

```
1. Define a new Event under a new name
   e.g. WeaponFired -> WeaponFiredV2

2. Mark the old Event as Deprecated (do not delete it)
   /// @deprecated use WeaponFiredV2
   pub struct WeaponFired { ... }

3. Implement an Upcaster
   impl Upcaster for WeaponFired {
       fn upcast(self) -> WeaponFiredV2 { ... }
   }

4. Pass through the Upcaster during Replay to convert to the new form

5. Update docs/architecture/event-catalog.md

6. Write a new ADR for this change (do not edit an existing ADR)
```

## Syncing with the Event Catalog

`docs/architecture/event-catalog.md` is the single source of truth for Events.
Update it together with any code change, in every phase.

```bash
# CI verifies that Event definitions and the catalog agree
cargo run --bin check-event-catalog

# Failure means one of:
# - an Event exists in code but not in the catalog
# - an Event exists in the catalog but not in code
# - a field definition mismatch
```
