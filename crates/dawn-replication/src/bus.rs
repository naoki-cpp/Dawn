//! `InMemoryReplicationBus` — single-process event broadcast.
//!
//! Replaces `dawn_actor::ReplicationBus` (ADR-0027 / 8D-2a).
//!
//! All Sector nodes share a single channel (`BusMessage`).
//! Events and queries are ordered through this channel so that
//! `event_count()` reflects every `Batch` message sent before it — no
//! sleep or explicit flush needed in tests.
//!
//! TCP gossip callers can switch to `TcpReplicationTransport` behind the same
//! logical interface. Catch-up control traffic uses a separate bounded
//! broadcast channel so it does not affect event ordering or `event_count()`.

use crate::{CatchUpMessage, CatchUpTransport, LogBatch, ReplicationTransport};
use dawn_event_store::{store::EventStore, InMemoryEventStore};
use tokio::sync::{broadcast, mpsc, oneshot};

// ── Message type ──────────────────────────────────────────────────────────────

/// Messages routed through the bus channel.
#[derive(Debug)]
pub enum BusMessage {
    /// A batch of domain events from one Sector node's append-only log.
    Batch(LogBatch),
    /// Query: total event count accumulated so far.
    EventCount { reply: oneshot::Sender<usize> },
    /// Shut down the bus actor cleanly.
    Shutdown,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

struct BusActor {
    rx: mpsc::Receiver<BusMessage>,
    broadcast_tx: broadcast::Sender<LogBatch>,
    store: InMemoryEventStore,
}

impl BusActor {
    fn new(rx: mpsc::Receiver<BusMessage>, broadcast_tx: broadcast::Sender<LogBatch>) -> Self {
        Self {
            rx,
            broadcast_tx,
            store: InMemoryEventStore::new(),
        }
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                BusMessage::Batch(batch) => {
                    self.store.append_batch(batch.events.iter().cloned());
                    let _ = self.broadcast_tx.send(batch);
                }
                BusMessage::EventCount { reply } => {
                    let _ = reply.send(self.store.len());
                }
                BusMessage::Shutdown => break,
            }
        }
    }
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Cloneable handle to a running `BusActor`.
///
/// Drop-in replacement for the removed `dawn_actor::ReplicationBusHandle`.
#[derive(Debug, Clone)]
pub struct InMemoryReplicationBus {
    tx: mpsc::Sender<BusMessage>,
    broadcast_tx: broadcast::Sender<LogBatch>,
    catch_up_tx: broadcast::Sender<CatchUpMessage>,
}

impl InMemoryReplicationBus {
    /// Spawn a new bus actor and return a handle.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel(10_000);
        let (broadcast_tx, _) = broadcast::channel(10_000);
        let (catch_up_tx, _) = broadcast::channel(10_000);
        tokio::spawn(BusActor::new(rx, broadcast_tx.clone()).run());
        Self {
            tx,
            broadcast_tx,
            catch_up_tx,
        }
    }

    /// Returns a `Sender` for Sector actors to forward their event batches.
    pub fn event_sender(&self) -> mpsc::Sender<BusMessage> {
        self.tx.clone()
    }

    /// Total events accumulated since startup.
    ///
    /// Consistent with all `Batch` messages sent before this call because
    /// they share the same channel.
    pub async fn event_count(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(BusMessage::EventCount { reply: tx })
            .await
            .expect("BusActor is no longer running");
        rx.await.expect("BusActor dropped reply sender")
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(BusMessage::Shutdown).await;
    }
}

impl ReplicationTransport for InMemoryReplicationBus {
    fn broadcast(&self, batch: LogBatch) {
        let _ = self.tx.try_send(BusMessage::Batch(batch));
    }

    fn subscribe(&self) -> broadcast::Receiver<LogBatch> {
        self.broadcast_tx.subscribe()
    }
}

impl CatchUpTransport for InMemoryReplicationBus {
    fn send_catch_up(&self, message: CatchUpMessage) {
        let _ = self.catch_up_tx.send(message);
    }

