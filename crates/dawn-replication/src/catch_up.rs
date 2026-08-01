//! Bounded replica catch-up from a detected log gap.
//!
//! A [`CatchUpManager`] owns the complete receiver/owner handshake: bounded
//! suffix requests, compacted-prefix detection, snapshot fallback, retry
//! limits, and resumption from the snapshot log index. Foreign Sector state is
//! retained only inside [`crate::ReplicaSet`]; this module never applies it to
//! a live local world.

use crate::{Ingest, LogBatch, ReplicaSet, ReplicaSnapshot, SnapshotInstall};
use dawn_core::SectorId;
use dawn_event_store::EventStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::broadcast;

/// Transport side-channel for catch-up control messages.
pub trait CatchUpTransport: Send + Sync {
    fn send_catch_up(&self, message: CatchUpMessage);
    fn subscribe_catch_up(&self) -> broadcast::Receiver<CatchUpMessage>;
}

/// Resource and retry bounds for one catch-up manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchUpConfig {
    /// Maximum events in one retained-suffix response.
    pub max_events: usize,
    /// Tick calls between retransmissions of the same request.
    pub retry_interval_ticks: u32,
    /// Retransmissions allowed after the initial send.
    pub max_retries: u32,
    /// Maximum distinct suffix/snapshot rounds in one catch-up session.
    pub max_requests_per_session: u32,
    /// Maximum opaque snapshot payload retained by a replica.
    pub max_snapshot_bytes: usize,
}

impl Default for CatchUpConfig {
    fn default() -> Self {
        Self {
            max_events: 4096,
            retry_interval_ticks: 10,
            max_retries: 5,
            max_requests_per_session: 1024,
            max_snapshot_bytes: 256 * 1024 * 1024,
        }
    }
}

/// A directed request for one owner's missing retained suffix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatchUpRequest {
    pub request_id: u64,
    pub requester_sector_id: SectorId,
    pub owner_sector_id: SectorId,
    pub from_index: u64,
    pub max_events: u32,
}

/// Why an owner could not satisfy a catch-up request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatchUpUnavailable {
    InvalidRequest,
    SnapshotUnavailable,
    SnapshotTooLarge,
    SnapshotSectorMismatch,
    SnapshotBehindRetainedPrefix,
    SnapshotAheadOfOwner,
    RetainedSuffixUnavailable,
}

/// One directed response to a [`CatchUpRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatchUpResponse {
    pub request_id: u64,
    pub requester_sector_id: SectorId,
    pub owner_sector_id: SectorId,
    pub payload: CatchUpPayload,
}

/// Catch-up response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CatchUpPayload {
    /// A bounded retained suffix and the owner's current tail index.
    Suffix {
        batch: LogBatch,
        owner_next_index: u64,
    },
    /// Opaque recovery snapshot; receiver resumes at `snapshot.log_index`.
    Snapshot {
        snapshot: ReplicaSnapshot,
        owner_next_index: u64,
    },
    /// Receiver is already at the owner's tail.
    UpToDate { owner_next_index: u64 },
    /// Owner cannot currently recover the receiver.
    Unavailable { reason: CatchUpUnavailable },
}

/// Control-plane message carried beside ordinary log gossip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CatchUpMessage {
    Request(CatchUpRequest),
    Response(CatchUpResponse),
}

/// Local terminal failure class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchUpFailureKind {
    Remote(CatchUpUnavailable),
    RetryExhausted,
    RequestBudgetExhausted,
    InvalidResponse,
    NoProgress,
}

/// Observable terminal catch-up failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchUpFailure {
    pub sector_id: SectorId,
    pub from_index: u64,
    pub kind: CatchUpFailureKind,
}

/// Observable progress from one manager step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUpEvent {
    Applied {
        sector_id: SectorId,
        applied: usize,
        next_index: u64,
    },
    RequestIssued {
        owner_sector_id: SectorId,
        from_index: u64,
        request_id: u64,
        attempt: u32,
    },
    SnapshotInstalled {
        sector_id: SectorId,
        log_index: u64,
        bytes: usize,
    },
    Completed {
        sector_id: SectorId,
        next_index: u64,
    },
    Failed(CatchUpFailure),
}

