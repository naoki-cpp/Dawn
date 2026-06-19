//! `MultiNodeCluster` — orchestrates N `SectorSimulatorActor`s + one `InMemoryReplicationBus`.
//!
//! This is the Phase 2 completion test harness.
//!
//! # Consistency model
//!
//! Each node has its own ECS World and local EventStore.
//! After every tick, events are forwarded to the shared `InMemoryReplicationBus`
//! *before* the tick reply is returned to the caller.
//!
//! Because `tick_all()` awaits all nodes sequentially (or via join), and
//! `total_replicated_events()` sends a query *after* all ticks complete,
//! the query is guaranteed to observe all events from all ticks.
//! No sleep, no flush, no barrier is required.

use crate::sector_simulator_actor::{NodeStats, SectorSimulatorHandle, TickSummary};
use dawn_sector::spawner::{generate_ships, SpawnConfig};
use dawn_replication::InMemoryReplicationBus;
use dawn_consensus::{InProcessTransport, PartitionableTransport, RaftActor, RaftActorHandle, RaftState, RaftTransport, Role, Term};
use dawn_core::{NodeId, SectorBounds, SectorId};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

// ── Raft wiring ───────────────────────────────────────────────────────────────

/// One node's Raft endpoints produced by [`spawn_raft_actors`]:
/// the proposal/timer handle and the committed-entries channel consumed at
/// Tick Step 7.5.
pub(crate) type RaftEndpoint = (RaftActorHandle, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>);