    fn subscribe_catch_up(&self) -> broadcast::Receiver<CatchUpMessage> {
        self.catch_up_tx.subscribe()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CatchUpRequest;
    use dawn_core::{events::VelocityChanged, NodeId, SectorId, ShipId, Tick, Velocity};

    fn make_batch(sector_id: SectorId, from_index: u64, count: usize) -> LogBatch {
        let events = (0..count)
            .map(|i| {
                dawn_core::DomainEvent::VelocityChanged(VelocityChanged {
                    ship_id: ShipId::new(NodeId(0), i as u64),
                    velocity: Velocity::new(1.0, 0.0, 0.0),
                    tick: Tick(1),
                })
            })
            .collect();
        LogBatch::new(sector_id, from_index, events)
    }

    fn make_events(count: usize) -> Vec<dawn_core::DomainEvent> {
        (0..count)
            .map(|i| {
                dawn_core::DomainEvent::VelocityChanged(VelocityChanged {
                    ship_id: ShipId::new(NodeId(0), i as u64),
                    velocity: Velocity::new(1.0, 0.0, 0.0),
                    tick: Tick(1),
                })
            })
            .collect()
    }

    #[tokio::test]
    async fn event_count_is_zero_before_any_events_are_sent() {
        let bus = InMemoryReplicationBus::spawn();
        assert_eq!(bus.event_count().await, 0);
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn events_sent_before_query_are_counted_correctly() {
        let bus = InMemoryReplicationBus::spawn();
        let sender = bus.event_sender();

        sender
            .send(BusMessage::Batch(LogBatch::new(
                SectorId(0),
                0,
                make_events(5),
            )))
            .await
            .unwrap();
        sender
            .send(BusMessage::Batch(LogBatch::new(
                SectorId(0),
                5,
                make_events(3),
            )))
            .await
            .unwrap();

        assert_eq!(bus.event_count().await, 8);
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn multiple_senders_all_contribute_to_event_count() {
        let bus = InMemoryReplicationBus::spawn();

        let s1 = bus.event_sender();
        let s2 = bus.event_sender();
        let s3 = bus.event_sender();

        tokio::join!(
            async {
                s1.send(BusMessage::Batch(LogBatch::new(
                    SectorId(0),
                    0,
                    make_events(10),
                )))
                .await
                .unwrap()
            },
            async {
                s2.send(BusMessage::Batch(LogBatch::new(
                    SectorId(1),
                    10,
                    make_events(10),
                )))
                .await
                .unwrap()
            },
            async {
                s3.send(BusMessage::Batch(LogBatch::new(
                    SectorId(2),
                    20,
                    make_events(10),
                )))
                .await
                .unwrap()
            },
        );

        assert_eq!(bus.event_count().await, 30);
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn subscribers_receive_log_batches_with_sector_and_index() {
        let bus = InMemoryReplicationBus::spawn();
        let mut rx = bus.subscribe();
        let batch = make_batch(SectorId(7), 42, 2);

        bus.broadcast(batch.clone());

        let received = rx.recv().await.unwrap();
        assert_eq!(received, batch);
        assert_eq!(received.next_index(), 44);

        bus.shutdown().await;
    }

    #[tokio::test]
    async fn catch_up_control_does_not_change_event_count() {
        let bus = InMemoryReplicationBus::spawn();
        let mut control_rx = bus.subscribe_catch_up();

        bus.broadcast(make_batch(SectorId(0), 0, 5));
        bus.send_catch_up(CatchUpMessage::Request(CatchUpRequest {
            request_id: 1,
            requester_sector_id: SectorId(1),
            owner_sector_id: SectorId(0),
            from_index: 0,
            max_events: 10,
        }));

        assert!(matches!(
            control_rx.recv().await.unwrap(),
            CatchUpMessage::Request(_)
        ));
        assert_eq!(bus.event_count().await, 5);
        bus.shutdown().await;
    }
}