/// Outbound messages and observations produced by one manager call.
#[derive(Debug, Default)]
pub struct CatchUpStep {
    pub outbound: Vec<CatchUpMessage>,
    pub events: Vec<CatchUpEvent>,
}

#[derive(Debug, Clone)]
struct CatchUpSession {
    request: CatchUpRequest,
    retries: u32,
    ticks_until_retry: u32,
    requests_sent: u32,
}

/// Owns replica ingestion and all catch-up state for one local Sector node.
#[derive(Debug)]
pub struct CatchUpManager {
    local_sector_id: SectorId,
    config: CatchUpConfig,
    replicas: ReplicaSet,
    sessions: HashMap<SectorId, CatchUpSession>,
    /// A terminal failure blocks automatic restart at the same cursor. Any
    /// contiguous progress clears the block, and operators may call
    /// [`Self::reset_failure`] explicitly.
    failed_at: HashMap<SectorId, u64>,
    next_request_id: u64,
}

impl CatchUpManager {
    pub fn new(local_sector_id: SectorId, config: CatchUpConfig) -> Self {
        assert!(
            config.max_events > 0,
            "catch-up max_events must be non-zero"
        );
        assert!(
            config.max_events <= u32::MAX as usize,
            "catch-up max_events must fit on the wire"
        );
        assert!(
            config.retry_interval_ticks > 0,
            "catch-up retry interval must be non-zero"
        );
        assert!(
            config.max_requests_per_session > 0,
            "catch-up request budget must be non-zero"
        );
        Self {
            local_sector_id,
            replicas: ReplicaSet::new(config.max_events),
            config,
            sessions: HashMap::new(),
            failed_at: HashMap::new(),
            next_request_id: 1,
        }
    }

    pub fn replicas(&self) -> &ReplicaSet {
        &self.replicas
    }

    pub fn failure_cursor(&self, sector_id: SectorId) -> Option<u64> {
        self.failed_at.get(&sector_id).copied()
    }

    pub fn reset_failure(&mut self, sector_id: SectorId) {
        self.failed_at.remove(&sector_id);
    }

    /// Ingest ordinary gossip. A detected gap automatically emits one bounded
    /// request; repeated/reordered gap batches do not allocate more sessions.
    pub fn ingest_batch(&mut self, batch: LogBatch) -> CatchUpStep {
        let mut step = CatchUpStep::default();
        let sector_id = batch.sector_id;

        // Local authoritative state must never enter the foreign recovery set.
        if sector_id == self.local_sector_id {
            return step;
        }

        match self.replicas.ingest(&batch) {
            Ingest::Applied {
                applied,
                next_index,
                ..
            } => {
                if applied > 0 {
                    self.failed_at.remove(&sector_id);
                    step.events.push(CatchUpEvent::Applied {
                        sector_id,
                        applied,
                        next_index,
                    });

                    // Progress may have made an in-flight request stale. Replace
                    // it with one request at the new cursor; late responses to
                    // the old request are ignored by request_id.
                    if let Some(old) = self.sessions.remove(&sector_id) {
                        self.issue_request(sector_id, next_index, old.requests_sent, &mut step);
                    }
                }
            }
            Ingest::Duplicate => {}
            Ingest::Gap(request) => {
                self.ensure_request(request.sector_id, request.from_index, &mut step);
            }
        }

        step
    }

    /// Handle a directed catch-up control message. `snapshot_provider` is only
    /// evaluated when this node owns a compacted prefix requested by a peer.
    pub fn handle_message<S, F>(
        &mut self,
        message: CatchUpMessage,
        owner_store: &S,
        snapshot_provider: F,
    ) -> CatchUpStep
    where
        S: EventStore,
        F: FnOnce() -> Option<ReplicaSnapshot>,
    {
        match message {
            CatchUpMessage::Request(request) => {
                self.serve_request(request, owner_store, snapshot_provider)
            }
            CatchUpMessage::Response(response) => self.apply_response(response),
        }
    }

