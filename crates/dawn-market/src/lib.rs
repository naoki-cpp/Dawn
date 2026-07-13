//! Player-to-player Market (ADR-0034 §4/§5/§6).
//!
//! This crate will own the bid/ask order book and the `PlayerId`-keyed
//! Currency ledger, backed by its own SQLite database -- a separate
//! authority from the Sector's event-sourced state, deliberately outside
//! the Sector tick's determinism constraints (INV-001/002/005). It bridges
//! into Sector-owned state (a ship's `InventoryComp`) only through three
//! one-sided `dawn-core` commands built elsewhere in the roadmap
//! (`RemoveItemCommand` on List, `ReturnItemCommand` on Cancel,
//! `CreditItemCommand` on Settle) -- this crate constructs those commands,
//! but never applies them itself; the caller (currently `dawn-simulation`,
//! see roadmap.md §12 9D-1) is responsible for routing each one to
//! whichever `SimulationNode` currently owns the affected ship.
//!
//! # Scope of this crate today
//!
//! This is the Phase 9D-1 slice: establishing this crate's place in the
//! Dependency DAG (leaf, `dawn-core` + serde + rusqlite only, same position
//! as `dawn-wire` -- no dependency on `dawn-ecs`/`dawn-event-store`/
//! `dawn-sector`, and no crate depends on this one yet). The order book,
//! Currency ledger, and the three bridging commands are 9D-2/9D-3/9D-4
//! follow-up work (roadmap.md §12); this crate deliberately has no public
//! API yet, so there is nothing here for `/rust-api-audit`'s C-EXAMPLE
//! check to exercise -- skipped for this reason, not omitted by oversight.
