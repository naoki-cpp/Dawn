//! Runtime-facing resolution for Sector-owned client admission attempts.
//!
//! Socket adapters report only whether transport completed. This module turns
//! that result into exactly one authoritative admission transition: commit on
//! success, abort on transport failure, or a commit-rejected result when the
//! reserved attempt became stale before completion. Adapters never select
//! cleanup behavior or advance `ResumeTicket` state themselves.

use dawn_event_store::store::EventStore;

use crate::{
    client_admission::{
        ClientAdmissionAttempt, ClientAdmissionCommitError, CommittedClientAdmission,
    },
    node::SimulationNode,
};

/// The authoritative result of resolving one asynchronous admission attempt.
///
/// `ClientAdmissionAttempt` is consumed by [`resolve_client_admission`], so an
/// attempt cannot be committed, aborted, or otherwise resolved more than once.
#[must_use = "the runtime must handle the authoritative admission resolution"]
#[derive(Debug, PartialEq, Eq)]
pub enum ClientAdmissionResolution<T, E> {
    /// Transport completed and the Sector committed the reserved identity.
    Committed {
        value: T,
        admission: CommittedClientAdmission,
    },
    /// Transport failed and the Sector released the attempt's live claims.
    Aborted { error: E },
    /// Transport completed, but the reservation was stale or otherwise no
    /// longer commit-capable. The Sector has already performed cleanup.
    CommitRejected { error: ClientAdmissionCommitError },
}

