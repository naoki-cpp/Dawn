//! `SectorSimulatorActor` — async Actor wrapper around `SimulationNode`.
//!
//! Provides an async, message-passing interface to the synchronous
//! `SimulationNode`.  After each state-changing operation, new events are
//! forwarded to the `ReplicationBus` *before* the reply is sent to the caller.
//! This ordering guarantee makes multi-node consistency tests deterministic
//! without sleeps or explicit flush operations.
//!
//! # Ordering contract
//!
//! ```text
//! Actor loop:
//!   1. Execute operation (tick / spawn_ship)
//!   2. Send new events to ReplicationBus  ← must happen FIRST
//!   3. Send reply to caller               ← happens AFTER
//!
//! Consequence: by the time the caller's .await returns, all events are
//! already in the bus channel ahead of any subsequent query.
//! ```

use crate::node::{SimulationNode, TickResult};
use crate::transit::TransitOp;
use dawn_actor::BusMessage;
use dawn_consensus::{RaftActorHandle, Role, Term};
use dawn_event_store::store::EventStore as _;
use dawn_core::{NodeId, Position, SectorBounds, SectorId, ShipId, Tick, Velocity};
use tokio::sync::{mpsc, oneshot};

// ── Public result/stats types ─────────────────────────────────────────────────

/// Summary returned to the caller after a tick completes.
///
/// Does not include the raw event list (those are forwarded to the bus).
///
/// Read by the actor tests; the demo callers of `tick`/`tick_all` ignore it,
/// hence the non-test `dead_code` allowance.
#[derive(Debug, Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct TickSummary {
    pub tick          : Tick,
    pub events_emitted: usize,
}

/// Snapshot of a node's current state, returned by `GetStats`.
#[derive(Debug)]
pub struct NodeStats {
    pub node_id    : NodeId,
    pub sector_id  : SectorId,
    pub ship_count : usize,
    pub event_count: usize,
    pub current_tick: Tick,
}

// ── Messages ──────────────────────────────────────────────────────────────────

