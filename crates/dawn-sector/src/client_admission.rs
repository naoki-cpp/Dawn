//! Transactional client admission owned by `dawn-sector`.
//!
//! Runtime adapters wait on sockets and promote committed sessions. This module
//! owns every authoritative mutation between those two points: population-cap
//! checks, fresh identity allocation and spawn, resume validation, observer-
//! scoped handoff construction, commit, and abort cleanup.

use dawn_core::{PlayerId, Position, ResumeTicket, ShipId};

use crate::node::{HandoffPayload, MissingObserverShip, SimulationNode};

/// Runtime-provided identity intent for one connection attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientAdmissionIntent {
    /// Allocate a new player identity and spawn a new Ship at this position.
    Fresh { spawn_position: Position },
    /// Resume the identity bound to a server-issued, one-time capability.
    Resume { resume_ticket: ResumeTicket },
}

/// Why a client admission attempt could not begin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientAdmissionRefusal {
    /// A fresh spawn would exceed the Sector population backstop.
    FreshAtPopulationCap,
    /// The opaque resume capability is unknown, already consumed, or no longer
    /// points at a live, non-Transit Ship on this Sector.
    ResumeTicketInvalid,
    /// Another handshake already holds the Ship/Player or prepared-fresh claim.
    ResumeAlreadyPending {
        player_id: PlayerId,
        ship_id: ShipId,
    },
    /// The requested pair would overwrite a different established identity.
    ResumeIdentityConflict {
        player_id: PlayerId,
        ship_id: ShipId,
    },
    /// A freshly-created observer could not be used to construct its scoped
    /// handoff. The fresh Ship has already been removed before this is returned.
    MissingObserver(MissingObserverShip),
}

impl std::fmt::Display for ClientAdmissionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FreshAtPopulationCap => write!(f, "Sector is at its population cap"),
            Self::ResumeTicketInvalid => write!(f, "resume ticket is invalid"),
            Self::ResumeAlreadyPending { player_id, ship_id } => write!(
                f,
                "resume refused for {player_id}: ship #{} or player already has an in-flight resume",
                ship_id.raw()
            ),
            Self::ResumeIdentityConflict { player_id, ship_id } => write!(
                f,
                "resume refused for {player_id}: ship #{} conflicts with established ownership",
                ship_id.raw()
            ),
            Self::MissingObserver(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ClientAdmissionRefusal {}

/// Identity made authoritative by a successful admission commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommittedClientAdmission {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub resumed: bool,
}

/// A successful WebSocket handshake could not be committed because the
/// reserved Ship or admission token disappeared while the handshake was in
/// flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAdmissionCommitError {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
}

impl std::fmt::Display for ClientAdmissionCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "admission commit refused for {}: ship #{} or reservation unavailable",
            self.player_id,
            self.ship_id.raw()
        )
    }
}

impl std::error::Error for ClientAdmissionCommitError {}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AdmissionOrigin {
    Fresh { spawn_position: Position },
    Resume { presented_ticket: ResumeTicket },
}

/// One begun but not yet committed client admission.
///
/// Runtime code must resolve the attempt exactly once through
/// [`crate::client_admission_resolution::resolve_client_admission`]. Direct
/// commit and abort transitions are crate-private so adapters cannot
/// reconstruct lifecycle or cleanup policy.
///
/// Fresh attempts hold a durable allocation watermark and SQLite prepared row
/// plus a live capacity claim. Abort releases only the live claim because a
/// partial handshake may already have exposed the pair. Resume attempts hold a
/// non-authoritative Ship/Player lock until commit; abort releases that lock and
/// never removes the pre-existing Ship.
#[derive(Debug)]
pub struct ClientAdmissionAttempt {
    player_id: PlayerId,
    ship_id: ShipId,
    resume_ticket: ResumeTicket,
    origin: AdmissionOrigin,
    handoff: Option<HandoffPayload>,
}

impl ClientAdmissionAttempt {
    pub fn player_id(&self) -> PlayerId {
        self.player_id
    }

    pub fn ship_id(&self) -> ShipId {
        self.ship_id
    }