    /// Advance retry timers once. Retries resend the same request_id so
    /// duplicate owner responses remain harmless.
    pub fn tick(&mut self) -> CatchUpStep {
        let mut step = CatchUpStep::default();
        let sectors: Vec<_> = self.sessions.keys().copied().collect();

        for sector_id in sectors {
            let mut retry = None;
            let mut exhausted = None;
            if let Some(session) = self.sessions.get_mut(&sector_id) {
                if session.ticks_until_retry > 1 {
                    session.ticks_until_retry -= 1;
                    continue;
                }

                if session.retries < self.config.max_retries {
                    session.retries += 1;
                    session.ticks_until_retry = self.config.retry_interval_ticks;
                    retry = Some((session.request.clone(), session.retries + 1));
                } else {
                    exhausted = Some(session.request.from_index);
                }
            }

            if let Some((request, attempt)) = retry {
                step.outbound.push(CatchUpMessage::Request(request.clone()));
                step.events.push(CatchUpEvent::RequestIssued {
                    owner_sector_id: sector_id,
                    from_index: request.from_index,
                    request_id: request.request_id,
                    attempt,
                });
            }

            if let Some(from_index) = exhausted {
                self.fail(
                    sector_id,
                    from_index,
                    CatchUpFailureKind::RetryExhausted,
                    &mut step,
                );
            }
        }

        step
    }

    fn ensure_request(&mut self, sector_id: SectorId, from_index: u64, step: &mut CatchUpStep) {
        let cursor = self.replicas.next_index(sector_id);
        if self.failed_at.get(&sector_id) == Some(&cursor) {
            return;
        }
        if self.sessions.contains_key(&sector_id) {
            return;
        }
        self.issue_request(sector_id, from_index, 0, step);
    }

    fn issue_request(
        &mut self,
        sector_id: SectorId,
        from_index: u64,
        previous_requests: u32,
        step: &mut CatchUpStep,
    ) {
        if previous_requests >= self.config.max_requests_per_session {
            self.fail(
                sector_id,
                from_index,
                CatchUpFailureKind::RequestBudgetExhausted,
                step,
            );
            return;
        }

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let request = CatchUpRequest {
            request_id,
            requester_sector_id: self.local_sector_id,
            owner_sector_id: sector_id,
            from_index,
            max_events: self.config.max_events as u32,
        };
        let requests_sent = previous_requests + 1;
        self.sessions.insert(
            sector_id,
            CatchUpSession {
                request: request.clone(),
                retries: 0,
                ticks_until_retry: self.config.retry_interval_ticks,
                requests_sent,
            },
        );
        step.outbound.push(CatchUpMessage::Request(request.clone()));
        step.events.push(CatchUpEvent::RequestIssued {
            owner_sector_id: sector_id,
            from_index,
            request_id,
            attempt: 1,
        });
    }

    fn serve_request<S, F>(
        &self,
        request: CatchUpRequest,
        store: &S,
        snapshot_provider: F,
    ) -> CatchUpStep
    where
        S: EventStore,
        F: FnOnce() -> Option<ReplicaSnapshot>,
    {
        let mut step = CatchUpStep::default();
        if request.owner_sector_id != self.local_sector_id
            || request.requester_sector_id == self.local_sector_id
        {
            return step;
        }

        let owner_next_index = store.next_index();
        let payload = if request.max_events == 0 {
            CatchUpPayload::Unavailable {
                reason: CatchUpUnavailable::InvalidRequest,
            }
        } else if request.from_index >= owner_next_index {
            CatchUpPayload::UpToDate { owner_next_index }
        } else {
            let first_retained_index = store
                .iter_from(0)
                .next()
                .map_or(owner_next_index, |record| record.log_index);

            if request.from_index < first_retained_index {
                match snapshot_provider() {
                    None => CatchUpPayload::Unavailable {
                        reason: CatchUpUnavailable::SnapshotUnavailable,
                    },
                    Some(snapshot) if snapshot.bytes.len() > self.config.max_snapshot_bytes => {
                        CatchUpPayload::Unavailable {
                            reason: CatchUpUnavailable::SnapshotTooLarge,
                        }
                    }
                    Some(snapshot) if snapshot.sector_id != self.local_sector_id => {
                        CatchUpPayload::Unavailable {
                            reason: CatchUpUnavailable::SnapshotSectorMismatch,
                        }
                    }
                    Some(snapshot) if snapshot.log_index < first_retained_index => {
                        CatchUpPayload::Unavailable {
                            reason: CatchUpUnavailable::SnapshotBehindRetainedPrefix,
                        }
                    }
                    Some(snapshot) if snapshot.log_index > owner_next_index => {
                        CatchUpPayload::Unavailable {
                            reason: CatchUpUnavailable::SnapshotAheadOfOwner,
                        }
                    }
                    Some(snapshot) => CatchUpPayload::Snapshot {
                        snapshot,
                        owner_next_index,
                    },
                }
            } else {
                let max_events = (request.max_events as usize).min(self.config.max_events);
                let records: Vec<_> = store
                    .iter_from(request.from_index)
                    .take(max_events)
                    .collect();
                match records.first() {
                    Some(first) if first.log_index == request.from_index => {
                        CatchUpPayload::Suffix {
                            batch: LogBatch::new(
                                self.local_sector_id,
                                first.log_index,
                                records.iter().map(|record| record.event.clone()).collect(),
                            ),
                            owner_next_index,
                        }
                    }
                    _ => CatchUpPayload::Unavailable {
                        reason: CatchUpUnavailable::RetainedSuffixUnavailable,
                    },
                }
            }
        };

        step.outbound
            .push(CatchUpMessage::Response(CatchUpResponse {
                request_id: request.request_id,
                requester_sector_id: request.requester_sector_id,
                owner_sector_id: self.local_sector_id,
                payload,
            }));
        step
    }

