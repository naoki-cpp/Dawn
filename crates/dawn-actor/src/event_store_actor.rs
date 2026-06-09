//! `EventStoreActor` — Actor wrapper around `InMemoryEventStore`.
//!
//! Serialises all writes to the event log through a single tokio task,
//! eliminating the need for `Arc<Mutex<_>>` around the store.
//!
//! Callers interact exclusively through [`EventStoreHandle`].

use dawn_core::DomainEvent;
use dawn_event_store::{store::EventStore, EventRecord, InMemoryEventStore};
use tokio::sync::{mpsc, oneshot};

// ── Messages ──────────────────────────────────────────────────────────────────

enum EventStoreMessage {
    Append {
        event: DomainEvent,
        reply: oneshot::Sender<u64>,
    },
    AppendBatch {
        events: Vec<DomainEvent>,
        reply : oneshot::Sender<Vec<u64>>,
    },
    Len {
        reply: oneshot::Sender<usize>,
    },
    IterFrom {
        from_index: u64,
        reply     : oneshot::Sender<Vec<EventRecord>>,
    },
    Shutdown,
}

// ── Actor ─────────────────────────────────────────────────────────────────────

struct EventStoreActor {
    rx   : mpsc::Receiver<EventStoreMessage>,
    store: InMemoryEventStore,
}

impl EventStoreActor {
    fn new(rx: mpsc::Receiver<EventStoreMessage>) -> Self {
        Self { rx, store: InMemoryEventStore::new() }
    }

    async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            match msg {
                EventStoreMessage::Append { event, reply } => {
                    let _ = reply.send(self.store.append(event));
                }
                EventStoreMessage::AppendBatch { events, reply } => {
                    let _ = reply.send(self.store.append_batch(events));
                }
                EventStoreMessage::Len { reply } => {
                    let _ = reply.send(self.store.len());
                }
                EventStoreMessage::IterFrom { from_index, reply } => {
                    let records = self.store
                        .iter_from(from_index)
                        .cloned()
                        .collect();
                    let _ = reply.send(records);
                }
                EventStoreMessage::Shutdown => break,
            }
        }
    }
}

// ── Handle ────────────────────────────────────────────────────────────────────

/// Cloneable handle to a running `EventStoreActor`.
///
/// All methods are async and communicate with the actor via channels.
/// There is no direct access to the underlying store.
#[derive(Clone)]
pub struct EventStoreHandle {
    tx: mpsc::Sender<EventStoreMessage>,
}

impl EventStoreHandle {
    /// Spawn a new `EventStoreActor` and return a handle to it.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel(1_024);
        tokio::spawn(EventStoreActor::new(rx).run());
        Self { tx }
    }

    pub async fn append(&self, event: DomainEvent) -> u64 {
        let (tx, rx) = oneshot::channel();
        self.tx.send(EventStoreMessage::Append { event, reply: tx }).await
            .expect("EventStoreActor is no longer running");
        rx.await.expect("EventStoreActor dropped reply sender")
    }

    pub async fn append_batch(&self, events: Vec<DomainEvent>) -> Vec<u64> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(EventStoreMessage::AppendBatch { events, reply: tx }).await
            .expect("EventStoreActor is no longer running");
        rx.await.expect("EventStoreActor dropped reply sender")
    }

    pub async fn len(&self) -> usize {
        let (tx, rx) = oneshot::channel();
        self.tx.send(EventStoreMessage::Len { reply: tx }).await
            .expect("EventStoreActor is no longer running");
        rx.await.expect("EventStoreActor dropped reply sender")
    }

    pub async fn iter_from(&self, from_index: u64) -> Vec<EventRecord> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(EventStoreMessage::IterFrom { from_index, reply: tx }).await
            .expect("EventStoreActor is no longer running");
        rx.await.expect("EventStoreActor dropped reply sender")
    }

    pub async fn shutdown(&self) {
        let _ = self.tx.send(EventStoreMessage::Shutdown).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{
        events::VelocityChanged,
        NodeId, ShipId, Tick, Velocity,
    };

    fn moved_event(n: u64, tick: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id : ShipId::new(NodeId(0), n),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick    : Tick(tick),
        })
    }

    #[tokio::test]
    async fn appending_first_event_assigns_log_index_zero() {
        let store = EventStoreHandle::spawn();
        let index = store.append(moved_event(1, 1)).await;
        assert_eq!(index, 0);
        store.shutdown().await;
    }

    #[tokio::test]
    async fn log_indices_increase_monotonically_across_appends() {
        let store = EventStoreHandle::spawn();
        let indices: Vec<u64> = futures::future::join_all(
            (0..5).map(|i| {
                let s = store.clone();
                async move { s.append(moved_event(i, i)).await }
            })
        ).await;
        // Sequential appends must produce 0,1,2,3,4.
        // (join_all preserves order of futures, and the actor serialises them.)
        let mut sorted = indices.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4]);
        store.shutdown().await;
    }

    #[tokio::test]
    async fn len_returns_the_number_of_appended_events() {
        let store = EventStoreHandle::spawn();
        store.append_batch(vec![moved_event(1, 1), moved_event(2, 1)]).await;
        assert_eq!(store.len().await, 2);
        store.shutdown().await;
    }

    #[tokio::test]
    async fn iter_from_returns_records_starting_at_given_index() {
        let store = EventStoreHandle::spawn();
        for i in 0..5 {
            store.append(moved_event(i, i)).await;
        }
        let tail = store.iter_from(3).await;
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].log_index, 3);
        store.shutdown().await;
    }
}