enum SectorSimulatorMessage {
    Tick {
        reply: oneshot::Sender<TickSummary>,
    },
    SpawnShip {
        position: Position,
        velocity: Velocity,
        reply   : oneshot::Sender<ShipId>,
    },
    GetStats {
        reply: oneshot::Sender<NodeStats>,
    },
    /// Query this node's Raft role/term (ADR-0014, for tests/observability).
    GetRaftRole {
        reply: oneshot::Sender<(Role, Term)>,
    },
    /// Request a Sector Transit for `ship_id` (ADR-0014).
    ///
    /// Validated locally, then proposed to the Raft Log; the actual state
    /// change happens only when the committed proposal is applied at
    /// Step 7.5. `reply` is `false` if the command was rejected up front.
    Transit {
        ship_id: ShipId,
        to     : SectorId,
        reply  : oneshot::Sender<bool>,
    },
    /// Request a Jump-Gate Transit for `ship_id` via `gate_id` (ADR-0009).
    ///
    /// Test-only: the production `--serve --cluster` loop routes jumps
    /// through the direct `SimulationNode` path in `main.rs`, not this
    /// actor. Validated locally (Ship in range of the gate, not already in
    /// transit), then proposed to the Raft Log via the same `TransitOp`
    /// pipeline as [`SectorSimulatorMessage::Transit`]. `reply` is `false`
    /// if the command was rejected up front.
    #[cfg(test)]
    Jump {
        ship_id: ShipId,
        gate_id: dawn_core::JumpGateId,
        reply  : oneshot::Sender<bool>,
    },
    Shutdown,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

struct SectorSimulatorActor {
    rx              : mpsc::Receiver<SectorSimulatorMessage>,
    node            : SimulationNode,
    bus_tx          : mpsc::Sender<BusMessage>,
    /// Index of the next event in the node's log that has not yet been
    /// forwarded to the ReplicationBus.
    last_replicated : u64,
    /// Handle to this node's RaftActor (ADR-0014).
    raft            : RaftActorHandle,
    /// Committed Raft Log payloads from this node's RaftActor, applied at
    /// Step 7.5 each Tick (ADR-0014 §7).
    raft_committed_rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl SectorSimulatorActor {
    fn new(
        rx    : mpsc::Receiver<SectorSimulatorMessage>,
        node  : SimulationNode,
        bus_tx: mpsc::Sender<BusMessage>,
        raft  : RaftActorHandle,
        raft_committed_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Self {
        Self { rx, node, bus_tx, last_replicated: 0, raft, raft_committed_rx }
    }

    /// Step 7.5 (ADR-0014 §7): apply committed Raft Log entries to the ECS.
    ///
    /// Delegates to [`crate::transit::apply_committed_raft_entries`], which
    /// is shared with the `--serve --cluster` loop.
    fn apply_committed_raft_entries(&mut self) {
        crate::transit::apply_committed_raft_entries(
            &mut self.node,
            &self.raft,
            &mut self.raft_committed_rx,
        );
    }

    /// Forward any un-replicated events from the node's local log to the bus.
    async fn flush_to_bus(&mut self) {
        let new_events: Vec<_> = self.node
            .event_store()
            .iter_from(self.last_replicated)
            .map(|r| r.event.clone())
            .collect();

        if !new_events.is_empty() {
            self.last_replicated += new_events.len() as u64;
            // Ignore send error — bus may have been shut down in tests.
            let _ = self.bus_tx.send(BusMessage::Events(new_events)).await;
        }
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                SectorSimulatorMessage::Tick { reply } => {
                    // Step 7.5 (ADR-0014): apply committed Raft entries.
                    // Applied before the simulation step so the resulting
                    // events are flushed to the bus in the same Tick.
                    self.apply_committed_raft_entries();

                    let result: TickResult = self.node.tick();
                    let summary = TickSummary {
                        tick          : result.tick,
                        events_emitted: result.events_emitted,
                    };

                    // Step 2: replicate BEFORE replying (ordering contract).
                    // flush_to_bus covers both the Step 7.5 Transit events
                    // and the events emitted by the simulation step.
                    self.flush_to_bus().await;

                    // Step 10: advance this node's Raft election/heartbeat
                    // timers by one logical Tick (INV-005 / FBD-003).
                    self.raft.tick();

                    // Step 3: reply to caller.
                    let _ = reply.send(summary);
                }

                SectorSimulatorMessage::SpawnShip { position, velocity, reply } => {
                    let ship_id = self.node.spawn_ship(dawn_core::ShipTypeId(1), position, velocity);

                    // Spawn appends to the internal log; flush those events.
                    self.flush_to_bus().await;

                    let _ = reply.send(ship_id);
                }

                SectorSimulatorMessage::GetStats { reply } => {
                    let _ = reply.send(NodeStats {
                        node_id     : self.node.node_id(),
                        sector_id   : self.node.sector_id(),
                        ship_count  : self.node.ship_count(),
                        event_count : self.node.total_event_count(),
                        current_tick: self.node.current_tick(),
                    });
                }

                SectorSimulatorMessage::GetRaftRole { reply } => {
                    let (role, term) = self.raft.role().await;
                    let _ = reply.send((role, term));
                }

                SectorSimulatorMessage::Transit { ship_id, to, reply } => {
                    // Up-front validation only (INV-006: rejection produces
                    // no event). The ECS is untouched until the proposal
                    // commits and is applied at Step 7.5.
                    let accepted = self.node.can_propose_transit(ship_id);
                    if accepted {
                        self.raft.propose(TransitOp::Request { ship_id, to, gate_id: None }.encode());
                    }
                    let _ = reply.send(accepted);
                }

                #[cfg(test)]
                SectorSimulatorMessage::Jump { ship_id, gate_id, reply } => {
                    // Up-front validation only (INV-006): Ship must exist,
                    // not be in transit, and be within the gate's
                    // activation_radius (ADR-0009).
                    let accepted = self.node.can_propose_jump(ship_id, gate_id);
                    if accepted {
                        let to = self.node.jump_gate(gate_id)
                            .expect("can_propose_jump confirmed gate exists")
                            .to_sector;
                        self.raft.propose(TransitOp::Request { ship_id, to, gate_id: Some(gate_id) }.encode());
                    }
                    let _ = reply.send(accepted);
                }

                SectorSimulatorMessage::Shutdown => break,
            }
        }
    }
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Cloneable handle to a running `SectorSimulatorActor`.
#[derive(Clone)]
pub struct SectorSimulatorHandle {
    tx: mpsc::Sender<SectorSimulatorMessage>,
}

impl SectorSimulatorHandle {
    /// Spawn a `SectorSimulatorActor` and return a handle.
    ///
    /// `bus` provides the replication channel.  Pass `bus.event_sender()` here.
    /// `raft` is this node's `RaftActorHandle` (ADR-0014), already wired to
    /// its peers via `RaftTransport`. `raft_committed_rx` is the matching
    /// committed-entries channel from the same `RaftActor`.
    pub fn spawn(
        node_id  : NodeId,
        sector_id: SectorId,
        bounds   : SectorBounds,
        bus_tx   : mpsc::Sender<BusMessage>,
        raft     : RaftActorHandle,
        raft_committed_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let node = SimulationNode::new(node_id, sector_id, bounds);
        tokio::spawn(SectorSimulatorActor::new(rx, node, bus_tx, raft, raft_committed_rx).run());
        Self { tx }
    }

    pub async fn tick(&self) -> TickSummary {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SectorSimulatorMessage::Tick { reply: tx }).await
            .expect("SectorSimulatorActor is no longer running");
        rx.await.expect("SectorSimulatorActor dropped reply sender")
    }

    pub async fn spawn_ship(&self, position: Position, velocity: Velocity) -> ShipId {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SectorSimulatorMessage::SpawnShip { position, velocity, reply: tx }).await
            .expect("SectorSimulatorActor is no longer running");
        rx.await.expect("SectorSimulatorActor dropped reply sender")
    }

