//! Storage-independent Sector transition boundary (ADR-0049 / issue #272).
//!
//! The pure part of a command must decide its outcome without mutating live
//! state or knowing which journal will persist it. Runtime adapters persist
//! the returned transition and only then ask the authoritative state owner to
//! apply the same recovery delta.

use dawn_core::{
    ApproachTarget, CreditItemCommand, DomainEvent, JumpGateId, LockOnCommand, PlayerId,
    RemoveItemCommand, ReturnItemCommand, SectorId, ShipId, StationId, Tick, Velocity, WarpTarget,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use crate::persistence::ShipSnapshot;

/// One composition-adapter mutation admitted into a Tick's prepare/durable/
/// apply pipeline alongside lock commands (issue #315). Each variant mirrors
/// one of the item-mutation commands `SimulationNode` already exposes
/// (`ship_cargo.rs`); Market settlement delivery constructs these instead of
/// calling those methods directly outside the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketSettlementInput {
    RemoveItem(RemoveItemCommand),
    ReturnItem(ReturnItemCommand),
    CreditItem(CreditItemCommand),
}

impl MarketSettlementInput {
    pub fn settlement_id(&self) -> u64 {
        match self {
            Self::RemoveItem(cmd) => cmd.settlement_id,
            Self::ReturnItem(cmd) => cmd.settlement_id,
            Self::CreditItem(cmd) => cmd.settlement_id,
        }
    }
}

/// Result of admitting one `MarketSettlementInput` into a Tick's write set
/// (issue #315). `applied` mirrors the ownership/idempotency checks the
/// underlying `SimulationNode::*_item_owned` method already performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketSettlementOutcome {
    pub settlement_id: u64,
    pub applied: bool,
}

/// Frame-scoped input to one Tick's prepare/durable/apply pipeline (issue
/// #315). Bundles the existing lock-command input with Market settlement
/// mutations so both are captured in the same durable `TickRecoveryDelta`
/// rather than Market settlement mutating live state outside any tick
/// boundary.
#[derive(Debug, Clone, Copy)]
pub struct FrameInput<'a> {
    pub lock_commands: &'a [LockOnCommand],
    pub market_settlements: &'a [MarketSettlementInput],
}

impl<'a> FrameInput<'a> {
    /// A frame with no Market settlement input, e.g. tests and non-Market
    /// runtime call sites that only ever carried lock commands.
    pub fn lock_only(lock_commands: &'a [LockOnCommand]) -> Self {
        Self {
            lock_commands,
            market_settlements: &[],
        }
    }
}

/// Version of the recovery-delta payload produced by this module.
///
/// Bumped 1 -> 2 (ADR-0049, issue #312): added `applied_market_settlements`,
/// which was already in `StateSnapshot` but missing from
/// `TickRecoveryDelta` -- a lost settlement ACK became replayable after a
/// tick-rollback but not after a full checkpoint restore. Pre-release; no
/// upcaster required.
pub const RECOVERY_DELTA_VERSION: u16 = 2;

/// Node-level scalar/collection authoritative state that both a tick's
/// recovery delta and a checkpoint must reproduce exactly (ADR-0049, issue
/// #312) -- captured and restored through one function pair
/// (`SimulationNode::capture_node_state`/`restore_node_state`) instead of
/// `TickRecoveryDelta` and `StateSnapshot` construction/restoration each
/// hand-duplicating the same field list. Ship-level state is `ShipState`,
/// captured separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeState {
    pub id_counter: u64,
    pub player_id_counter: u64,
    pub transit_attempt_counter: u64,
    pub active_ships: BTreeMap<PlayerId, ShipId>,
    pub owners: BTreeMap<ShipId, PlayerId>,
    pub docked_ships: BTreeMap<ShipId, StationId>,
    pub docked_players: BTreeMap<PlayerId, StationId>,
    pub pending_bot_lock_commands: Vec<LockOnCommand>,
    pub pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    pub applied_market_settlements: Vec<u64>,
    pub transit_saga: crate::persistence::TransitSagaSnapshot,
}

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

/// Read-only facts required to prepare the logical Tick transition.
///
/// The first Tick vertical slice owns the counter boundary. System write sets
/// are migrated independently so this contract does not pretend that the
/// legacy ECS Tick is already storage-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickCommandState {
    pub current_tick: Tick,
}

/// Exact authoritative change made by a successful Stop command.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StopRecoveryDelta {
    pub ship_id: ShipId,
    pub clear_warp: bool,
    pub clear_steering: bool,
    pub thrust: StopThrustState,
}

/// Final thrust write made by the Stop reducer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StopThrustState {
    pub direction: Velocity,
    pub is_braking: bool,
}