    fn apply_response(&mut self, response: CatchUpResponse) -> CatchUpStep {
        let mut step = CatchUpStep::default();
        if response.requester_sector_id != self.local_sector_id
            || response.owner_sector_id == self.local_sector_id
        {
            return step;
        }

        let sector_id = response.owner_sector_id;
        let Some(session) = self.sessions.get(&sector_id) else {
            return step;
        };
        if session.request.request_id != response.request_id
            || session.request.requester_sector_id != response.requester_sector_id
            || session.request.owner_sector_id != response.owner_sector_id
        {
            return step;
        }
        let session = self
            .sessions
            .remove(&sector_id)
            .expect("session checked above");
        let from_index = session.request.from_index;

        match response.payload {
            CatchUpPayload::Suffix {
                batch,
                owner_next_index,
            } => {
                if batch.sector_id != sector_id
                    || batch.events.len() > self.config.max_events
                    || batch.next_index() > owner_next_index
                {
                    self.fail(
                        sector_id,
                        from_index,
                        CatchUpFailureKind::InvalidResponse,
                        &mut step,
                    );
                    return step;
                }

                let before = self.replicas.next_index(sector_id);
                match self.replicas.ingest(&batch) {
                    Ingest::Applied {
                        applied,
                        next_index,
                        ..
                    } => {
                        if applied > 0 {
                            self.failed_at.remove(&sector_id);
                            step.events.push(CatchUpEvent::Applied {
                                sector_id,
                                applied,
                                next_index,
                            });
                        }
                    }
                    Ingest::Duplicate => {}
                    Ingest::Gap(_) => {
                        self.fail(
                            sector_id,
                            before,
                            CatchUpFailureKind::InvalidResponse,
                            &mut step,
                        );
                        return step;
                    }
                }
                self.resume_or_complete(
                    sector_id,
                    before,
                    owner_next_index,
                    session.requests_sent,
                    &mut step,
                );
            }
            CatchUpPayload::Snapshot {
                snapshot,
                owner_next_index,
            } => {
                if snapshot.sector_id != sector_id
                    || snapshot.bytes.len() > self.config.max_snapshot_bytes
                    || snapshot.log_index > owner_next_index
                {
                    self.fail(
                        sector_id,
                        from_index,
                        CatchUpFailureKind::InvalidResponse,
                        &mut step,
                    );
                    return step;
                }

                let before = self.replicas.next_index(sector_id);
                let bytes = snapshot.bytes.len();
                let log_index = snapshot.log_index;
                if let SnapshotInstall::Installed { .. } = self.replicas.install_snapshot(snapshot)
                {
                    self.failed_at.remove(&sector_id);
                    step.events.push(CatchUpEvent::SnapshotInstalled {
                        sector_id,
                        log_index,
                        bytes,
                    });
                }
                self.resume_or_complete(
                    sector_id,
                    before,
                    owner_next_index,
                    session.requests_sent,
                    &mut step,
                );
            }
            CatchUpPayload::UpToDate { owner_next_index } => {
                let next_index = self.replicas.next_index(sector_id);
                if next_index == owner_next_index {
                    self.failed_at.remove(&sector_id);
                    step.events.push(CatchUpEvent::Completed {
                        sector_id,
                        next_index,
                    });
                } else {
                    self.fail(
                        sector_id,
                        next_index,
                        CatchUpFailureKind::InvalidResponse,
                        &mut step,
                    );
                }
            }
            CatchUpPayload::Unavailable { reason } => {
                self.fail(
                    sector_id,
                    from_index,
                    CatchUpFailureKind::Remote(reason),
                    &mut step,
                );
            }
        }

        step
    }

