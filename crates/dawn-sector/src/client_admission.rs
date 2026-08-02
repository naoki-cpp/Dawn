//! Transactional client admission owned by `dawn-sector`.
//!
//! Runtime adapters wait on sockets and promote committed sessions. This module
//! owns every authoritative mutation between those two points: population-cap
//! checks, fresh identity allocation and spawn, resume validation, observer-
//! scoped handoff construction, commit, and abort cleanup.

use dawn_core::{PlayerId, Position, ShipId};
use dawn_event_store::store::EventStore;

use crate::node::{HandoffPayload, MissingObserverShip, SimulationNode};

/// Runtime-provided identity intent for one connection attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientAdmissionIntent {
    /// Allocate a new player identity and spawn a new Ship at this position.
    Fresh { spawn_position: Position },
    /// Resume a Ship that must already exist in this Sector.
    Resume {
        player_id: PlayerId,
        ship_id: ShipId,
    },
}

/// Why a client admission attempt could not begin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientAdmissionRefusal {
    /// A fresh spawn would exceed the Sector population backstop.
    FreshAtPopulationCap,
    /// ADR-0007: a requested resume Ship is absent and must not fall back to a
    /// fresh spawn.
    ResumeShipMissing {
        player_id: PlayerId,
        ship_id: ShipId,
    },
    /// Another handshake already holds the Ship-level resume lock.
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
            Self::ResumeShipMissing { player_id, ship_id } => write!(
                f,
                "resume refused for {player_id}: ship #{} is not present in this Sector",
                ship_id.raw()
            ),
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
    Resume,
}

/// One begun but not yet committed client admission.
///
/// The attempt must be resolved exactly once after asynchronous handshake
/// completion:
///
/// - [`Self::commit`] after the socket successfully receives the handoff;
/// - [`Self::abort`] after any handshake failure or disconnect.
///
/// Fresh attempts hold only a durable identity watermark plus a non-durable
/// reservation; abort releases the reservation and never creates a Ship. Resume
/// attempts hold a non-authoritative Ship lock until commit; abort releases that
/// lock and never removes the pre-existing Ship.
#[derive(Debug)]
pub struct ClientAdmissionAttempt {
    player_id: PlayerId,
    ship_id: ShipId,
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

