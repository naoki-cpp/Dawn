//! `MultiNodeCluster` — orchestrates N `SectorSimulatorActor`s + one `ReplicationBus`.
//!
//! This is the Phase 2 completion test harness.
//!
//! # Consistency model
//!
//! Each node has its own ECS World and local EventStore.
//! After every tick, events are forwarded to the shared `ReplicationBus`
//! *before* the tick reply is returned to the caller.
//!
//! Because `tick_all()` awaits all nodes sequentially (or via join), and
//! `total_replicated_events()` sends a query *after* all ticks complete,
//! the query is guaranteed to observe all events from all ticks.
//! No sleep, no flush, no barrier is required.

use crate::sector_simulator_actor::{NodeStats, SectorSimulatorHandle, TickSummary};
use crate::spawner::{generate_ships, SpawnConfig};
use dawn_actor::ReplicationBusHandle;
use dawn_core::{NodeId, SectorBounds, SectorId};

// ── Cluster ───────────────────────────────────────────────────────────────────

pub struct MultiNodeCluster {
    nodes: Vec<SectorSimulatorHandle>,
    bus  : ReplicationBusHandle,
}

impl MultiNodeCluster {
    /// Create a cluster of `node_count` nodes.
    ///
    /// Each node is assigned `SectorId(i)` and `NodeId(i)`.
    /// All nodes share the same `SectorBounds`.
    pub fn new(node_count: usize) -> Self {
        let bus   = ReplicationBusHandle::spawn();
        let nodes = (0..node_count as u8)
            .map(|i| SectorSimulatorHandle::spawn(
                NodeId(i),
                SectorId(i),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                bus.event_sender(),
            ))
            .collect();
        Self { nodes, bus }
    }

    /// Spawn `count` ships on every node using the given config.
    ///
    /// Counter offsets are staggered per node so `ShipId`s remain globally unique.
    pub async fn spawn_ships_on_all(&self, count: usize, config: &SpawnConfig) {
        for (idx, node) in self.nodes.iter().enumerate() {
            let offset = (idx * count) as u64;
            let ships  = generate_ships(count, config, offset);
            for (_, pos, vel) in ships {
                node.spawn_ship(pos, vel).await;
            }
        }
    }

    /// Tick every node once and return the per-node summaries.
    pub async fn tick_all(&self) -> Vec<TickSummary> {
        let mut results = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            results.push(node.tick().await);
        }
        results
    }

    /// Total events accumulated in the `ReplicationBus`.
    ///
    /// Deterministic: all events sent before this call are counted.
    pub async fn total_replicated_events(&self) -> usize {
        self.bus.event_count().await
    }

    /// Stats snapshot for every node.
    pub async fn get_all_stats(&self) -> Vec<NodeStats> {
        let mut stats = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            stats.push(node.get_stats().await);
        }
        stats
    }

    #[allow(dead_code)]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub async fn shutdown(self) {
        for node in &self.nodes {
            node.shutdown().await;
        }
        self.bus.shutdown().await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::NodeId;

    const NODES : usize = 3;
    const SHIPS : usize = 10;
    const TICKS : usize = 5;

    fn config() -> SpawnConfig {
        SpawnConfig::default_for_node(NodeId(0))
    }

    // ── Phase 2 completion test ───────────────────────────────────────────────

    #[tokio::test]
    async fn three_nodes_all_events_are_replicated_to_the_shared_bus() {
        let cluster = MultiNodeCluster::new(NODES);
        cluster.spawn_ships_on_all(SHIPS, &config()).await;

        for _ in 0..TICKS {
            cluster.tick_all().await;
        }

        let total = cluster.total_replicated_events().await;

        // ADR-0008: NPC ships at constant velocity emit no VelocityChanged events.
        // Each node: SHIPS spawns only (no VelocityChanged for constant-velocity NPCs).
        let per_node = SHIPS;
        let expected = NODES * per_node;

        assert_eq!(
            total, expected,
            "expected {expected} events ({NODES} nodes × {per_node} each: \
             {SHIPS} spawns only — NPC ships at constant velocity emit no VelocityChanged)"
        );

        cluster.shutdown().await;
    }

    // ── Per-node invariants ───────────────────────────────────────────────────

    #[tokio::test]
    async fn each_node_manages_an_independent_sector() {
        let cluster = MultiNodeCluster::new(NODES);
        let stats = cluster.get_all_stats().await;

        let sector_ids: Vec<_> = stats.iter().map(|s| s.sector_id).collect();
        let node_ids  : Vec<_> = stats.iter().map(|s| s.node_id).collect();

        // All Sector IDs must be distinct (each node owns a unique Sector).
        let unique_sectors: std::collections::HashSet<_> = sector_ids.iter().collect();
        assert_eq!(unique_sectors.len(), NODES, "sector IDs must be unique across nodes");

        let unique_nodes: std::collections::HashSet<_> = node_ids.iter().collect();
        assert_eq!(unique_nodes.len(), NODES, "node IDs must be unique");

        cluster.shutdown().await;
    }

    #[tokio::test]
    async fn tick_counter_advances_independently_per_node() {
        let cluster = MultiNodeCluster::new(NODES);
        cluster.spawn_ships_on_all(5, &config()).await;

        for _ in 0..TICKS {
            cluster.tick_all().await;
        }

        for stats in cluster.get_all_stats().await {
            assert_eq!(
                stats.current_tick,
                dawn_core::Tick(TICKS as u64),
                "node {} should be at Tick({})",
                stats.node_id.0,
                TICKS,
            );
        }

        cluster.shutdown().await;
    }

    #[tokio::test]
    async fn replicated_event_count_grows_monotonically_across_ticks() {
        let cluster = MultiNodeCluster::new(NODES);
        cluster.spawn_ships_on_all(5, &config()).await;

        let mut prev = cluster.total_replicated_events().await;
        for _ in 0..TICKS {
            cluster.tick_all().await;
            let current = cluster.total_replicated_events().await;
            assert!(current >= prev, "replicated event count must never decrease");
            prev = current;
        }

        cluster.shutdown().await;
    }
}