    fn resume_or_complete(
        &mut self,
        sector_id: SectorId,
        before: u64,
        owner_next_index: u64,
        requests_sent: u32,
        step: &mut CatchUpStep,
    ) {
        let next_index = self.replicas.next_index(sector_id);
        if next_index > owner_next_index {
            self.fail(
                sector_id,
                next_index,
                CatchUpFailureKind::InvalidResponse,
                step,
            );
        } else if next_index == owner_next_index {
            self.failed_at.remove(&sector_id);
            step.events.push(CatchUpEvent::Completed {
                sector_id,
                next_index,
            });
        } else if next_index <= before {
            self.fail(sector_id, next_index, CatchUpFailureKind::NoProgress, step);
        } else {
            self.issue_request(sector_id, next_index, requests_sent, step);
        }
    }

    fn fail(
        &mut self,
        sector_id: SectorId,
        from_index: u64,
        kind: CatchUpFailureKind,
        step: &mut CatchUpStep,
    ) {
        self.sessions.remove(&sector_id);
        self.failed_at.insert(sector_id, from_index);
        step.events.push(CatchUpEvent::Failed(CatchUpFailure {
            sector_id,
            from_index,
            kind,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{events::VelocityChanged, DomainEvent, NodeId, ShipId, Tick, Velocity};
    use dawn_event_store::{EventStore, FileEventStore, InMemoryEventStore};

    fn event(n: u64) -> DomainEvent {
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ShipId::new(NodeId(1), n),
            velocity: Velocity::new(n as f64, 0.0, 0.0),
            tick: Tick(n),
        })
    }

    fn fill<S: EventStore>(store: &mut S, count: u64) {
        for n in 0..count {
            store.append(event(n));
        }
    }

    fn batch<S: EventStore>(store: &S, sector_id: SectorId, from: u64, count: usize) -> LogBatch {
        LogBatch::new(
            sector_id,
            from,
            store
                .iter_from(from)
                .take(count)
                .map(|record| record.event.clone())
                .collect(),
        )
    }

    fn config(max_events: usize) -> CatchUpConfig {
        CatchUpConfig {
            max_events,
            retry_interval_ticks: 1,
            max_retries: 1,
            max_requests_per_session: 32,
            max_snapshot_bytes: 1024,
        }
    }

    fn drive<S: EventStore>(
        owner: &mut CatchUpManager,
        receiver: &mut CatchUpManager,
        owner_store: &S,
        receiver_store: &InMemoryEventStore,
        mut outbound: Vec<CatchUpMessage>,
        snapshot: Option<ReplicaSnapshot>,
    ) {
        for _ in 0..64 {
            if outbound.is_empty() {
                return;
            }
            assert_eq!(outbound.len(), 1, "catch-up keeps one request in flight");
            let owner_step =
                owner.handle_message(outbound.pop().unwrap(), owner_store, || snapshot.clone());
            assert_eq!(owner_step.outbound.len(), 1);
            let receiver_step = receiver.handle_message(
                owner_step.outbound.into_iter().next().unwrap(),
                receiver_store,
                || None,
            );
            outbound = receiver_step.outbound;
        }
        panic!("catch-up did not converge within the request budget");
    }

    #[test]
    fn gap_automatically_catches_up_through_bounded_suffixes() {
        let owner_sector = SectorId(1);
        let receiver_sector = SectorId(2);
        let mut store = InMemoryEventStore::new();
        fill(&mut store, 8);
        let receiver_store = InMemoryEventStore::new();
        let mut owner = CatchUpManager::new(owner_sector, config(2));
        let mut receiver = CatchUpManager::new(receiver_sector, config(2));

        receiver.ingest_batch(batch(&store, owner_sector, 0, 2));
        let gap = receiver.ingest_batch(batch(&store, owner_sector, 6, 2));
        assert_eq!(gap.outbound.len(), 1);

        drive(
            &mut owner,
            &mut receiver,
            &store,
            &receiver_store,
            gap.outbound,
            None,
        );

        assert_eq!(receiver.replicas().next_index(owner_sector), 8);
        assert_eq!(receiver.replicas().replicated_len(owner_sector), 8);
        assert!(receiver.replicas().snapshot(owner_sector).is_none());
    }

    #[test]
    fn compacted_gap_falls_back_to_snapshot_then_resumes_at_snapshot_index() {
        let dir = tempfile::tempdir().unwrap();
        let hot = dir.path().join("events.log");
        let cold = dir.path().join("cold.log");
        let owner_sector = SectorId(1);
        let receiver_sector = SectorId(2);
        let mut store = FileEventStore::open(&hot).unwrap();
        fill(&mut store, 10);
        store.compact(6, &cold).unwrap();

        let snapshot = ReplicaSnapshot::new(owner_sector, 6, vec![1, 2, 3, 4]);
        let receiver_store = InMemoryEventStore::new();
        let mut owner = CatchUpManager::new(owner_sector, config(2));
        let mut receiver = CatchUpManager::new(receiver_sector, config(2));

        let gap = receiver.ingest_batch(batch(&store, owner_sector, 8, 2));
        drive(
            &mut owner,
            &mut receiver,
            &store,
            &receiver_store,
            gap.outbound,
            Some(snapshot.clone()),
        );

        assert_eq!(receiver.replicas().next_index(owner_sector), 10);
        assert_eq!(receiver.replicas().base_index(owner_sector), 6);
        assert_eq!(receiver.replicas().replicated_len(owner_sector), 4);
        assert_eq!(receiver.replicas().snapshot(owner_sector), Some(&snapshot));
    }

    #[test]
    fn duplicates_overlaps_reordering_and_repeated_gaps_stay_bounded() {
        let sector = SectorId(1);
        let mut manager = CatchUpManager::new(SectorId(2), config(2));
        let mut store = InMemoryEventStore::new();
        fill(&mut store, 10);

        manager.ingest_batch(batch(&store, sector, 0, 3));
        manager.ingest_batch(batch(&store, sector, 0, 3));
        manager.ingest_batch(batch(&store, sector, 2, 3));
        assert_eq!(manager.replicas().next_index(sector), 5);
        assert_eq!(manager.replicas().replicated_len(sector), 5);

        let first_gap = manager.ingest_batch(batch(&store, sector, 8, 2));
        assert_eq!(first_gap.outbound.len(), 1);
        let repeated_gap = manager.ingest_batch(batch(&store, sector, 8, 2));
        assert!(repeated_gap.outbound.is_empty());
        assert_eq!(manager.replicas().replicated_len(sector), 5);

        assert_eq!(manager.tick().outbound.len(), 1, "one bounded retry");
        let exhausted = manager.tick();
        assert!(matches!(
            exhausted.events.as_slice(),
            [CatchUpEvent::Failed(CatchUpFailure {
                kind: CatchUpFailureKind::RetryExhausted,
                ..
            })]
        ));
        assert!(manager.tick().outbound.is_empty());
        assert!(manager
            .ingest_batch(batch(&store, sector, 8, 2))
            .outbound
            .is_empty());
    }

    #[test]
    fn local_sector_batches_are_never_added_to_the_foreign_recovery_set() {
        let local = SectorId(4);
        let mut manager = CatchUpManager::new(local, config(2));
        let mut store = InMemoryEventStore::new();
        fill(&mut store, 2);

        manager.ingest_batch(batch(&store, local, 0, 2));

        assert_eq!(manager.replicas().next_index(local), 0);
        assert_eq!(manager.replicas().replicated_len(local), 0);
    }
}