    /// The Ticket that must be presented for the next connection attempt.
    pub fn resume_ticket(&self) -> ResumeTicket {
        self.resume_ticket
    }

    pub fn is_resumed(&self) -> bool {
        matches!(self.origin, AdmissionOrigin::Resume { .. })
    }

    /// Move the observer-scoped wire payload into the asynchronous socket task
    /// while retaining this attempt token for the later commit or abort.
    pub fn take_handoff_payload(&mut self) -> HandoffPayload {
        self.handoff
            .take()
            .expect("client admission handoff payload may only be taken once")
    }

    /// Commit after successful socket handshake completion.
    ///
    /// Resume ownership is established here, not at begin, so a failed resume
    /// handshake cannot leave ownership or docked-player state behind. Fresh
    /// Ship and gameplay state are materialized here from the reserved identity.
    pub(crate) fn commit(
        self,
        node: &mut SimulationNode,
    ) -> Result<CommittedClientAdmission, ClientAdmissionCommitError> {
        let present = match self.origin {
            AdmissionOrigin::Fresh { spawn_position } => node.commit_reserved_fresh_admission(
                self.player_id,
                self.ship_id,
                spawn_position,
                self.resume_ticket,
            ),
            AdmissionOrigin::Resume { presented_ticket } => node.commit_reserved_resume_admission(
                self.player_id,
                self.ship_id,
                presented_ticket,
                self.resume_ticket,
            ),
        };
        if !present {
            match self.origin {
                AdmissionOrigin::Fresh { .. } => {
                    node.abort_reserved_fresh_admission(self.ship_id);
                }
                AdmissionOrigin::Resume { .. } => {
                    node.release_resume_admission(self.player_id, self.ship_id);
                }
            }
            return Err(ClientAdmissionCommitError {
                player_id: self.player_id,
                ship_id: self.ship_id,
            });
        }

        Ok(CommittedClientAdmission {
            player_id: self.player_id,
            ship_id: self.ship_id,
            resumed: matches!(self.origin, AdmissionOrigin::Resume { .. }),
        })
    }

    /// Abort after any handshake error or disconnect.
    ///
    /// A fresh attempt releases its live capacity claim while retaining the
    /// consumed identity and prepared row for an exact retry. A resume attempt
    /// releases only its non-authoritative Ship/Player lock. Neither path
    /// removes a committed or pre-existing Ship.
    pub(crate) fn abort(self, node: &mut SimulationNode) {
        match self.origin {
            AdmissionOrigin::Fresh { .. } => {
                node.abort_reserved_fresh_admission(self.ship_id);
            }
            AdmissionOrigin::Resume { .. } => {
                node.release_resume_admission(self.player_id, self.ship_id);
            }
        }
    }
}