/// Resolve one transport outcome through the authoritative Sector boundary.
///
/// Runtime adapters should call this exactly once after sending the handoff.
/// They may publish the returned transport value only for
/// [`ClientAdmissionResolution::Committed`]; all commit/abort and cleanup
/// policy remains inside `dawn-sector`.
pub fn resolve_client_admission<S: EventStore, T, E>(
    node: &mut SimulationNode<S>,
    attempt: ClientAdmissionAttempt,
    transport_result: Result<T, E>,
) -> ClientAdmissionResolution<T, E> {
    match transport_result {
        Ok(value) => match attempt.commit(node) {
            Ok(admission) => ClientAdmissionResolution::Committed { value, admission },
            Err(error) => ClientAdmissionResolution::CommitRejected { error },
        },
        Err(error) => {
            attempt.abort(node);
            ClientAdmissionResolution::Aborted { error }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, Position, SectorBounds, SectorId};

    use crate::client_admission::{ClientAdmissionIntent, ClientAdmissionRefusal};

    const AOI_CELL_SIZE: f64 = 1_000.0;

    fn node() -> SimulationNode {
        SimulationNode::new(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn fresh_attempt(node: &mut SimulationNode) -> ClientAdmissionAttempt {
        node.begin_client_admission(
            ClientAdmissionIntent::Fresh {
                spawn_position: Position::ORIGIN,
            },
            AOI_CELL_SIZE,
        )
        .expect("fresh admission should begin")
    }

    #[test]
    fn successful_transport_commits_and_returns_adapter_value() {
        let mut node = node();
        let attempt = fresh_attempt(&mut node);
        let ship_id = attempt.ship_id();

        let resolution = resolve_client_admission(&mut node, attempt, Ok::<_, &'static str>(42));

        assert!(matches!(
            resolution,
            ClientAdmissionResolution::Committed {
                value: 42,
                admission
            } if admission.ship_id == ship_id && !admission.resumed
        ));
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn transport_failure_aborts_and_preserves_adapter_error() {
        let mut node = node();
        let attempt = fresh_attempt(&mut node);

        let resolution =
            resolve_client_admission::<_, (), _>(&mut node, attempt, Err("client disconnected"));

        assert_eq!(
            resolution,
            ClientAdmissionResolution::Aborted {
                error: "client disconnected"
            }
        );
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn stale_success_is_rejected_and_cleanup_is_already_complete() {
        let mut node = node();
        let attempt = fresh_attempt(&mut node);
        let ship_id = attempt.ship_id();
        node.abort_reserved_fresh_admission(ship_id);

        let resolution = resolve_client_admission(&mut node, attempt, Ok::<_, ()>(()));

        assert!(matches!(
            resolution,
            ClientAdmissionResolution::CommitRejected { error }
                if error.ship_id == ship_id
        ));
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn aborted_resume_keeps_the_last_client_visible_ticket_retryable() {
        let mut node = node();
        let fresh = fresh_attempt(&mut node);
        let current_ticket = fresh.resume_ticket();
        let committed = fresh.commit(&mut node).expect("fresh commit");

        let resume = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: current_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("resume admission should begin");
        let staged_ticket = resume.resume_ticket();
        let resolution =
            resolve_client_admission::<_, (), _>(&mut node, resume, Err("handoff failed"));
        assert!(matches!(
            resolution,
            ClientAdmissionResolution::Aborted {
                error: "handoff failed"
            }
        ));
        assert_eq!(node.ship_count(), 1);

        let staged_retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: staged_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("ticket exposed before abort remains retryable");
        assert_eq!(staged_retry.player_id(), committed.player_id);
        assert_eq!(staged_retry.ship_id(), committed.ship_id);
        let successor_ticket = staged_retry.resume_ticket();
        assert!(matches!(
            resolve_client_admission::<_, (), _>(
                &mut node,
                staged_retry,
                Err("second handoff failed"),
            ),
            ClientAdmissionResolution::Aborted { .. }
        ));

        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: current_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("presenting the staged ticket advances past the older current ticket"),
            ClientAdmissionRefusal::ResumeTicketInvalid
        );
        let staged_again = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: staged_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("the last client-visible ticket survives a repeated abort");
        assert_eq!(staged_again.resume_ticket(), successor_ticket);
        assert!(matches!(
            resolve_client_admission::<_, (), _>(&mut node, staged_again, Err("cleanup")),
            ClientAdmissionResolution::Aborted { .. }
        ));
    }

    #[test]
    fn consecutive_aborted_resumes_keep_the_last_client_visible_ticket_retryable() {
        let mut node = node();
        let fresh = fresh_attempt(&mut node);
        let committed_ticket = fresh.resume_ticket();
        assert!(matches!(
            resolve_client_admission(&mut node, fresh, Ok::<_, ()>(())),
            ClientAdmissionResolution::Committed { .. }
        ));

        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: committed_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("committed ticket begins the first resume");
        let first_visible_ticket = first.resume_ticket();
        assert_ne!(first_visible_ticket, committed_ticket);
        assert!(matches!(
            resolve_client_admission::<_, (), _>(
                &mut node,
                first,
                Err("first handoff failed"),
            ),
            ClientAdmissionResolution::Aborted { .. }
        ));

        let second = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: first_visible_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("the first advertised ticket begins the second resume");
        let second_visible_ticket = second.resume_ticket();
        assert_ne!(second_visible_ticket, first_visible_ticket);
        assert!(matches!(
            resolve_client_admission::<_, (), _>(
                &mut node,
                second,
                Err("second handoff failed"),
            ),
            ClientAdmissionResolution::Aborted { .. }
        ));

        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: first_visible_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("the client-visible ticket must survive the second abort");
        assert_eq!(retry.resume_ticket(), second_visible_ticket);
        assert!(matches!(
            resolve_client_admission(&mut node, retry, Ok::<_, ()>(())),
            ClientAdmissionResolution::Committed { .. }
        ));

        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: first_visible_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("a committed rotation consumes the presented ticket"),
            ClientAdmissionRefusal::ResumeTicketInvalid
        );
        let next = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: second_visible_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("the committed successor remains usable");
        assert!(matches!(
            resolve_client_admission::<_, (), _>(&mut node, next, Err("cleanup")),
            ClientAdmissionResolution::Aborted { .. }
        ));
    }
}