/// Exact authoritative logical-time change made by a successful Tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickRecoveryDelta {
    pub from: Tick,
    pub to: Tick,
    pub mode: TickRecoveryMode,
    /// Changed ship state after the tick. Unchanged ships are omitted so the
    /// durable record is a write set rather than a whole-world snapshot.
    pub ship_states: Vec<ShipState>,
    /// Ships whose entities are removed by the committed tick.
    pub removed_ships: Vec<ShipId>,
    /// Cross-tick authoritative queues produced by the Bot and Warp systems.
    pub pending_bot_lock_commands: Vec<LockOnCommand>,
    pub pending_auto_jumps: Vec<(ShipId, JumpGateId)>,
    /// Presentation corrections are carried with the transition output so a
    /// runtime can publish them after the durable commit.
    pub completed_warps: Vec<ShipId>,
    /// Source-local Transit attempt allocator watermark.
    pub transit_attempt_counter: u64,
    /// Node-level authority captured after the tick. These fields make the
    /// delta include command mutations that happened before the tick runner
    /// prepared its write set, so a successful tick is a complete recovery
    /// boundary rather than only a movement diff.
    pub id_counter: u64,
    pub player_id_counter: u64,
    pub active_ships: BTreeMap<PlayerId, ShipId>,
    pub owners: BTreeMap<ShipId, PlayerId>,
    pub docked_ships: BTreeMap<ShipId, StationId>,
    pub docked_players: BTreeMap<PlayerId, StationId>,
    /// Market settlement identities already applied to ship cargo. Keeping
    /// these in the delta makes a lost settlement ACK safe to retry, the
    /// same guarantee the checkpoint already gave (ADR-0049, issue #312).
    pub applied_market_settlements: Vec<u64>,
    pub transit_saga: crate::persistence::TransitSagaSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickRecoveryMode {
    Logical,
    Full,
}

impl TickRecoveryDelta {
    pub fn logical(from: Tick, to: Tick) -> Self {
        Self {
            from,
            to,
            mode: TickRecoveryMode::Logical,
            ship_states: Vec::new(),
            removed_ships: Vec::new(),
            pending_bot_lock_commands: Vec::new(),
            pending_auto_jumps: Vec::new(),
            completed_warps: Vec::new(),
            transit_attempt_counter: 0,
            id_counter: 0,
            player_id_counter: 0,
            active_ships: BTreeMap::new(),
            owners: BTreeMap::new(),
            docked_ships: BTreeMap::new(),
            docked_players: BTreeMap::new(),
            applied_market_settlements: Vec::new(),
            transit_saga: crate::persistence::TransitSagaSnapshot::default(),
        }
    }
}

