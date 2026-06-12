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
use dawn_consensus::{InProcessTransport, PartitionableTransport, RaftActor, RaftActorHandle, RaftState, RaftTransport, Role, Term};
use dawn_core::{NodeId, SectorBounds, SectorId};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

// ── Cluster ───────────────────────────────────────────────────────────────────

pub struct MultiNodeCluster {
    nodes      : Vec<SectorSimulatorHandle>,
    bus        : ReplicationBusHandle,
    /// Shared fault-injection set for the cluster's Raft transports (ADR-0014).
    partitioned: Arc<Mutex<HashSet<NodeId>>>,
}

impl MultiNodeCluster {
    /// Create a cluster of `node_count` nodes.
    ///
    /// Each node is assigned `SectorId(i)` and `NodeId(i)`.
    /// All nodes share the same `SectorBounds`.
    ///
    /// Each node also gets a `RaftActor` (ADR-0014), wired to its peers via
    /// `PartitionableTransport` over in-process mpsc channels. Election
    /// timeout/heartbeat timers advance once per simulation Tick (Step 10).
    pub fn new(node_count: usize) -> Self {
        let bus = ReplicationBusHandle::spawn();
        let ids: Vec<NodeId> = (0..node_count as u8).map(NodeId).collect();

        // One mailbox per RaftActor, addressed by NodeId.
        let mut raft_txs = HashMap::new();
        let mut raft_rxs = HashMap::new();
        for &id in &ids {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            raft_txs.insert(id, tx);
            raft_rxs.insert(id, rx);
        }

        let partitioned = PartitionableTransport::new_partition_set();
        let mut rng = rand::thread_rng();

        let nodes = ids.iter().map(|&id| {
            let peers: Vec<NodeId> = ids.iter().copied().filter(|&p| p != id).collect();

            let peer_txs: HashMap<NodeId, _> = raft_txs.iter()
                .filter(|&(&p, _)| p != id)
                .map(|(&p, tx)| (p, tx.clone()))
                .collect();
            let transport: Arc<dyn RaftTransport> = Arc::new(PartitionableTransport::new(
                id,
                InProcessTransport::new(peer_txs),
                partitioned.clone(),
            ));

            // election_timeout = 10 + jitter(0..10) ticks, heartbeat every 3 ticks.
            let state = RaftState::new_randomized(id, peers.clone(), 10, 10, 3, &mut rng);
            let raft_rx = raft_rxs.remove(&id).unwrap();
            tokio::spawn(RaftActor::new(state, peers, transport, raft_rx).run());
            let raft = RaftActorHandle::new(raft_txs[&id].clone());

            SectorSimulatorHandle::spawn(
                id,
                SectorId(id.0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                bus.event_sender(),
                raft,
            )
        }).collect();

        Self { nodes, bus, partitioned }
    }

    /// Cut `node` off from the rest of the cluster's Raft messages
    /// (ADR-0014 fault injection — simulates a node failure for election
    /// timeout/leader-failover testing).
    pub fn partition_node(&self, node: NodeId) {
        PartitionableTransport::partition(&self.partitioned, node);
    }

    /// Restore `node`'s Raft connectivity.
    pub fn heal_node(&self, node: NodeId) {
        PartitionableTransport::heal(&self.partitioned, node);
    }

    /// Current Raft role/term of every node, in node order.
    pub async fn raft_roles(&self) -> Vec<(Role, Term)> {
        let mut roles = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            roles.push(node.raft_role().await);
        }
        roles
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

    // ── Raft wiring (ADR-0014) ────────────────────────────────────────────────

    #[tokio::test]
    async fn ticking_the_cluster_eventually_elects_exactly_one_leader() {
        let cluster = MultiNodeCluster::new(NODES);

        // election_timeout is at most 10 + 10 = 20 ticks; give it ample margin.
        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let roles = cluster.raft_roles().await;
        let leaders = roles.iter().filter(|(role, _)| *role == dawn_consensus::Role::Leader).count();
        assert_eq!(leaders, 1, "exactly one node should be Leader after enough ticks: {roles:?}");

        let terms: std::collections::HashSet<_> = roles.iter().map(|(_, term)| *term).collect();
        assert_eq!(terms.len(), 1, "all nodes should agree on the term: {roles:?}");

        cluster.shutdown().await;
    }

    /// Leader-failure scenario (ADR-0014): when the current Leader is
    /// partitioned away, the remaining nodes must elect a new Leader in a
    /// higher term, and there must never be more than one Leader at once.
    #[tokio::test]
    async fn cluster_elects_a_new_leader_after_the_current_leader_is_partitioned() {
        let cluster = MultiNodeCluster::new(NODES);

        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let roles = cluster.raft_roles().await;
        let (leader_idx, (_, old_term)) = roles.iter().enumerate()
            .find(|(_, (role, _))| *role == dawn_consensus::Role::Leader)
            .expect("a leader must exist before the partition");
        let old_term = *old_term;

        cluster.partition_node(NodeId(leader_idx as u8));

        for _ in 0..30 {
            cluster.tick_all().await;

            // Split-brain absence: among nodes that can still communicate,
            // at most one Leader at any tick. The partitioned old leader is
            // excluded — it cannot know it has been deposed and correctly
            // keeps believing it is Leader in its stale term, but it can no
            // longer replicate to a majority, so this is not split-brain.
            let roles = cluster.raft_roles().await;
            let leaders = roles.iter().enumerate()
                .filter(|&(idx, (role, _))| idx != leader_idx && *role == dawn_consensus::Role::Leader)
                .count();
            assert!(leaders <= 1, "at most one connected Leader at any time: {roles:?}");
        }

        let roles = cluster.raft_roles().await;
        let leaders: Vec<_> = roles.iter().enumerate()
            .filter(|&(idx, (role, _))| idx != leader_idx && *role == dawn_consensus::Role::Leader)
            .collect();
        assert_eq!(leaders.len(), 1, "remaining nodes must elect a new leader: {roles:?}");
        let (new_leader_idx, (_, new_term)) = leaders[0];
        assert_ne!(new_leader_idx, leader_idx, "the partitioned node cannot be the new leader");
        assert!(*new_term > old_term, "new leader must be in a higher term: old={old_term:?} new={new_term:?}");

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
