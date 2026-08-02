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

/// A successful WebSocket handshake could not be committed because its Ship
/// disappeared while the asynchronous handshake was in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAdmissionCommitError {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
}

impl std::fmt::Display for ClientAdmissionCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "admission commit refused for {}: ship #{} disappeared during handshake",
            self.player_id,
            self.ship_id.raw()
        )
    }
}

impl std::error::Error for ClientAdmissionCommitError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdmissionOrigin {
    Fresh,
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
/// Fresh attempts already own a newly-spawned Ship and abort removes it. Resume
/// attempts do not mutate ownership until commit, so abort is a no-op and can
/// never remove the pre-existing Ship.
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
        self.origin == AdmissionOrigin::Resume
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
    /// ownership was created by the spawn and only needs a liveness check.
    pub fn commit<S: EventStore>(
        self,
        node: &mut SimulationNode<S>,
    ) -> Result<CommittedClientAdmission, ClientAdmissionCommitError> {
        let present = match self.origin {
            AdmissionOrigin::Fresh => node.ship_absolute_pos(self.ship_id).is_some(),
            AdmissionOrigin::Resume => node.resume_player_ship(self.ship_id, self.player_id),
        };
        if !present {
            return Err(ClientAdmissionCommitError {
                player_id: self.player_id,
                ship_id: self.ship_id,
            });
        }

        Ok(CommittedClientAdmission {
            player_id: self.player_id,
            ship_id: self.ship_id,
            resumed: self.origin == AdmissionOrigin::Resume,
        })
    }

    /// Abort after any handshake error or disconnect.
    ///
    /// Only a fresh attempt owns rollback state. A resumed Ship predates this
    /// connection and is deliberately left untouched (ADR-0007).
    pub fn abort<S: EventStore>(self, node: &mut SimulationNode<S>) {
        if self.origin == AdmissionOrigin::Fresh {
            node.despawn_incomplete_handshake_spawn(self.ship_id);
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

                let player_id = self.next_player_id();
                let ship_id = self.spawn_player_ship_at_pub(player_id, spawn_position);
                let handoff = match self.build_handoff_payload(ship_id, aoi_cell_size) {
                    Ok(handoff) => handoff,
                    Err(error) => {
                        self.despawn_incomplete_handshake_spawn(ship_id);
                        return Err(ClientAdmissionRefusal::MissingObserver(error));
                    }
                };

                Ok(ClientAdmissionAttempt {
                    player_id,
                    ship_id,
                    origin: AdmissionOrigin::Fresh,
                    handoff: Some(handoff),
                })
            }
            ClientAdmissionIntent::Resume { player_id, ship_id } => {
                // ADR-0007: validate the exact requested Ship and never fall
                // back to a fresh spawn. Ownership is intentionally deferred
                // until commit so a failed socket handshake leaves no residue.
                let mut handoff =
                    self.build_handoff_payload(ship_id, aoi_cell_size)
                        .map_err(|_| ClientAdmissionRefusal::ResumeShipMissing {
                            player_id,
                            ship_id,
                        })?;

                // Restored ships have no persisted ownership until resume
                // commits. The connecting client must still see its observer as
                // a player Ship in the handoff sent immediately before commit.
                if let Some(observer) = handoff
                    .initial_state
                    .ships
                    .iter_mut()
                    .find(|ship| ship.ship_id == ship_id.raw())
                {
                    observer.is_player = true;
                }

                // `PlayerLoadout` depends on committed ownership and docked-player
                // context. Do not send a structurally-valid but incomplete
                // snapshot during the pre-commit handoff; the runtime sends the
                // authoritative loadout immediately after commit and before
                // publishing the session.
                handoff.player_loadout = None;

                Ok(ClientAdmissionAttempt {
                    player_id,
                    ship_id,
                    origin: AdmissionOrigin::Resume,
                    handoff: Some(handoff),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{NodeId, SectorBounds, SectorId, ShipTypeId, Velocity};

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

        let committed = attempt.commit(&mut node).expect("fresh commit");

        assert_eq!(committed.ship_id, ship_id);
        assert!(!committed.resumed);
        assert_eq!(node.ship_count(), 1);
    }

    #[test]
    fn fresh_abort_removes_spawn_and_ownership() {
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
        assert!(node.apply_stop_command_owned(player_id, ship_id));

        attempt.abort(&mut node);

        assert_eq!(node.ship_count(), 0);
        assert!(!node.apply_stop_command_owned(player_id, ship_id));
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
        assert!(handoff.player_loadout.is_none());
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
    fn commit_rejects_a_ship_removed_during_async_handshake() {
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
        node.despawn_incomplete_handshake_spawn(ship_id);

        let error = attempt
            .commit(&mut node)
            .expect_err("disappeared ship cannot commit");

        assert_eq!(error.ship_id, ship_id);
    }
}