/// Canonical authoritative state of one ship (ADR-0049, issue #312): the
/// single type used to capture and restore a ship for the tick-prepare
/// rollback, `TickRecoveryDelta`, and `StateSnapshot` alike, so all three
/// read the same optional-component list (`dawn_ecs::OptionalShipComponents`,
/// via `SimulationNode::capture_tick_ship_state`/`apply_tick_ship_state`)
/// instead of each maintaining its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipState {
    pub snapshot: ShipSnapshot,
    pub fitting_present: bool,
    pub inventory_present: bool,
    pub movement: TickMovementState,
    pub fitting: TickFittingState,
    pub locks: Option<TickLockState>,
    pub weapon_last_fired_tick: Option<Tick>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickMovementState {
    pub thrust: Option<StopThrustState>,
    pub approach: Option<TickApproachState>,
    pub orbit: Option<TickOrbitState>,
    pub keep_at_range: Option<TickKeepAtRangeState>,
    pub warp: Option<TickWarpState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TickApproachState {
    pub target: ApproachTarget,
    pub auto_jump_gate: Option<JumpGateId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TickOrbitState {
    pub target: ApproachTarget,
    pub radius: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TickKeepAtRangeState {
    pub target: ApproachTarget,
    pub range: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TickWarpState {
    pub target: WarpTarget,
    pub phase: TickWarpPhase,
    pub auto_jump: bool,
    pub warp_start_abs: dawn_core::AbsolutePosition,
    pub warp_total: u32,
    pub warp_elapsed: u32,
    pub warp_arrival_abs: dawn_core::AbsolutePosition,
    pub warp_start_vel: Velocity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickWarpPhase {
    Aligning,
    Warping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TickFittingState {
    pub high: Vec<TickSlotState>,
    pub mid: Vec<TickSlotState>,
    pub low: Vec<TickSlotState>,
    pub rig: Vec<TickSlotState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickSlotState {
    pub module_id: dawn_core::fitting::ModuleId,
    pub is_active: bool,
    pub cycle_remaining: u64,
    pub target_ship_id: Option<ShipId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickLockState {
    pub entries: Vec<TickLockEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickLockEntry {
    pub target_id: ShipId,
    pub state: TickLockPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TickLockPhase {
    Locking { remaining_ticks: u64 },
    Locked,
}

/// Versioned authoritative state delta carried by a durable transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SectorRecoveryDelta {
    Stop(StopRecoveryDelta),
    Tick(Box<TickRecoveryDelta>),
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
    #[error("logical Tick overflow at {current}")]
    TickOverflow { current: Tick },
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
        delta: delta.clone(),
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
                clear_warp: true,
                clear_steering: true,
                thrust: StopThrustState {
                    direction: Velocity::ZERO,
                    is_braking: true,
                },
            }),
            public_events: Vec::new(),
            reliable_effects: Vec::new(),
        })
    }

    /// Prepare the logical Tick counter transition without changing state.
    pub fn prepare_tick(
        state: TickCommandState,
        transition_id: SectorTransitionId,
        context: TransitionContext,
    ) -> Result<PreparedSectorTransition, TransitionError> {
        let Some(next_tick) = state.current_tick.0.checked_add(1).map(Tick) else {
            return Err(TransitionError::TickOverflow {
                current: state.current_tick,
            });
        };

        Ok(PreparedSectorTransition {
            transition_id,
            context,
            recovery_delta: SectorRecoveryDelta::Tick(Box::new(TickRecoveryDelta::logical(
                state.current_tick,
                next_tick,
            ))),
            public_events: Vec::new(),
            reliable_effects: Vec::new(),
        })
    }
}

/// Error raised while applying a prepared delta to the live Sector state.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionApplyError {
    #[error("prepared transition references unknown ship {0:?}")]
    UnknownShip(ShipId),
    #[error("prepared Tick transition contains invalid state for ship {0:?}")]
    InvalidTickShipState(ShipId),
    #[error("prepared Tick transition targets sector {actual:?}; expected {expected:?}")]
    SectorMismatch {
        expected: SectorId,
        actual: SectorId,
    },
    #[error("prepared Tick transition expected current {expected}, found {actual}")]
    TickMismatch { expected: Tick, actual: Tick },
    #[error("prepared Tick transition must advance exactly one step: {from} -> {to}")]
    InvalidTickStep { from: Tick, to: Tick },
    #[error("prepared Tick contains invalid Transit Saga state: {0}")]
    InvalidTransitSaga(String),
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
            SectorRecoveryDelta::Stop(StopRecoveryDelta {
                ship_id: ship(),
                clear_warp: true,
                clear_steering: true,
                thrust: StopThrustState {
                    direction: Velocity::ZERO,
                    is_braking: true,
                },
            })
        );
    }

    #[test]
    fn tick_preparation_is_pure_and_contains_the_counter_delta() {
        let state = TickCommandState {
            current_tick: Tick(41),
        };

        let prepared = SectorEngine::prepare_tick(state, SectorTransitionId(12), context())
            .expect("valid Tick transition");

        assert_eq!(state.current_tick, Tick(41));
        assert_eq!(
            prepared.recovery_delta,
            SectorRecoveryDelta::Tick(Box::new(TickRecoveryDelta::logical(Tick(41), Tick(42))))
        );
    }

    #[test]
    fn tick_preparation_rejects_counter_overflow() {
        assert_eq!(
            SectorEngine::prepare_tick(
                TickCommandState {
                    current_tick: Tick(u64::MAX),
                },
                SectorTransitionId(13),
                context()
            ),
            Err(TransitionError::TickOverflow {
                current: Tick(u64::MAX),
            })
        );
    }

    #[test]
    fn recovery_delta_round_trips_through_the_version_gate() {
        let delta = SectorRecoveryDelta::Stop(StopRecoveryDelta {
            ship_id: ship(),
            clear_warp: true,
            clear_steering: true,
            thrust: StopThrustState {
                direction: Velocity::ZERO,
                is_braking: true,
            },
        });
        let payload = encode_recovery_delta(&delta).expect("delta should encode");

        assert_eq!(decode_recovery_delta(&payload), Ok(delta));
    }

    #[test]
    fn tick_recovery_delta_round_trips_through_the_version_gate() {
        let delta =
            SectorRecoveryDelta::Tick(Box::new(TickRecoveryDelta::logical(Tick(41), Tick(42))));
        let payload = encode_recovery_delta(&delta).expect("delta should encode");

        assert_eq!(decode_recovery_delta(&payload), Ok(delta));
    }

    #[test]
    fn recovery_delta_rejects_an_unknown_version() {
        let payload = postcard::to_stdvec(&VersionedRecoveryDelta {
            version: RECOVERY_DELTA_VERSION + 1,
            delta: SectorRecoveryDelta::Stop(StopRecoveryDelta {
                ship_id: ship(),
                clear_warp: true,
                clear_steering: true,
                thrust: StopThrustState {
                    direction: Velocity::ZERO,
                    is_braking: true,
                },
            }),
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