    pub fn is_resumed(&self) -> bool {
        matches!(self.origin, AdmissionOrigin::Resume)
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
    pub fn commit<S: EventStore>(
        self,
        node: &mut SimulationNode<S>,
    ) -> Result<CommittedClientAdmission, ClientAdmissionCommitError> {
        let present = match self.origin {
            AdmissionOrigin::Fresh { spawn_position } => {
                node.commit_reserved_fresh_admission(self.player_id, self.ship_id, spawn_position)
            }
            AdmissionOrigin::Resume => {
                node.commit_reserved_resume_admission(self.player_id, self.ship_id)
            }
        };
        if !present {
            match self.origin {
                AdmissionOrigin::Fresh { .. } => {
                    node.abort_reserved_fresh_admission(self.ship_id);
                }
                AdmissionOrigin::Resume => {
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
            resumed: matches!(self.origin, AdmissionOrigin::Resume),
        })
    }

    /// Abort after any handshake error or disconnect.
    ///
    /// A fresh attempt releases its non-durable capacity reservation while
    /// retaining the consumed identity watermark. A resume attempt releases
    /// only its non-authoritative Ship lock. Neither path removes a committed
    /// or pre-existing Ship.
    pub fn abort<S: EventStore>(self, node: &mut SimulationNode<S>) {
        match self.origin {
            AdmissionOrigin::Fresh { .. } => {
                node.abort_reserved_fresh_admission(self.ship_id);
            }
            AdmissionOrigin::Resume => {
                node.release_resume_admission(self.player_id, self.ship_id);
            }
        }
    }
}

impl<S: EventStore> SimulationNode<S> {
    /// Begin a transactional client admission and build its observer-scoped
    /// handoff payload.
    ///
    /// No runtime should separately allocate identity, spawn/adopt a Ship, or
    /// decide rollback. It should only pass socket-derived intent here, send the
    /// returned payload, then resolve the attempt with `commit` or `abort`.
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

                let (player_id, ship_id) = self.reserve_fresh_admission_identity();
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
                    origin: AdmissionOrigin::Fresh { spawn_position },
                    handoff: Some(handoff),
                })
            }
            ClientAdmissionIntent::Resume { player_id, ship_id } => {
                // ADR-0007: validate the exact requested Ship and never fall
                // back to a fresh spawn. A Ship-level reservation serializes
                // concurrent resume handshakes until this attempt commits or aborts.
                if self.ship_absolute_pos(ship_id).is_none() {
                    return Err(ClientAdmissionRefusal::ResumeShipMissing { player_id, ship_id });
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
                    let mut handoff =
                        self.build_handoff_payload(ship_id, aoi_cell_size)
                            .map_err(|_| ClientAdmissionRefusal::ResumeShipMissing {
                                player_id,
                                ship_id,
                            })?;

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
                        .ok_or(ClientAdmissionRefusal::ResumeShipMissing { player_id, ship_id })?;
                    handoff.player_loadout = Some(loadout);
                    Ok(handoff)
                })();

                match result {
                    Ok(handoff) => Ok(ClientAdmissionAttempt {
                        player_id,
                        ship_id,
                        origin: AdmissionOrigin::Resume,
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
        SimulationNode::new(
            NodeId(7),
            SectorId(3),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
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
        assert_eq!(node.ship_count(), 0);
        assert!(matches!(
            node.event_store().all_records(),
            [record] if matches!(record.event, DomainEvent::ClientAdmissionIdentityReserved(_))
        ));

        let committed = attempt.commit(&mut node).expect("fresh commit");

        assert_eq!(committed.ship_id, ship_id);
        assert!(!committed.resumed);
        assert_eq!(node.ship_count(), 1);
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
            node.event_store().all_records(),
            [record] if matches!(record.event, DomainEvent::ClientAdmissionIdentityReserved(_))
        ));

        attempt.abort(&mut node);

        assert_eq!(node.ship_count(), 0);
        assert_eq!(node.event_store().all_records().len(), 1);
        node.set_population_cap(1);
        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Fresh {
                    spawn_position: Position::ORIGIN,
                },
                AOI_CELL_SIZE,
            )
            .expect("abort must release capacity despite the retained watermark");
        retry.abort(&mut node);
    }

    #[test]
    fn failed_resume_leaves_pre_existing_ship_unowned_and_intact() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
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
        let mut attempt = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
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
    fn concurrent_resume_attempts_for_one_ship_are_serialized() {
        let mut node = node();
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        let first_player = PlayerId(12);
        let second_player = PlayerId(13);
        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id: first_player,
                    ship_id,
                },
                AOI_CELL_SIZE,
            )
            .expect("first resume obtains the Ship lock");

        let second = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id: second_player,
                    ship_id,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("second concurrent resume must be refused");
        assert_eq!(
            second,
            ClientAdmissionRefusal::ResumeAlreadyPending {
                player_id: second_player,
                ship_id,
            }
        );

        first.abort(&mut node);
        let retry = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id: second_player,
                    ship_id,
                },
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
        let first = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id,
                    ship_id: first_ship,
                },
                AOI_CELL_SIZE,
            )
            .expect("first resume obtains the Player lock");
        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id,
                    ship_id: second_ship,
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
        node.begin_client_admission(
            ClientAdmissionIntent::Resume {
                player_id: owner,
                ship_id,
            },
            AOI_CELL_SIZE,
        )
        .expect("owner resume")
        .commit(&mut node)
        .expect("owner commit");

        assert_eq!(
            node.begin_client_admission(
                ClientAdmissionIntent::Resume {
                    player_id: attacker,
                    ship_id,
                },
                AOI_CELL_SIZE,
            )
            .expect_err("different Player cannot take an owned Ship"),
            ClientAdmissionRefusal::ResumeIdentityConflict {
                player_id: attacker,
                ship_id,
            }
        );
    }

    #[test]
    fn exact_resume_identity_may_reconnect() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = node.spawn_ship(ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
        node.begin_client_admission(
            ClientAdmissionIntent::Resume { player_id, ship_id },
            AOI_CELL_SIZE,
        )
        .expect("first resume")
        .commit(&mut node)
        .expect("first commit");
        let reconnect = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
                AOI_CELL_SIZE,
            )
            .expect("same identity reconnects and replaces its runtime session");
        reconnect.abort(&mut node);
    }

    #[test]
    fn missing_resume_never_falls_back_to_fresh_spawn() {
        let mut node = node();
        let player_id = PlayerId(12);
        let ship_id = ShipId::new(NodeId(99), 1);

        let refusal = node
            .begin_client_admission(
                ClientAdmissionIntent::Resume { player_id, ship_id },
                AOI_CELL_SIZE,
            )
            .expect_err("missing resume must be refused");

        assert_eq!(
            refusal,
            ClientAdmissionRefusal::ResumeShipMissing { player_id, ship_id }
        );
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