/// Spawn one `RaftActor` per `NodeId`, fully wired to its peers via
/// `PartitionableTransport` over in-process mpsc channels (ADR-0014).
///
/// Returns the per-node endpoints (in `ids` order) and the shared
/// fault-injection partition set. Election timeout = 10 + jitter(0..10)
/// ticks, heartbeat every 3 ticks; timers advance once per simulation Tick
/// (Step 10).
pub(crate) fn spawn_raft_actors(
    ids: &[NodeId],
) -> (Vec<RaftEndpoint>, Arc<Mutex<HashSet<NodeId>>>) {
    // One mailbox per RaftActor, addressed by NodeId.
    let mut raft_txs = HashMap::new();
    let mut raft_rxs = HashMap::new();
    for &id in ids {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        raft_txs.insert(id, tx);
        raft_rxs.insert(id, rx);
    }

    let partitioned = PartitionableTransport::new_partition_set();
    let mut rng = rand::thread_rng();

    let endpoints = ids.iter().map(|&id| {
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

        let state = RaftState::new_randomized(id, peers.clone(), 10, 10, 3, &mut rng);
        let raft_rx = raft_rxs.remove(&id).unwrap();
        let (committed_tx, committed_rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(RaftActor::new(state, peers, transport, raft_rx, committed_tx).run());
        (RaftActorHandle::new(raft_txs[&id].clone()), committed_rx)
    }).collect();

    (endpoints, partitioned)
}

// ── Cluster ───────────────────────────────────────────────────────────────────

pub struct MultiNodeCluster {
    nodes      : Vec<SectorSimulatorHandle>,
    bus        : InMemoryReplicationBus,
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
        let bus = InMemoryReplicationBus::spawn();
        let ids: Vec<NodeId> = (0..node_count as u8).map(NodeId).collect();

        let (endpoints, partitioned) = spawn_raft_actors(&ids);

        let nodes = ids.iter().zip(endpoints).map(|(&id, (raft, committed_rx))| {
            SectorSimulatorHandle::spawn(
                id,
                SectorId(id.0),
                SectorBounds::centered(SectorBounds::DEFAULT_HALF),
                bus.event_sender(),
                raft,
                committed_rx,
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

    /// Total events accumulated in the `InMemoryReplicationBus`.
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

    /// Per-node actor handles, in `NodeId` order (used by the `--raft-demo`
    /// binary mode to address individual nodes).
    pub fn nodes(&self) -> &[SectorSimulatorHandle] {
        &self.nodes
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

    #[tokio::test]
    async fn nodes_accessor_exposes_one_handle_per_node_in_id_order() {
        let cluster = MultiNodeCluster::new(NODES);
        assert_eq!(cluster.nodes().len(), NODES);
        cluster.shutdown().await;
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

    /// Full Raft Transit pipeline (ADR-0014 §3): a TransitCommand on the
    /// owning node travels through the Raft Log (Request → commit →
    /// export → Commit → commit → import) and the Ship ends up owned by
    /// the destination Sector only.
    #[tokio::test]
    async fn committed_transit_moves_ship_across_sectors_through_the_raft_log() {
        use dawn_core::{Position, Velocity};

        let cluster = MultiNodeCluster::new(NODES);

        // Elect a leader first so proposals are not dropped.
        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let ship_id = cluster.nodes[0]
            .spawn_ship(Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0))
            .await;

        let accepted = cluster.nodes[0].transit(ship_id, SectorId(1)).await;
        assert!(accepted, "transit command must pass up-front validation");

        // Two Raft rounds (Request, then Commit with the exported state)
        // ride on heartbeats every 3 ticks; 30 ticks is ample.
        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let stats = cluster.get_all_stats().await;
        assert_eq!(stats[0].ship_count, 0, "origin sector must no longer own the ship");
        assert_eq!(stats[1].ship_count, 1, "destination sector must own the ship");
        assert_eq!(stats[2].ship_count, 0, "third sector must be unaffected");

        cluster.shutdown().await;
    }

    /// Full Jump Gate pipeline (ADR-0009): a Ship within a Jump Gate's
    /// `activation_radius` can jump through it, and the resulting Transit
    /// rides the same Raft pipeline as `transit()` (Request -> commit ->
    /// export -> Commit -> commit -> import).
    #[tokio::test]
    async fn committed_jump_moves_ship_to_gates_destination_sector() {
        use dawn_core::{JumpGateId, Velocity};

        let cluster = MultiNodeCluster::new(NODES);

        // Elect a leader first so proposals are not dropped.
        for _ in 0..30 {
            cluster.tick_all().await;
        }

        // Gate 0 (Sector 0 -> Sector 1) sits near Sector 0's +X edge.
        let gate = dawn_sector::galaxy::Galaxy::builtin().gates
            .into_iter()
            .find(|g| g.id == JumpGateId(0))
            .expect("gate 0 must exist");
        assert_eq!(gate.from_sector, SectorId(0));
        assert_eq!(gate.to_sector, SectorId(1));

        let ship_id = cluster.nodes[0]
            .spawn_ship(gate.position, Velocity::new(0.0, 0.0, 0.0))
            .await;

        let accepted = cluster.nodes[0].jump(ship_id, gate.id).await;
        assert!(accepted, "jump command must pass up-front validation");

        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let stats = cluster.get_all_stats().await;
        assert_eq!(stats[0].ship_count, 0, "origin sector must no longer own the ship");
        assert_eq!(stats[1].ship_count, 1, "destination sector must own the ship");

        cluster.shutdown().await;
    }

    /// A Jump command for a ship outside the gate's activation_radius is
    /// rejected up front and never reaches the Raft Log (INV-006).
    #[tokio::test]
    async fn jump_command_for_ship_out_of_gate_range_is_rejected_without_proposing() {
        use dawn_core::{JumpGateId, Position, Velocity};

        let cluster = MultiNodeCluster::new(NODES);
        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let ship_id = cluster.nodes[0]
            .spawn_ship(Position::ORIGIN, Velocity::new(0.0, 0.0, 0.0))
            .await;

        let accepted = cluster.nodes[0].jump(ship_id, JumpGateId(0)).await;
        assert!(!accepted, "ship far from the gate must be rejected up front");

        cluster.shutdown().await;
    }

    /// A Transit command for a ship already in transit is rejected up front
    /// and never reaches the Raft Log (INV-006).
    #[tokio::test]
    async fn transit_command_for_unknown_ship_is_rejected_without_proposing() {
        let cluster = MultiNodeCluster::new(NODES);
        for _ in 0..30 {
            cluster.tick_all().await;
        }

        let bogus = dawn_core::ShipId::new(NodeId(9), 999);
        let accepted = cluster.nodes[0].transit(bogus, SectorId(1)).await;
        assert!(!accepted, "unknown ship must be rejected up front");

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

    /// Phase 7 completion criterion (ADR-0014 §3 / roadmap.md §10): a Sector
    /// Transit proposed while the old Leader is partitioned away still
    /// completes once the remaining nodes elect a new Leader.
    #[tokio::test]
    async fn transit_completes_after_a_new_leader_is_elected_during_node_failure() {
        use dawn_core::{Position, Velocity};

        let cluster = MultiNodeCluster::new(NODES);

        // Elect the first Leader.
        for _ in 0..30 {
            cluster.tick_all().await;
        }
        let roles = cluster.raft_roles().await;
        let (old_leader_idx, _) = roles.iter().enumerate()
            .find(|(_, (role, _))| *role == dawn_consensus::Role::Leader)
            .expect("a leader must exist before the partition");

        // Take the old Leader down before any Transit is proposed.
        cluster.partition_node(NodeId(old_leader_idx as u8));

        // Elect a new Leader among the remaining nodes.
        for _ in 0..30 {
            cluster.tick_all().await;
        }
        let roles = cluster.raft_roles().await;
        let (new_leader_idx, _) = roles.iter().enumerate()
            .find(|&(idx, (role, _))| idx != old_leader_idx && *role == dawn_consensus::Role::Leader)
            .expect("a new leader must be elected after the partition");

        // Propose a Transit owned by the new Leader's sector, away from the
        // partitioned node's sector (which can no longer import).
        let owner_idx = new_leader_idx;
        let dest_idx = (0..NODES).find(|&i| i != old_leader_idx && i != owner_idx)
            .expect("a third sector must exist for the destination");

        let ship_id = cluster.nodes[owner_idx]
            .spawn_ship(Position::ORIGIN, Velocity::new(1.0, 0.0, 0.0))
            .await;
        let accepted = cluster.nodes[owner_idx].transit(ship_id, SectorId(dest_idx as u8)).await;
        assert!(accepted, "transit command must pass up-front validation");

        for _ in 0..40 {
            cluster.tick_all().await;
        }

        let stats = cluster.get_all_stats().await;
        assert_eq!(stats[owner_idx].ship_count, 0, "origin sector must no longer own the ship");
        assert_eq!(stats[dest_idx].ship_count, 1, "destination sector must own the ship");
        assert_eq!(stats[old_leader_idx].ship_count, 0, "the partitioned sector must be unaffected");

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
