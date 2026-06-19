//! # dawn-replication
//!
//! Sector-local event log replication (ADR-0021, ADR-0027).
//!
//! Strategy: gossip-based append-log shipping (not CRDT / LWW).
//! Single-ownership means no concurrent writes, so conflict-free merge is
//! unnecessary. Events are idempotent (INV-004) and ordered by logical tick
//! (INV-005), so delivering them in any order converges to the same state.
//!
//! ## Current scope (8D-2a)
//!
//! - `ReplicationTransport` trait — wire-format-agnostic interface.
//! - `InMemoryReplicationBus` — single-process implementation used by tests
//!   and the existing multi-node bench. This replaces `dawn_actor::ReplicationBus`.
//!
//! ## Planned (8D-2b / 8D-2c / 8D-2d)
//!
//! - `AntiEntropy` — request missing events by log index range.
//! - `TcpReplicationTransport` — LAN plaintext, postcard wire format.
//! - `SnapshotTransfer` — catch up far-behind replicas via snapshot.

pub mod bus;

pub use bus::{BusMessage, InMemoryReplicationBus};
