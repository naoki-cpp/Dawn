//! Outbound append-log publishing for Sector owners (ADR-0027).
//!
//! This module owns the sender-side cursor and `LogBatch` construction so
//! callers do not need to know log indices or transport payload shape.

use crate::{LogBatch, ReplicationTransport};
use dawn_core::SectorId;
use dawn_storage::PublicEventIndex;

/// Publishes the newly appended suffix of one owner's event log.
///
/// The publisher keeps the next unpublished public-event index. The runtime
/// passes committed frame output directly; this adapter wraps it in a
/// `LogBatch` and hands it to the configured replication transport.
#[derive(Debug)]
pub struct OutboundLogPublisher<T> {
    transport: T,
    next_public_event_index: PublicEventIndex,
}

impl<T: ReplicationTransport> OutboundLogPublisher<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_public_event_index: PublicEventIndex(0),
        }
    }

    pub fn with_next_public_event_index(
        transport: T,
        next_public_event_index: impl Into<PublicEventIndex>,
    ) -> Self {
        Self {
            transport,
            next_public_event_index: next_public_event_index.into(),
        }
    }

    pub fn next_public_event_index(&self) -> u64 {
        self.next_public_event_index.0
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
        let batch = LogBatch::new(sector_id, self.next_public_event_index, events.to_vec());
        self.next_public_event_index = batch.next_public_event_index();
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

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(0), n),
            velocity: Velocity::new(n as f64, 0.0, 0.0),
            tick: Tick(n),
        })
    }

    #[tokio::test]
    async fn publish_events_assigns_contiguous_public_event_ranges() {
        let bus = crate::InMemoryReplicationBus::spawn();
        let mut rx = bus.subscribe();
        let mut publisher = OutboundLogPublisher::new(bus.clone());

        assert_eq!(
            publisher.publish_events(SectorId(7), &[event(0), event(1)]),
            2
        );
        assert_eq!(publisher.next_public_event_index(), 2);

        let first = rx.recv().await.unwrap();
        assert_eq!(first.sector_id, SectorId(7));
        assert_eq!(first.from_public_event_index, 0);
        assert_eq!(first.events.len(), 2);

        assert_eq!(publisher.publish_events(SectorId(7), &[]), 0);

        assert_eq!(publisher.publish_events(SectorId(7), &[event(2)]), 1);

        let second = rx.recv().await.unwrap();
        assert_eq!(second.from_public_event_index, 2);
        assert_eq!(second.events.len(), 1);

        let bus = publisher.into_transport();
        bus.shutdown().await;
    }

    #[tokio::test]
    async fn configured_cursor_starts_after_rebuilt_events() {
        let bus = crate::InMemoryReplicationBus::spawn();
        let mut rx = bus.subscribe();
        let mut publisher = OutboundLogPublisher::with_next_public_event_index(bus.clone(), 2_u64);
        assert_eq!(publisher.next_public_event_index(), 2);
        assert_eq!(publisher.publish_events(SectorId(7), &[event(2)]), 1);

        let batch = rx.recv().await.unwrap();
        assert_eq!(batch.sector_id, SectorId(7));
        assert_eq!(batch.from_public_event_index, 2);
        assert_eq!(batch.events.len(), 1);

        let bus = publisher.into_transport();
        bus.shutdown().await;
    }
}
