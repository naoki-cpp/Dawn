//! `InMemoryReplicationBus` — single-process event broadcast.

use crate::{CatchUpMessage, CatchUpTransport, LogBatch, ReplicationTransport};
use dawn_event_store::{store::EventStore, InMemoryEventStore};
use tokio::sync::{broadcast, mpsc, oneshot};

#[derive(Debug)]
pub enum BusMessage {
    Batch(LogBatch),
    EventCount { reply: oneshot::Sender<usize> },
    Shutdown,
}

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

#[derive(Debug, Clone)]
pub struct InMemoryReplicationBus {
    tx: mpsc::Sender<BusMessage>,
    broadcast_tx: broadcast::Sender<LogBatch>,
    catch_up_tx: broadcast::Sender<CatchUpMessage>,
}

impl InMemoryReplicationBus {
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

    pub fn event_sender(&self) -> mpsc::Sender<BusMessage> {
        self.tx.clone()
    }

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

    #[tokio::test]
    async fn event_count_tracks_only_log_batches() {
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

    #[tokio::test]
    async fn subscribers_receive_log_batches() {
        let bus = InMemoryReplicationBus::spawn();
        let mut rx = bus.subscribe();
        let batch = make_batch(SectorId(7), 42, 2);
        bus.broadcast(batch.clone());
        assert_eq!(rx.recv().await.unwrap(), batch);
        bus.shutdown().await;
    }
}
