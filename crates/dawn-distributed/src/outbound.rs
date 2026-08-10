//! Outbound append-log publishing for Sector owners (ADR-0027).
//!
//! This module owns the sender-side cursor and `LogBatch` construction so
//! callers do not need to know log indices or transport payload shape.

use crate::{LogBatch, ReplicationTransport};
use dawn_core::SectorId;
use dawn_storage::EventStore;

/// Publishes the newly appended suffix of one owner's event log.
///
/// The publisher keeps the next un-published log index. Each call reads the
/// suffix from the owner store, wraps it in a `LogBatch`, and hands it to the
/// configured replication transport.
#[derive(Debug)]
pub struct OutboundLogPublisher<T> {
    transport: T,
    next_index: u64,
}

impl<T: ReplicationTransport> OutboundLogPublisher<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_index: 0,
        }
    }

    pub fn from_store_tail<S: EventStore>(transport: T, store: &S) -> Self {
        Self {
            transport,
            next_index: store.next_index(),
        }
    }

    pub fn with_next_index(transport: T, next_index: u64) -> Self {
        Self {
            transport,
            next_index,
        }
    }

    pub fn next_index(&self) -> u64 {
        self.next_index
    }

    /// Publish all events appended since the previous call.
    ///
    /// Returns the number of events handed to the transport.
    pub fn publish_new_events<S: EventStore>(&mut self, sector_id: SectorId, store: &S) -> usize {
        let events: Vec<_> = store
            .iter_from(self.next_index)
            .map(|record| record.event.clone())
            .collect();

        self.publish_events(sector_id, &events)
    }

    /// Publish an explicit transition output from the authoritative engine.
    ///
    /// The caller owns the event collection and has already established the
    /// durable ordering boundary. The publisher only assigns the contiguous
    /// replication range and advances its cursor after broadcasting it.
    pub fn publish_events(
        &mut self,
        sector_id: SectorId,
        events: &[dawn_core::DomainEvent],
    ) -> usize {
        if events.is_empty() {
            return 0;
        }

        let published = events.len();
        let batch = LogBatch::new(sector_id, self.next_index, events.to_vec());
        self.next_index = batch.next_index();
        self.transport.broadcast(batch);
        published
    }

    pub fn into_transport(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, DomainEvent, NodeId, ShipId, Tick, Velocity};
    use dawn_storage::{EventStore, InMemoryEventStore};

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(0), n),
            velocity: Velocity::new(n as f64, 0.0, 0.0),
            tick: Tick(n),
        })
    }

    #[tokio::test]
    async fn publish_new_events_sends_only_the_unpublished_suffix() {
        let bus = crate::InMemoryReplicationBus::spawn();
        let mut rx = bus.subscribe();
        let mut publisher = OutboundLogPublisher::new(bus.clone());
        let mut store = InMemoryEventStore::new();

        store.append(event(0));
        store.append(event(1));

        assert_eq!(publisher.publish_new_events(SectorId(7), &store), 2);
        assert_eq!(publisher.next_index(), 2);

        let first = rx.recv().await.unwrap();
        assert_eq!(first.sector_id, SectorId(7));
        assert_eq!(first.from_index, 0);
        assert_eq!(first.events.len(), 2);

        assert_eq!(publisher.publish_new_events(SectorId(7), &store), 0);

        store.append(event(2));
        assert_eq!(publisher.publish_new_events(SectorId(7), &store), 1);

        let second = rx.recv().await.unwrap();
        assert_eq!(second.from_index, 2);
        assert_eq!(second.events.len(), 1);

        let bus = publisher.into_transport();
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn from_store_tail_starts_after_existing_events() {
        let bus = crate::InMemoryReplicationBus::spawn();
        let mut rx = bus.subscribe();
        let mut store = InMemoryEventStore::new();

        store.append(event(0));
        store.append(event(1));

        let mut publisher = OutboundLogPublisher::from_store_tail(bus.clone(), &store);
        assert_eq!(publisher.next_index(), 2);
        assert_eq!(publisher.publish_new_events(SectorId(7), &store), 0);
        assert!(rx.try_recv().is_err());

        store.append(event(2));
        assert_eq!(publisher.publish_new_events(SectorId(7), &store), 1);

        let batch = rx.recv().await.unwrap();
        assert_eq!(batch.sector_id, SectorId(7));
        assert_eq!(batch.from_index, 2);
        assert_eq!(batch.events.len(), 1);

        let bus = publisher.into_transport();
        bus.shutdown().await;
    }
}
