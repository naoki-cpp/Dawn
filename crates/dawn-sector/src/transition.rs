//! Storage-independent Sector transition boundary (ADR-0049 / issue #272).
//!
//! The pure part of a command must decide its outcome without mutating live
//! state or knowing which journal will persist it. Runtime adapters persist
//! the returned transition and only then ask the authoritative state owner to
//! apply the same recovery delta.

use dawn_core::{DomainEvent, SectorId, ShipId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Version of the recovery-delta payload produced by this module.
pub const RECOVERY_DELTA_VERSION: u16 = 1;

/// Opaque identity for one logical Sector transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SectorTransitionId(pub u128);

/// Durability context bound to a prepared transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransitionContext {
    pub sector_id: SectorId,
    pub owner_epoch: u64,
}

/// Read-only facts required to prepare a Stop command.
///
/// This is deliberately smaller than `SimulationNode`: command policy can be
/// tested without constructing an ECS world or a persistence adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopCommandState {
    pub ship_id: ShipId,
    pub exists: bool,
    pub is_docked: bool,
    pub is_in_transit: bool,
    pub is_warping: bool,
}

/// Exact authoritative change made by a successful Stop command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRecoveryDelta {
    pub ship_id: ShipId,
}

/// Versioned authoritative state delta carried by a durable transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SectorRecoveryDelta {
    Stop(StopRecoveryDelta),
}

/// A complete transition prepared before any live authoritative mutation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedSectorTransition {
    pub transition_id: SectorTransitionId,
    pub context: TransitionContext,
    pub recovery_delta: SectorRecoveryDelta,
    pub public_events: Vec<DomainEvent>,
    pub reliable_effects: Vec<Vec<u8>>,
}

/// Rejection returned by the pure command policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("ship {0:?} does not exist")]
    UnknownShip(ShipId),
    #[error("ship {0:?} is docked")]
    Docked(ShipId),
    #[error("ship {0:?} is in transit")]
    InTransit(ShipId),
    #[error("ship {0:?} is in committed warp")]
    Warping(ShipId),
}

