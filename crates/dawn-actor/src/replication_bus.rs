//! `ReplicationBus` — collects events from all Sector nodes into a shared log.
//!
//! ## Ordering guarantee
//!
//! Events and queries share a **single channel** (`BusMessage`).
//! This ensures that when `event_count()` returns, every `Events` message
//! sent *before* the query has already been processed — no sleep/flush needed.
//!
//! ## Usage pattern
//!
//! ```text
//! let bus = ReplicationBusHandle::spawn();
//!
//! // Give each SectorSimulatorActor a clone of the sender.
//! let sender: mpsc::Sender<BusMessage> = bus.event_sender();
//!
//! // After all ticks complete:
//! let count = bus.event_count().await;   // deterministic, no sleep
//! ```

use dawn_core::DomainEvent;
use dawn_event_store::{store::EventStore, InMemoryEventStore};
use tokio::sync::{mpsc, oneshot};

// ── Message type ──────────────────────────────────────────────────────────────

/// All messages routed through the single bus channel.
///
/// `SectorSimulatorActor` sends `Events`.
/// `ReplicationBusHandle` sends queries.
pub enum BusMessage {
    /// A batch of domain events from one Sector node.
    Events(Vec<DomainEvent>),
    /// Query: total event count accumulated so far.
    EventCount { reply: oneshot::Sender<usize> },
    /// Shut down the bus actor cleanly.
    Shutdown,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

struct ReplicationBusActor {
    rx   : mpsc::Receiver<BusMessage>,
    store: InMemoryEventStore,
}

impl ReplicationBusActor {
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

/// Cloneable handle to a running `ReplicationBusActor`.
#[derive(Clone)]
pub struct ReplicationBusHandle {
    tx: mpsc::Sender<BusMessage>,
}

impl ReplicationBusHandle {
    /// Spawn a new `ReplicationBusActor` and return a handle.
    pub fn spawn() -> Self {
        // Large buffer: 3 nodes × 10,000 ships × N ticks worth of batches.
        let (tx, rx) = mpsc::channel(10_000);
        tokio::spawn(ReplicationBusActor::new(rx).run());
        Self { tx }
    }

    /// Returns a `Sender` for `SectorSimulatorActor`s to forward events.
    ///
    /// Callers send `BusMessage::Events(batch)` through this sender.
    pub fn event_sender(&self) -> mpsc::Sender<BusMessage> {
        self.tx.clone()
    }

    /// Total number of events accumulated in the bus since startup.
    ///
    /// Because events and this query share the same channel, this value is
    /// consistent with all events sent *before* this call.
    pub async fn event_count(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BusMessage::EventCount { reply: tx }).await
            .expect("ReplicationBusActor is no longer running");
        rx.await.expect("ReplicationBusActor dropped reply sender")
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

    fn events(count: usize) -> Vec<DomainEvent> {
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
        let bus = ReplicationBusHandle::spawn();
        assert_eq!(bus.event_count().await, 0);
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn events_sent_before_query_are_counted_correctly() {
        let bus = ReplicationBusHandle::spawn();
        let sender = bus.event_sender();

        sender.send(BusMessage::Events(events(5))).await.unwrap();
        sender.send(BusMessage::Events(events(3))).await.unwrap();

        // Single channel: both Events messages are ahead of this query.
        assert_eq!(bus.event_count().await, 8);
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn multiple_senders_all_contribute_to_event_count() {
        let bus = ReplicationBusHandle::spawn();

        // Simulate 3 nodes sending concurrently.
        let s1 = bus.event_sender();
        let s2 = bus.event_sender();
        let s3 = bus.event_sender();

        tokio::join!(
            async { s1.send(BusMessage::Events(events(10))).await.unwrap() },
            async { s2.send(BusMessage::Events(events(10))).await.unwrap() },
            async { s3.send(BusMessage::Events(events(10))).await.unwrap() },
        );

        assert_eq!(bus.event_count().await, 30);
        bus.shutdown().await;
    }
}