    pub async fn get_stats(&self) -> NodeStats {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SectorSimulatorMessage::GetStats { reply: tx }).await
            .expect("SectorSimulatorActor is no longer running");
        rx.await.expect("SectorSimulatorActor dropped reply sender")
    }

    /// Request a Sector Transit (ADR-0014). Returns `false` if rejected up
    /// front (unknown Ship / already in transit). Acceptance only means the
    /// proposal was submitted to Raft; the move happens once it commits.
    pub async fn transit(&self, ship_id: ShipId, to: SectorId) -> bool {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SectorSimulatorMessage::Transit { ship_id, to, reply: tx }).await
            .expect("SectorSimulatorActor is no longer running");
        rx.await.expect("SectorSimulatorActor dropped reply sender")
    }

    /// Request a Jump-Gate Transit (ADR-0009). Test-only (see
    /// [`SectorSimulatorMessage::Jump`]). Returns `false` if rejected up
    /// front (unknown Ship, already in transit, unknown gate, or Ship out of
    /// range). Acceptance only means the proposal was submitted to Raft; the
    /// move happens once it commits.
    #[cfg(test)]
    pub async fn jump(&self, ship_id: ShipId, gate_id: dawn_core::JumpGateId) -> bool {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SectorSimulatorMessage::Jump { ship_id, gate_id, reply: tx }).await
            .expect("SectorSimulatorActor is no longer running");
        rx.await.expect("SectorSimulatorActor dropped reply sender")
    }

    /// Current Raft role/term of this node (ADR-0014).
    pub async fn raft_role(&self) -> (Role, Term) {
        let (tx, rx) = oneshot::channel();
        self.tx.send(SectorSimulatorMessage::GetRaftRole { reply: tx }).await
            .expect("SectorSimulatorActor is no longer running");
        rx.await.expect("SectorSimulatorActor dropped reply sender")
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(SectorSimulatorMessage::Shutdown).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_actor::ReplicationBusHandle;
    use dawn_core::{SectorBounds, Velocity};

    fn spawn_actor() -> (SectorSimulatorHandle, dawn_actor::ReplicationBusHandle) {
        let bus = dawn_actor::ReplicationBusHandle::spawn();

        // Single-node Raft cluster: no peers, transport delivers nowhere.
        let (raft_tx, raft_rx) = mpsc::unbounded_channel();
        let (committed_tx, committed_rx) = mpsc::unbounded_channel();
        let transport = std::sync::Arc::new(dawn_consensus::InProcessTransport::new(
            std::collections::HashMap::new(),
        ));
        let state = dawn_consensus::RaftState::new(NodeId(0), vec![], 10, 2);
        tokio::spawn(dawn_consensus::RaftActor::new(state, vec![], transport, raft_rx, committed_tx).run());
        let raft = RaftActorHandle::new(raft_tx);

        let handle = SectorSimulatorHandle::spawn(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            bus.event_sender(),
            raft,
            committed_rx,
        );
        (handle, bus)
    }

    #[tokio::test]
    async fn actor_starts_with_tick_zero() {
        let (actor, bus) = spawn_actor();
        let stats = actor.get_stats().await;
        assert_eq!(stats.current_tick, Tick::ZERO);
        actor.shutdown().await;
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn spawning_a_ship_is_reflected_in_stats() {
        let (actor, bus) = spawn_actor();
        actor.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0)).await;
        let stats = actor.get_stats().await;
        assert_eq!(stats.ship_count, 1);
        actor.shutdown().await;
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn ticking_advances_the_logical_tick() {
        let (actor, bus) = spawn_actor();
        actor.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0)).await;
        let summary = actor.tick().await;
        assert_eq!(summary.tick, Tick(1));
        actor.shutdown().await;
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn moving_ships_produce_events_forwarded_to_bus() {
        // NPC ships at constant velocity emit no VelocityChanged (ADR-0008).
        // We test that the bus correctly receives spawn events, and that
        // ticking a stationary NPC produces no extra events.
        let (actor, bus) = spawn_actor();
        actor.spawn_ship(Position::ORIGIN, Velocity::ZERO).await;

        // Spawn event is in bus. Tick a stationary NPC → no VelocityChanged.
        let summary = actor.tick().await;
        assert_eq!(summary.events_emitted, 0, "stationary NPC emits no events");

        let count = bus.event_count().await;
        assert_eq!(count, 1, "only the spawn event should be in the bus");

        actor.shutdown().await;
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn spawn_events_are_replicated_before_reply_is_sent() {
        let (actor, bus) = spawn_actor();

        for i in 0..5 {
            actor.spawn_ship(
                Position::new(i as f32 * 50.0, 0.0, 0.0),
                Velocity::ZERO,
            ).await;
        }

        // All spawn events must be in the bus because spawn_ship awaits the reply,
        // and the reply is sent AFTER flush_to_bus().
        assert_eq!(bus.event_count().await, 5);

        actor.shutdown().await;
        bus.shutdown().await;
    }
}