impl SimulationNode {
    /// Begin a transactional client admission and build its observer-scoped
    /// handoff payload.
    ///
    /// No runtime should separately allocate identity, spawn/adopt a Ship, or
    /// decide rollback. It should only pass socket-derived intent here, send the
    /// returned payload, then pass the transport result and attempt to
    /// [`crate::client_admission_resolution::resolve_client_admission`].
    pub fn begin_client_admission(
        &mut self,
        intent: ClientAdmissionIntent,
        aoi_cell_size: f64,
    ) -> Result<ClientAdmissionAttempt, ClientAdmissionRefusal> {
        match intent {
            ClientAdmissionIntent::Fresh { spawn_position } => {
                if self.at_population_cap() {
                    return Err(ClientAdmissionRefusal::FreshAtPopulationCap);
                }

                let (player_id, ship_id, resume_ticket) =
                    self.reserve_fresh_admission_identity(spawn_position);
                let handoff = match self.build_fresh_admission_handoff(
                    player_id,
                    ship_id,
                    spawn_position,
                    aoi_cell_size,
                ) {
                    Ok(handoff) => handoff,
                    Err(error) => {
                        self.abort_reserved_fresh_admission(ship_id);
                        return Err(ClientAdmissionRefusal::MissingObserver(error));
                    }
                };

                Ok(ClientAdmissionAttempt {
                    player_id,
                    ship_id,
                    resume_ticket,
                    origin: AdmissionOrigin::Fresh { spawn_position },
                    handoff: Some(handoff),
                })
            }
            ClientAdmissionIntent::Resume { resume_ticket } => {
                // A successful Welcome may have outlived the process before the
                // fresh Ship committed. Reclaim only that exact SQLite-prepared
                // pair; this is not ADR-0007 fresh fallback and allocates no ID.
                if let Some(prepared) = self.prepared_fresh_admission(resume_ticket) {
                    let (player_id, ship_id, spawn_position) = prepared;
                    // A prepared identity is still a fresh population claim:
                    // abort releases its live claim, so a later retry must
                    // compete with ships admitted while it was disconnected.
                    if self.at_population_cap() {
                        return Err(ClientAdmissionRefusal::FreshAtPopulationCap);
                    }
                    if !self.claim_prepared_fresh_admission(player_id, ship_id, resume_ticket) {
                        return Err(ClientAdmissionRefusal::ResumeAlreadyPending {
                            player_id,
                            ship_id,
                        });
                    }
                    let handoff = match self.build_fresh_admission_handoff(
                        player_id,
                        ship_id,
                        spawn_position,
                        aoi_cell_size,
                    ) {
                        Ok(handoff) => handoff,
                        Err(error) => {
                            self.abort_reserved_fresh_admission(ship_id);
                            return Err(ClientAdmissionRefusal::MissingObserver(error));
                        }
                    };
                    return Ok(ClientAdmissionAttempt {
                        player_id,
                        ship_id,
                        resume_ticket,
                        origin: AdmissionOrigin::Fresh { spawn_position },
                        handoff: Some(handoff),
                    });
                }

                let Some((player_id, ship_id)) = self.resolve_client_resume_ticket(resume_ticket)
                else {
                    return Err(ClientAdmissionRefusal::ResumeTicketInvalid);
                };
                // A Ship/Player reservation serializes concurrent resume
                // handshakes until resolution.
                if self.ship_absolute_pos(ship_id).is_none() || self.is_ship_in_transit(ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeTicketInvalid);
                }
                if self.resume_admission_identity_conflicts(player_id, ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeIdentityConflict {
                        player_id,
                        ship_id,
                    });
                }
                if !self.reserve_resume_admission(player_id, ship_id) {
                    return Err(ClientAdmissionRefusal::ResumeAlreadyPending {
                        player_id,
                        ship_id,
                    });
                }

                let result = (|| {
                    let mut handoff = self
                        .build_handoff_payload(ship_id, aoi_cell_size)
                        .map_err(|_| ClientAdmissionRefusal::ResumeTicketInvalid)?;

                    if let Some(observer) = handoff
                        .initial_state
                        .ships
                        .iter_mut()
                        .find(|ship| ship.ship_id == ship_id.raw())
                    {
                        observer.is_player = true;
                    }

                    let loadout = self
                        .build_player_loadout_json_for_admission(player_id, ship_id)
                        .ok_or(ClientAdmissionRefusal::ResumeTicketInvalid)?;
                    handoff.player_loadout = Some(loadout);
                    let proposed_next_ticket = self.issue_resume_ticket();
                    let next_ticket = self
                        .stage_client_resume_ticket(
                            ship_id,
                            player_id,
                            resume_ticket,
                            proposed_next_ticket,
                        )
                        .ok_or(ClientAdmissionRefusal::ResumeTicketInvalid)?;
                    Ok((handoff, next_ticket))
                })();

                match result {
                    Ok((handoff, next_ticket)) => Ok(ClientAdmissionAttempt {
                        player_id,
                        ship_id,
                        resume_ticket: next_ticket,
                        origin: AdmissionOrigin::Resume {
                            presented_ticket: resume_ticket,
                        },
                        handoff: Some(handoff),
                    }),
                    Err(refusal) => {
                        self.release_resume_admission(player_id, ship_id);
                        Err(refusal)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{DomainEvent, NodeId, SectorBounds, SectorId, ShipTypeId, Velocity};

    const AOI_CELL_SIZE: f64 = 1_000.0;

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    fn seeded_resume_ticket(
        node: &mut SimulationNode,
        player_id: PlayerId,
        ship_id: ShipId,
    ) -> ResumeTicket {
        let ticket = ResumeTicket::from_bytes([ship_id.raw() as u8; ResumeTicket::BYTE_LEN]);
        node.record_client_resume_ownership(ship_id, player_id, ticket);
        ticket
    }

    fn commit_runtime_frame(node: &mut SimulationNode) {
        let mut journal = dawn_storage::InMemoryJournal::new();
        let mut consensus = crate::transit::LocalRuntimeConsensus;
        let mut health = crate::transit::RuntimeHealth::default();
        let transition_id = crate::transit::runtime_transition_id(node);
        crate::transit::run_durable_runtime_frame(
            node,
            &mut journal,
            &mut consensus,
            &crate::transit::LocalRuntimeDurabilityPolicy,
            &mut health,
            crate::transition::FrameInput::lock_only(&[]),
            crate::transit::DurableRuntimeTickContext {
                transition_id,
                owner_epoch: 0,
                durability: dawn_storage::DurabilityMode::Synced,
                profile: crate::transit::RuntimeDurabilityProfile::LocalDurable,
            },
            crate::transit::reconcile_runtime_repositories,
            |_, _, _| {},
        )
        .expect("admission frame should commit");
    }

    #[test]
    fn fresh_commit_keeps_the_spawned_ship() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::new(30_000.0, 0.0, 0.0),
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh admission should begin");
        let ship_id = attempt.ship_id();
        let resume_ticket = attempt.resume_ticket();
        assert_eq!(node.ship_count(), 0);
        assert!(matches!(
            node.pending_events(),
            [DomainEvent::ClientAdmissionIdentityReserved(_)]
        ));

        let committed = attempt.commit(&mut node).expect("fresh commit");

        assert_eq!(committed.ship_id, ship_id);
        assert!(!committed.resumed);
        assert_eq!(node.ship_count(), 1);
        assert!(node.prepared_fresh_admission(resume_ticket).is_some());

        commit_runtime_frame(&mut node);

        assert!(node.prepared_fresh_admission(resume_ticket).is_none());
        assert_eq!(
            node.station_item_count(
                committed.player_id,
                dawn_core::StationId(0),
                dawn_core::ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
            ),
            1
        );
    }

    #[test]
    fn fresh_begin_persists_identity_watermark_and_abort_releases_capacity() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh admission should begin");
        let player_id = attempt.player_id();
        let ship_id = attempt.ship_id();
        let starter_ship = dawn_core::ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE);

        assert_eq!(node.ship_count(), 0);
        assert!(!node.apply_stop_command_owned(player_id, ship_id));
        assert_eq!(
            node.station_item_count(player_id, dawn_core::StationId(0), starter_ship),
            0
        );
        assert!(matches!(
            node.pending_events(),
            [DomainEvent::ClientAdmissionIdentityReserved(_)]
        ));

        attempt.abort(&mut node);

        assert_eq!(node.ship_count(), 0);
        assert_eq!(node.pending_events().len(), 1);
        node.set_population_cap(1);
        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("abort must release capacity despite the retained prepared identity");
        retry.abort(&mut node);
    }

