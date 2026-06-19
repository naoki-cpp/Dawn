//! `InMemoryReplicationBus` — single-process event broadcast.
//!
//! Replaces `dawn_actor::ReplicationBus` (ADR-0027 / 8D-2a).
//!
//! All Sector nodes share a single channel (`BusMessage`).
//! Events and queries are ordered through this channel so that
//! `event_count()` reflects every `Events` message sent before it — no
//! sleep or explicit flush needed in tests.
//!
//! When `TcpReplicationTransport` (8D-2c) is ready, callers switch to that
//! implementation behind the same logical interface.

use dawn_core::DomainEvent;
use dawn_event_store::{store::EventStore, InMemoryEventStore};
use tokio::sync::{mpsc, oneshot};

// ── Message type ──────────────────────────────────────────────────────────────

/// Messages routed through the bus channel.
pub enum BusMessage {
    /// A batch of domain events from one Sector node.
    Events(Vec<DomainEvent>),
    /// Query: total event count accumulated so far.
    EventCount { reply: oneshot::Sender<usize> },
    /// Shut down the bus actor cleanly.
    Shutdown,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

struct BusActor {
    rx   : mpsc::Receiver<BusMessage>,
    store: InMemoryEventStore,
}

impl BusActor {
    fn new(rx: mpsc::Receiver<BusMessage>) -> Self {
        Self { rx, store: InMemoryEventStore::new() }
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                BusMessage::Events(events) => {
                    self.store.append_batch(events);
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
#[derive(Clone)]
pub struct InMemoryReplicationBus {
    tx: mpsc::Sender<BusMessage>,
}

impl InMemoryReplicationBus {
    /// Spawn a new bus actor and return a handle.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel(10_000);
        tokio::spawn(BusActor::new(rx).run());
        Self { tx }
    }

    /// Returns a `Sender` for Sector actors to forward their event batches.
    pub fn event_sender(&self) -> mpsc::Sender<BusMessage> {
        self.tx.clone()
    }

    /// Total events accumulated since startup.
    ///
    /// Consistent with all `Events` messages sent before this call because
    /// they share the same channel.
    pub async fn event_count(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BusMessage::EventCount { reply: tx }).await
            .expect("BusActor is no longer running");
        rx.await.expect("BusActor dropped reply sender")
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(BusMessage::Shutdown).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, NodeId, ShipId, Tick, Velocity};

    fn make_events(count: usize) -> Vec<DomainEvent> {
        (0..count)
            .map(|i| DomainEvent::VelocityChanged(VelocityChanged {
                ship_id : ShipId::new(NodeId(0), i as u64),
                velocity: Velocity::new(1.0, 0.0, 0.0),
                tick    : Tick(1),
            }))
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

        sender.send(BusMessage::Events(make_events(5))).await.unwrap();
        sender.send(BusMessage::Events(make_events(3))).await.unwrap();

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
            async { s1.send(BusMessage::Events(make_events(10))).await.unwrap() },
            async { s2.send(BusMessage::Events(make_events(10))).await.unwrap() },
            async { s3.send(BusMessage::Events(make_events(10))).await.unwrap() },
        );

        assert_eq!(bus.event_count().await, 30);
        bus.shutdown().await;
    }
}
