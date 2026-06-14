//! Entity identity types.
//!
//! # ID encoding
//!
//! `EntityId` packs `NodeId` into the upper 8 bits and a monotonic counter
//! into the lower 56 bits.  This guarantees global uniqueness across nodes
//! without coordination, as long as each node owns its own counter.
//!
//! IDs are **never reused**.  See CLAUDE.md INV-004 and FBD-005.

use serde::{Deserialize, Serialize};

// ── EntityId ─────────────────────────────────────────────────────────────────

/// Globally unique, non-reusable identifier for any entity in the world.
///
/// Layout: `[node_id: 8 bits | counter: 56 bits]`
///
/// `Ord` is the canonical id order: because `node_id` occupies the high bits,
/// comparing the packed `u64` orders by `node_id` first, then `counter`. Use it
/// for any determinism need (e.g. byte-stable snapshots, INV-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId(u64);

impl EntityId {
    /// Create a new ID.  `counter` must be monotonically increasing within a
    /// given `node_id` across the entire lifetime of the world.
    pub fn new(node_id: NodeId, counter: u64) -> Self {
        debug_assert!(
            counter < (1u64 << 56),
            "counter overflows 56-bit space: {counter}"
        );
        Self(((node_id.0 as u64) << 56) | (counter & 0x00FF_FFFF_FFFF_FFFF))
    }

    pub fn raw(self) -> u64 {
        self.0
    }

    /// 生の u64 値から EntityId を復元する（ネットワーク受信時などに使用）。
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub fn node_id(self) -> NodeId {
        NodeId((self.0 >> 56) as u8)
    }

    pub fn counter(self) -> u64 {
        self.0 & 0x00FF_FFFF_FFFF_FFFF
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entity({:016x})", self.0)
    }
}

// ── ShipId ───────────────────────────────────────────────────────────────────

/// Typed wrapper that distinguishes a Ship entity from any future entity type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShipId(pub EntityId);

impl ShipId {
    pub fn new(node_id: NodeId, counter: u64) -> Self {
        Self(EntityId::new(node_id, counter))
    }

    pub fn raw(self) -> u64 {
        self.0.raw()
    }
}

impl std::fmt::Display for ShipId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ship({:016x})", self.0.raw())
    }
}

// ── NodeId ───────────────────────────────────────────────────────────────────

/// Identifies a simulation node in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u8);

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Node({})", self.0)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_encodes_node_and_counter_without_overlap() {
        let id = EntityId::new(NodeId(3), 42);
        assert_eq!(id.node_id(), NodeId(3));
        assert_eq!(id.counter(), 42);
    }

    #[test]
    fn entity_ids_with_different_counters_are_not_equal() {
        let a = EntityId::new(NodeId(1), 0);
        let b = EntityId::new(NodeId(1), 1);
        assert_ne!(a, b);
    }

    #[test]
    fn entity_ids_with_different_node_ids_are_not_equal() {
        let a = EntityId::new(NodeId(0), 100);
        let b = EntityId::new(NodeId(1), 100);
        assert_ne!(a, b);
    }

    #[test]
    fn entity_id_maximum_counter_fits_in_56_bits() {
        let max_counter = (1u64 << 56) - 1;
        let id = EntityId::new(NodeId(255), max_counter);
        assert_eq!(id.counter(), max_counter);
        assert_eq!(id.node_id(), NodeId(255));
    }

    #[test]
    fn entity_id_ordering_is_node_id_dominant_then_counter() {
        // A lower node_id sorts first even with a much larger counter.
        let a = EntityId::new(NodeId(0), 1_000_000);
        let b = EntityId::new(NodeId(1), 0);
        assert!(a < b);
        // Within the same node, the lower counter sorts first.
        let c = EntityId::new(NodeId(3), 5);
        let d = EntityId::new(NodeId(3), 6);
        assert!(c < d);
    }
}