    #[test]
    fn aborted_fresh_identity_can_resume_the_exact_prepared_attempt() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::new(30_000.0, 0.0, 0.0),
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh admission");
        let player_id = attempt.player_id();
        let ship_id = attempt.ship_id();
        let resume_ticket = attempt.resume_ticket();
        attempt.abort(&mut node);

        let recovered = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("exact prepared identity is retryable");
        assert!(!recovered.is_resumed());
        recovered.commit(&mut node).expect("recovered fresh commit");
        assert!(node.apply_stop_command_owned(player_id, ship_id));
    }

    #[test]
    fn prepared_fresh_retry_rechecks_population_cap_after_abort() {
        let mut node = node();
        node.set_population_cap(1);

        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("first fresh admission");
        let resume_ticket = first.resume_ticket();
        first.abort(&mut node);

        let other = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("a different fresh identity should claim the released capacity");
        other
            .commit(&mut node)
            .expect("other fresh admission commits");

        assert!(matches!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            ),
            Err(ClientAdmissionRefusal::FreshAtPopulationCap)
        ));
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn failed_resume_leaves_pre_existing_ship_unowned_and_intact() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let resume_ticket = seeded_resume_ticket(&mut node, player_id, ship_id);
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("existing ship may begin resume");
        assert!(!node.apply_stop_command_owned(player_id, ship_id));

        attempt.abort(&mut node);

        assert_eq!(node.ship_count(), 1);
        assert!(node.ship_absolute_pos(ship_id).is_some());
        assert!(!node.apply_stop_command_owned(player_id, ship_id));
    }

    #[test]
    fn successful_resume_adopts_only_at_commit() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let resume_ticket = seeded_resume_ticket(&mut node, player_id, ship_id);
        let mut attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("existing ship may begin resume");
        let handoff = attempt.take_handoff_payload();
        let loadout = handoff
            .player_loadout
            .expect("resume handoff must include an await-sent projected loadout");
        assert_eq!(loadout.active_ship_id, Some(ship_id.raw()));
        assert!(loadout
            .owned_ships
            .iter()
            .any(|ship| ship.ship_id == ship_id.raw() && ship.is_active));
        assert!(handoff
            .initial_state
            .ships
            .iter()
            .any(|ship| ship.ship_id == ship_id.raw() && ship.is_player));
        assert!(!node.apply_stop_command_owned(player_id, ship_id));

        let committed = attempt.commit(&mut node).expect("resume commit");

        assert!(committed.resumed);
        let loadout = node
            .build_player_loadout_json(committed.ship_id)
            .expect("committed resume has a complete loadout");
        assert_eq!(loadout.active_ship_id, Some(ship_id.raw()));
        assert!(loadout
            .owned_ships
            .iter()
            .any(|ship| ship.ship_id == ship_id.raw() && ship.is_active));
        assert!(node.apply_stop_command_owned(player_id, ship_id));
    }

    #[test]
    fn resume_is_rejected_while_ship_is_in_transit() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let resume_ticket = seeded_resume_ticket(&mut node, player_id, ship_id);
        node.prepare_transit_commit(ship_id, SectorId(1), None)
            .expect("ship should enter transit");

        let refusal = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect_err("admission must not bypass Transit ownership");

        assert_eq!(refusal, ClientAdmissionRefusal::ResumeTicketInvalid);
    }

    #[test]
    fn reopening_repository_preserves_the_rotated_resume_ticket() {
        let db_file = tempfile::NamedTempFile::new().unwrap();
        let db_path = db_file.path().to_str().unwrap();
        let mut node = node();
        node.open_repositories(db_path).unwrap();

        let fresh = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh admission should begin");
        let original_ticket = fresh.resume_ticket();
        let committed = fresh
            .commit(&mut node)
            .expect("fresh admission should commit");
        commit_runtime_frame(&mut node);

        let resume = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: original_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("original ticket should resume once");
        let rotated_ticket = resume.resume_ticket();
        resume.commit(&mut node).expect("resume should commit");

        node.open_repositories(db_path)
            .expect("reopening the durable DB should reconcile grants");

        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: rotated_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("the rotated ticket must survive DB reconciliation");
        retry.abort(&mut node);

        let old_ticket_refusal = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: original_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("the consumed ticket must not be restored by reconciliation");
        assert_eq!(
            old_ticket_refusal,
            ClientAdmissionRefusal::ResumeTicketInvalid
        );
        assert!(node.is_active_ship(committed.player_id, committed.ship_id));
    }

    #[test]
    fn fresh_admission_skips_ship_ids_already_materialized_in_the_node() {
        let mut node = node();
        let materialized = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh admission should begin");

        assert_ne!(attempt.ship_id(), materialized);
        assert!(attempt.ship_id().0.counter() > materialized.0.counter());
        attempt.abort(&mut node);
    }

    #[test]
    fn concurrent_resume_attempts_for_one_ship_are_serialized() {
        let mut node = node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let first_player = PlayerId(12);
        let resume_ticket = seeded_resume_ticket(&mut node, first_player, ship_id);
        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("first resume obtains the Ship lock");

        let second = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect_err("second concurrent resume must be refused");
        assert_eq!(
            second,
            ClientAdmissionRefusal::ResumeAlreadyPending {
                player_id: first_player,
                ship_id,
            }
        );

        first.abort(&mut node);
        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("abort releases the Ship-level resume lock");
        retry.abort(&mut node);
    }

    #[test]
    fn one_player_cannot_hold_two_concurrent_resume_attempts() {
        let mut node = node();
        let player_id = PlayerId(12);
        let first_ship = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let second_ship = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let first_ticket = seeded_resume_ticket(&mut node, player_id, first_ship);
        let second_ticket = seeded_resume_ticket(&mut node, player_id, second_ship);
        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: first_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("first resume obtains the Player lock");
        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: second_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("same Player cannot concurrently resume another Ship"),
            ClientAdmissionRefusal::ResumeAlreadyPending {
                player_id,
                ship_id: second_ship,
            }
        );
        first.abort(&mut node);
    }

    #[test]
    fn established_owner_cannot_be_overwritten_by_another_player() {
        let mut node = node();
        let owner = PlayerId(12);
        let attacker = PlayerId(13);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let owner_ticket = seeded_resume_ticket(&mut node, owner, ship_id);
        node.begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: owner_ticket,
            },
            AOI_CELL_SIZE,
        )
        .expect("owner resume")
        .commit(&mut node)
        .expect("owner commit");

        let _ = attacker;
        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: ResumeTicket::from_bytes([99; ResumeTicket::BYTE_LEN]),
                },
                AOI_CELL_SIZE,
            )
            .expect_err("unknown Ticket cannot take an owned Ship"),
            ClientAdmissionRefusal::ResumeTicketInvalid
        );
    }

    #[test]
    fn exact_resume_identity_may_reconnect() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let owner_ticket = seeded_resume_ticket(&mut node, player_id, ship_id);
        node.begin_client_admission(
            ClientAdmissionIntent::Resume {
                resume_ticket: owner_ticket,
            },
            AOI_CELL_SIZE,
        )
        .expect("first resume")
        .commit(&mut node)
        .expect("first commit");
        let resume_ticket = node
            .client_resume_ticket(ship_id)
            .expect("committed ticket");
        assert_ne!(resume_ticket, owner_ticket);
        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: owner_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("a rotated ticket must invalidate the previous ticket"),
            ClientAdmissionRefusal::ResumeTicketInvalid
        );
        let reconnect = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { resume_ticket },
                AOI_CELL_SIZE,
            )
            .expect("same identity reconnects and replaces its runtime session");
        reconnect.abort(&mut node);
    }

    #[test]
    fn failed_resume_keeps_the_advertised_rotation_retryable() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let owner_ticket = seeded_resume_ticket(&mut node, player_id, ship_id);

        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: owner_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("resume should begin");
        let advertised_ticket = attempt.resume_ticket();
        attempt.abort(&mut node);

        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: advertised_ticket,
                },
                AOI_CELL_SIZE,
            )
            .expect("the ticket sent before the failed handshake must remain retryable");
        retry.abort(&mut node);
    }

    #[test]
    fn missing_resume_never_falls_back_to_fresh_spawn() {
        let mut node = node();
        let _player_id = PlayerId(12);
        let _ship_id = ShipId::new(NodeId(99), 1);

        let refusal = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    resume_ticket: ResumeTicket::from_bytes([88; ResumeTicket::BYTE_LEN]),
                },
                AOI_CELL_SIZE,
            )
            .expect_err("missing resume must be refused");

        assert_eq!(refusal, ClientAdmissionRefusal::ResumeTicketInvalid);
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn population_cap_refusal_has_no_identity_or_ship_residue() {
        let mut node = node();
        node.set_population_cap(0);

        let refusal = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("fresh admission at cap must be refused");

        assert_eq!(refusal, ClientAdmissionRefusal::FreshAtPopulationCap);
        assert_eq!(node.ship_count(), 0);
    }

    #[test]
    fn commit_rejects_a_lost_fresh_reservation() {
        let mut node = node();
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("fresh admission should begin");
        let ship_id = attempt.ship_id();
        node.abort_reserved_fresh_admission(ship_id);

        let error = attempt
            .commit(&mut node)
            .expect_err("lost reservation cannot commit");

        assert_eq!(error.ship_id, ship_id);
    }
}