/// Error returned when a recovery-delta payload cannot be encoded or decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionCodecError {
    #[error("recovery delta encoding failed: {0}")]
    Encode(String),
    #[error("recovery delta decoding failed: {0}")]
    Decode(String),
    #[error("unsupported recovery delta version {actual}; expected {expected}")]
    UnsupportedVersion { actual: u16, expected: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionedRecoveryDelta {
    version: u16,
    delta: SectorRecoveryDelta,
}

/// Encode a versioned authoritative recovery delta for the journal adapter.
pub fn encode_recovery_delta(delta: &SectorRecoveryDelta) -> Result<Vec<u8>, TransitionCodecError> {
    postcard::to_stdvec(&VersionedRecoveryDelta {
        version: RECOVERY_DELTA_VERSION,
        delta: *delta,
    })
    .map_err(|error| TransitionCodecError::Encode(error.to_string()))
}

/// Decode a versioned recovery delta for restart or replica recovery.
pub fn decode_recovery_delta(payload: &[u8]) -> Result<SectorRecoveryDelta, TransitionCodecError> {
    let encoded: VersionedRecoveryDelta = postcard::from_bytes(payload)
        .map_err(|error| TransitionCodecError::Decode(error.to_string()))?;
    if encoded.version != RECOVERY_DELTA_VERSION {
        return Err(TransitionCodecError::UnsupportedVersion {
            actual: encoded.version,
            expected: RECOVERY_DELTA_VERSION,
        });
    }
    Ok(encoded.delta)
}

/// Pure Sector command policy and transition constructor.
#[derive(Debug, Clone, Copy, Default)]
pub struct SectorEngine;

impl SectorEngine {
    /// Prepare Stop without changing the supplied state.
    pub fn prepare_stop(
        state: StopCommandState,
        transition_id: SectorTransitionId,
        context: TransitionContext,
    ) -> Result<PreparedSectorTransition, TransitionError> {
        if !state.exists {
            return Err(TransitionError::UnknownShip(state.ship_id));
        }
        if state.is_docked {
            return Err(TransitionError::Docked(state.ship_id));
        }
        if state.is_in_transit {
            return Err(TransitionError::InTransit(state.ship_id));
        }
        if state.is_warping {
            return Err(TransitionError::Warping(state.ship_id));
        }

        Ok(PreparedSectorTransition {
            transition_id,
            context,
            recovery_delta: SectorRecoveryDelta::Stop(StopRecoveryDelta {
                ship_id: state.ship_id,
            }),
            public_events: Vec::new(),
            reliable_effects: Vec::new(),
        })
    }
}

/// Error raised while applying a prepared delta to the live Sector state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransitionApplyError {
    #[error("prepared transition references unknown ship {0:?}")]
    UnknownShip(ShipId),
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::NodeId;

    fn ship() -> ShipId {
        ShipId::new(NodeId(0), 1)
    }

    fn context() -> TransitionContext {
        TransitionContext {
            sector_id: SectorId(0),
            owner_epoch: 7,
        }
    }

    #[test]
    fn stop_preparation_is_pure_and_contains_the_authoritative_delta() {
        let state = StopCommandState {
            ship_id: ship(),
            exists: true,
            is_docked: false,
            is_in_transit: false,
            is_warping: false,
        };

        let prepared = SectorEngine::prepare_stop(state, SectorTransitionId(11), context())
            .expect("valid Stop command");

        assert!(!state.is_warping);
        assert_eq!(prepared.transition_id, SectorTransitionId(11));
        assert_eq!(prepared.context, context());
        assert_eq!(prepared.public_events, Vec::new());
        assert_eq!(
            prepared.recovery_delta,
            SectorRecoveryDelta::Stop(StopRecoveryDelta { ship_id: ship() })
        );
    }

    #[test]
    fn recovery_delta_round_trips_through_the_version_gate() {
        let delta = SectorRecoveryDelta::Stop(StopRecoveryDelta { ship_id: ship() });
        let payload = encode_recovery_delta(&delta).expect("delta should encode");

        assert_eq!(decode_recovery_delta(&payload), Ok(delta));
    }

    #[test]
    fn recovery_delta_rejects_an_unknown_version() {
        let payload = postcard::to_stdvec(&VersionedRecoveryDelta {
            version: RECOVERY_DELTA_VERSION + 1,
            delta: SectorRecoveryDelta::Stop(StopRecoveryDelta { ship_id: ship() }),
        })
        .unwrap();

        assert_eq!(
            decode_recovery_delta(&payload),
            Err(TransitionCodecError::UnsupportedVersion {
                actual: RECOVERY_DELTA_VERSION + 1,
                expected: RECOVERY_DELTA_VERSION,
            })
        );
    }

    #[test]
    fn stop_rejects_every_state_that_cannot_be_steered() {
        let base = StopCommandState {
            ship_id: ship(),
            exists: true,
            is_docked: false,
            is_in_transit: false,
            is_warping: false,
        };

        assert_eq!(
            SectorEngine::prepare_stop(
                StopCommandState {
                    exists: false,
                    ..base
                },
                SectorTransitionId(1),
                context()
            ),
            Err(TransitionError::UnknownShip(ship()))
        );
        assert_eq!(
            SectorEngine::prepare_stop(
                StopCommandState {
                    is_docked: true,
                    ..base
                },
                SectorTransitionId(1),
                context()
            ),
            Err(TransitionError::Docked(ship()))
        );
        assert_eq!(
            SectorEngine::prepare_stop(
                StopCommandState {
                    is_in_transit: true,
                    ..base
                },
                SectorTransitionId(1),
                context()
            ),
            Err(TransitionError::InTransit(ship()))
        );
        assert_eq!(
            SectorEngine::prepare_stop(
                StopCommandState {
                    is_warping: true,
                    ..base
                },
                SectorTransitionId(1),
                context()
            ),
            Err(TransitionError::Warping(ship()))
        );
    }
}
