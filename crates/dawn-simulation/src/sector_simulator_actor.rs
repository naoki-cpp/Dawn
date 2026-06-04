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
use dawn_actor::BusMessage;
use dawn_event_store::store::EventStore as _;
use dawn_core::{NodeId, Position, SectorBounds, SectorId, ShipId, Tick, Velocity};
use tokio::sync::{mpsc, oneshot};

// ── Public result/stats types ─────────────────────────────────────────────────

/// Summary returned to the caller after a tick completes.
///
/// Does not include the raw event list (those are forwarded to the bus).
#[derive(Debug, Clone)]
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
}

impl SectorSimulatorActor {
    fn new(
        rx    : mpsc::Receiver<SectorSimulatorMessage>,
        node  : SimulationNode,
        bus_tx: mpsc::Sender<BusMessage>,
    ) -> Self {
        Self { rx, node, bus_tx, last_replicated: 0 }
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
                    let result: TickResult = self.node.tick();
                    let summary = TickSummary {
                        tick          : result.tick,
                        events_emitted: result.events_emitted,
                    };

                    // Step 2: replicate BEFORE replying (ordering contract).
                    if !result.events.is_empty() {
                        self.last_replicated += result.events.len() as u64;
                        let _ = self.bus_tx
                            .send(BusMessage::Events(result.events))
                            .await;
                    }

                    // Step 3: reply to caller.
                    let _ = reply.send(summary);
                }

                SectorSimulatorMessage::SpawnShip { position, velocity, reply } => {
                    let ship_id = self.node.spawn_ship(position, velocity);

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
    pub fn spawn(
        node_id  : NodeId,
        sector_id: SectorId,
        bounds   : SectorBounds,
        bus_tx   : mpsc::Sender<BusMessage>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let node = SimulationNode::new(node_id, sector_id, bounds);
        tokio::spawn(SectorSimulatorActor::new(rx, node, bus_tx).run());
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
        let bus    = dawn_actor::ReplicationBusHandle::spawn();
        let handle = SectorSimulatorHandle::spawn(
            NodeId(0),
            SectorId(0),
            SectorBounds::cube(SectorBounds::DEFAULT_SIZE),
            bus.event_sender(),
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
        let (actor, bus) = spawn_actor();
        actor.spawn_ship(Position::new(100.0, 100.0, 100.0), Velocity::new(1.0, 0.0, 0.0)).await;

        // Spawn event is already in bus. Now tick.
        let summary = actor.tick().await;
        assert_eq!(summary.events_emitted, 1);

        // Query bus AFTER tick reply — guaranteed to see the move event.
        let count = bus.event_count().await;
        assert_eq!(count, 2, "1 spawn + 1 move");

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
