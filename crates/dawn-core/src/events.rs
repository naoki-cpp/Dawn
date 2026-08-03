//! Domain events — immutable facts that have occurred in the world.
//!
//! # Invariants
//!
//! - Events are **never** modified or deleted after being appended.  (INV-001)
//! - Every movement event carries a `tick`.  (INV-005, CLAUDE.md §6)
//! - Adding a new variant is permitted.
//! - Removing or renaming a variant: pre-release → direct deletion is fine;
//!   post-release → Upcaster + new ADR required.  (See CLAUDE.md §7)
//!
//! # Adding a new event
//!
//! 1. Add the variant here.
//! 2. Update `docs/architecture/event-catalog.md`.
//! 3. Add a corresponding `Command` in `commands.rs` if applicable.
//! 4. Write a unit test in this module.

use crate::fitting::{FittingSnapshot, ModuleId, SlotKind};
use crate::item::ItemId;
use crate::navigation::{AnchorId, JumpGateId, StarSystemId, StationId};
use crate::ship_type::ShipTypeId;
use crate::{AbsolutePosition, PlayerId, Position, SectorId, ShipId, Tick, Velocity};
use serde::{Deserialize, Serialize};

/// Every domain event that can be appended to the Event Log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DomainEvent {
    /// A new Ship entered the world.
    ShipSpawned(ShipSpawned),

    /// A Ship's velocity changed (authoritative movement event).
    ///
    /// Emitted by `MovementSystem` when velocity differs from the previous Tick.
    /// Position is derived state: `position += velocity` each Tick.
    /// See ADR-0008.
    VelocityChanged(VelocityChanged),

    /// A Ship was permanently removed from the world.
    ShipDespawned(ShipDespawned),

    /// A Ship's equipment loadout and/or cargo projection changed.
    ShipFitted(ShipFitted),

    /// An Active module was turned on.
    ModuleActivated(ModuleActivated),

    /// An Active module was turned off.
    ModuleDeactivated(ModuleDeactivated),

    /// A Ship completed locking onto a target.
    TargetLocked(TargetLocked),

    /// A lock was lost (target out of range or destroyed).
    LockLost(LockLost),

    /// A Ship fired its weapon at a target.
    WeaponFired(WeaponFired),

    /// A Ship took damage.
    DamageTaken(DamageTaken),

    /// A Ship repaired its own shield or armor.
    RepairApplied(RepairApplied),

    /// A Ship was destroyed.
    ShipDestroyed(ShipDestroyed),

    /// A Sector Transit was committed by Raft (ownership remains with `from`
    /// until `SectorTransitCompleted`). See ADR-0014.
    SectorTransitRequested(SectorTransitRequested),

    /// A Sector Transit completed; ownership moved from `from` to `to`.
    SectorTransitCompleted(SectorTransitCompleted),

    /// A committed Sector Transit was aborted; ownership remains with `from`.
    SectorTransitAborted(SectorTransitAborted),

    /// A Ship used a Jump Gate to move to another Sector. See ADR-0009.
    JumpGateUsed(JumpGateUsed),

    /// A Ship moved to another Star System (emitted alongside
    /// `JumpGateUsed` when the destination Sector belongs to a different
    /// Star System). See ADR-0009.
    StarSystemChanged(StarSystemChanged),

    /// A Fold Disruptor locked onto a target ship, preventing warp and jump.
    /// Emitted when a tackle module becomes effective (in range + locked).
    /// See ADR-0024.
    TackleApplied(TackleApplied),

    /// A tackle effect ended (module off, out of range, tackler destroyed, or
    /// lock lost). The tackled ship may still be tackled by other ships.
    /// See ADR-0024.
    TackleReleased(TackleReleased),

    /// A Ship's coordinate anchor changed (ADR-0029). The ship's absolute
    /// position is unchanged; only its `(anchor, offset)` representation is
    /// rebased — e.g. on warp arrival, from the Sector-origin anchor to the
    /// destination body's anchor. Authoritative: carries the post-rebase
    /// `anchor` and `offset` so replay reproduces the representation exactly
    /// (the offset change is discontinuous and not via `VelocityChanged`, so it
    /// must be recorded as its own fact — INV-MOVE is about velocity-driven
    /// motion, a frame rebase keeps the same absolute position).
    AnchorRebased(AnchorRebased),

    /// A ship docked at an NPC station.
    ShipDocked(ShipDocked),

    /// A ship undocked from an NPC station.
    ShipUndocked(ShipUndocked),

    /// Scrap Metal was consumed in a docked station to build a packaged ship.
    PackagedShipBuilt(PackagedShipBuilt),

    /// A docked ship was converted into a packaged ship item.
    ShipDisassembled(ShipDisassembled),

    /// A station-inventory packaged ship item was converted into a new live
    /// docked ship (ADR-0034 9B, ADR-0037).
    ShipAssembled(ShipAssembled),

    /// A fresh client admission durably consumed a PlayerId/ShipId pair.
    /// No Ship is materialized by this event; replay only advances the
    /// allocation watermarks so identities are never reused after a crash.
    ClientAdmissionIdentityReserved(ClientAdmissionIdentityReserved),

    /// A fresh client admission committed its complete starter state.
    /// Replay materializes the Ship, fitting/cargo projection, ownership, and
    /// idempotent Station-inventory grant from this single durable fact.
    ClientAdmissionCommitted(ClientAdmissionCommitted),
}

impl DomainEvent {
    /// The `ShipId` that this event relates to.
    pub fn ship_id(&self) -> ShipId {
        match self {
            Self::ShipSpawned(e) => e.ship_id,
            Self::VelocityChanged(e) => e.ship_id,
            Self::ShipDespawned(e) => e.ship_id,
            Self::ShipFitted(e) => e.ship_id,
            Self::ModuleActivated(e) => e.ship_id,
            Self::ModuleDeactivated(e) => e.ship_id,
            Self::TargetLocked(e) => e.locker_id,
            Self::LockLost(e) => e.locker_id,
            Self::WeaponFired(e) => e.attacker_id,
            Self::DamageTaken(e) => e.ship_id,
            Self::RepairApplied(e) => e.ship_id,
            Self::ShipDestroyed(e) => e.ship_id,
            Self::SectorTransitRequested(e) => e.ship_id,
            Self::SectorTransitCompleted(e) => e.handoff.ship_id,
            Self::SectorTransitAborted(e) => e.ship_id,
            Self::JumpGateUsed(e) => e.ship_id,
            Self::StarSystemChanged(e) => e.ship_id,
            Self::TackleApplied(e) => e.ship_id,
            Self::TackleReleased(e) => e.ship_id,
            Self::AnchorRebased(e) => e.ship_id,
            Self::ShipDocked(e) => e.ship_id,
            Self::ShipUndocked(e) => e.ship_id,
            Self::PackagedShipBuilt(e) => e.ship_id,
            Self::ShipDisassembled(e) => e.ship_id,
            Self::ShipAssembled(e) => e.ship_id,
            Self::ClientAdmissionIdentityReserved(e) => e.ship_id,
            Self::ClientAdmissionCommitted(e) => e.ship_id,
        }
    }

    /// The logical tick at which this event was produced.
    /// `Tick::ZERO` for creation events that precede the tick loop.
    pub fn tick(&self) -> Tick {
        match self {
            Self::ShipSpawned(e) => e.tick,
            Self::VelocityChanged(e) => e.tick,
            Self::ShipDespawned(e) => e.tick,
            Self::ShipFitted(e) => e.tick,
            Self::ModuleActivated(e) => e.tick,
            Self::ModuleDeactivated(e) => e.tick,
            Self::TargetLocked(e) => e.tick,
            Self::LockLost(e) => e.tick,
            Self::WeaponFired(e) => e.tick,
            Self::DamageTaken(e) => e.tick,
            Self::RepairApplied(e) => e.tick,
            Self::ShipDestroyed(e) => e.tick,
            Self::SectorTransitRequested(e) => e.tick,
            Self::SectorTransitCompleted(e) => e.tick,
            Self::SectorTransitAborted(e) => e.tick,
            Self::JumpGateUsed(e) => e.tick,
            Self::StarSystemChanged(e) => e.tick,
            Self::TackleApplied(e) => e.tick,
            Self::TackleReleased(e) => e.tick,
            Self::AnchorRebased(e) => e.tick,
            Self::ShipDocked(e) => e.tick,
            Self::ShipUndocked(e) => e.tick,
            Self::PackagedShipBuilt(e) => e.tick,
            Self::ShipDisassembled(e) => e.tick,
            Self::ShipAssembled(e) => e.tick,
            Self::ClientAdmissionIdentityReserved(e) => e.tick,
            Self::ClientAdmissionCommitted(e) => e.tick,
        }
    }
}

// ── ShipSpawned ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipSpawned {
    pub ship_id: ShipId,
    pub sector_id: SectorId,
    /// Authoritative Sector-frame spawn position.
    pub initial_position: AbsolutePosition,
    /// 船種 ID。Replay 時に base_stats を復元するために必須（INV-002）。
    pub ship_type_id: ShipTypeId,
    pub tick: Tick,
}

/// Durable allocation watermark for one fresh client admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientAdmissionIdentityReserved {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub tick: Tick,
}

/// Atomic, replay-complete starter state for one successful fresh admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientAdmissionCommitted {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub resume_ticket: crate::ResumeTicket,
    pub sector_id: SectorId,
    pub initial_position: AbsolutePosition,
    pub ship_type_id: ShipTypeId,
    pub fitting: FittingSnapshot,
    pub inventory: Vec<ItemId>,
    pub starter_station_id: StationId,
    pub starter_item_id: ItemId,
    pub starter_item_count: u64,
    pub tick: Tick,
}

// ── VelocityChanged ───────────────────────────────────────────────────────────

/// The authoritative movement event (ADR-0008).
///
/// Emitted by `MovementSystem` when a Ship's velocity differs from the previous Tick.
/// Ships with unchanged velocity emit no event.
///
/// Replay: apply `VelocityChanged` in order, then derive `position += velocity`
/// each Tick. No physics simulation required.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VelocityChanged {
    pub ship_id: ShipId,
    pub velocity: Velocity,
    pub tick: Tick,
}

// ── AnchorRebased ─────────────────────────────────────────────────────────────

/// A Ship's coordinate anchor changed (ADR-0029). Authoritative: the absolute
/// position is unchanged; `anchor`/`offset` are the post-rebase representation.
///
/// Replay: on apply, set the ship's anchor and position offset directly (the
/// rebase is a discontinuous frame change, not velocity-driven motion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchorRebased {
    pub ship_id: ShipId,
    pub anchor: AnchorId,
    /// New position offset, relative to `anchor` (metres).
    pub offset: Position,
    pub tick: Tick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDocked {
    pub ship_id: ShipId,
    pub station_id: StationId,
    pub tick: Tick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipUndocked {
    pub ship_id: ShipId,
    pub station_id: StationId,
    pub tick: Tick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackagedShipBuilt {
    pub ship_id: ShipId,
    pub player_id: crate::PlayerId,
    pub station_id: StationId,
    pub ship_type_id: ShipTypeId,
    pub scrap_cost: u64,
    pub tick: Tick,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDisassembled {
    pub ship_id: ShipId,
    pub player_id: crate::PlayerId,
    pub station_id: StationId,
    pub ship_type_id: ShipTypeId,
    pub tick: Tick,
}

/// A station-inventory `PackagedShip` item was converted into a new live
/// docked ship, owned by `player_id` (ADR-0034 9B, ADR-0037). `ship_id` is
/// freshly allocated -- never reused (INV-004). Does not change the
/// player's `active_ship`; a later `SelectActiveShipCommand` makes it active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipAssembled {
    pub ship_id: ShipId,
    pub player_id: crate::PlayerId,
    pub station_id: StationId,
    pub ship_type_id: ShipTypeId,
    pub tick: Tick,
}

// ── ShipDespawned ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDespawned {
    pub ship_id: ShipId,
    pub tick: Tick,
}

// ── ModuleActivated / ModuleDeactivated ──────────────────────────────────────

/// Active モジュールがオンになった（ADR-0006）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleActivated {
    pub ship_id: ShipId,
    pub module_id: ModuleId,
    pub slot: SlotKind,
    /// Target of a targeted module (Weapon/Tackle), per ADR-0035.
    /// `None` for self-only modules.
    pub target_ship_id: Option<ShipId>,
    pub tick: Tick,
}

/// Why a module was force-deactivated by a system rather than by the player
/// (ADR-0035). `None` on `ModuleDeactivated.forced_reason` means the player
/// issued `DeactivateModuleCommand` themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleDeactivationReason {
    /// `CapacitorSystem` forced it off — insufficient cap to start a cycle.
    CapacitorExhausted,
    /// `SimulationNode::process_range_gate` forced it off — the targeted
    /// module's target drifted beyond its effective range.
    OutOfRange,
}

/// Active モジュールがオフになった（ADR-0006）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDeactivated {
    pub ship_id: ShipId,
    pub module_id: ModuleId,
    pub slot: SlotKind,
    /// `None` for a player-issued deactivation; `Some(reason)` for a
    /// system-forced one (ADR-0035) so the client can show the right label
    /// instead of always assuming capacitor exhaustion.
    pub forced_reason: Option<ModuleDeactivationReason>,
    pub tick: Tick,
}

// ── TargetLocked ─────────────────────────────────────────────────────────────

/// ロック完了イベント。LockSystem がカウントダウンを完了したときに発行する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetLocked {
    pub locker_id: ShipId,
    pub target_id: ShipId,
    pub tick: Tick,
}

// ── LockLost ─────────────────────────────────────────────────────────────────

/// ロック消失イベント。ターゲットが射程外または撃沈されたときに発行する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockLost {
    pub locker_id: ShipId,
    pub target_id: ShipId,
    pub tick: Tick,
}

// ── ShipFitted ────────────────────────────────────────────────────────────────

/// 装備スロット全体と船内インベントリ全体のスナップショットを含む。
/// Event Replay 時に FittingComp / InventoryComp を完全復元するために必要（INV-002）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipFitted {
    pub ship_id: ShipId,
    /// 変更後の装備全体スナップショット
    pub fitting: FittingSnapshot,
    /// 変更後のインベントリ全体スナップショット（ADR-0032）。Fit/Unfit は常に
    /// 装備とインベントリの両方を同時に変えるため、新規イベント型を起こさず
    /// 既存の ShipFitted に同梱する。Market の片側 Item bridge command も
    /// 装備を変えずにこのスナップショットを更新するため、同じイベントを再利用する。
    #[serde(default)]
    pub inventory: Vec<ItemId>,
    pub tick: Tick,
}

// ── WeaponFired ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeaponFired {
    pub attacker_id: ShipId,
    pub target_id: ShipId,
    pub damage: f32,
    pub tick: Tick,
}

// ── DamageTaken ───────────────────────────────────────────────────────────────

/// HP は Shield → Armor → Hull の順に消費される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DamageTaken {
    pub ship_id: ShipId,
    pub damage: f32,
    /// ダメージ後のシールド残量
    pub current_shield: f32,
    /// ダメージ後のアーマー残量
    pub current_armor: f32,
    /// ダメージ後のハル残量
    pub current_hull: f32,
    pub tick: Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepairLayer {
    Shield,
    Armor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairApplied {
    pub ship_id: ShipId,
    pub amount: f32,
    pub layer: RepairLayer,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub tick: Tick,
}

// ── ShipDestroyed ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDestroyed {
    pub ship_id: ShipId,
    pub killer_id: ShipId,
    pub tick: Tick,
}

// ── Sector Transit (ADR-0014) ───────────────────────────────────────────────────

/// A Sector Transit was committed by Raft. Ownership of `ship_id` remains
/// with `from` until `SectorTransitCompleted` is appended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorTransitRequested {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    /// Source-Sector Tick that identifies this request. This is a source-local
    /// nonce, not the Tick of every EventStore that records the transfer.
    pub request_tick: Tick,
    /// Original transfer kind. `None` is a non-Gate Sector Transit and must not
    /// be inferred as a Jump after restart merely because topology has a Gate.
    pub gate_id: Option<JumpGateId>,
    /// Authoritative destination entry point in the destination Sector frame.
    /// Anchor selection and local-offset derivation happen only at destination materialization.
    pub entry_pos: AbsolutePosition,
    /// Tick local to the EventStore that appended this record.
    pub tick: Tick,
}

/// A Sector Transit completed; ownership of `handoff.ship_id` moved from
/// `from` to `to`.
///
/// `handoff` is the same Transit-owned state carried by the consensus Commit,
/// so destination snapshot-plus-tail replay can materialize the Ship without
/// an in-memory Raft actor. Persistence `ShipSnapshot` does not cross this
/// protocol boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorTransitCompleted {
    pub handoff: crate::transit::TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    /// Source-local identity of the Request this completion closes.
    pub request_tick: Tick,
    /// Authoritative destination-Sector entry position.
    pub entry_pos: AbsolutePosition,
    /// Tick local to the EventStore that appended this record.
    pub tick: Tick,
}

/// A committed Sector Transit was aborted after `SectorTransitRequested`.
/// Ownership of `ship_id` remains with `from`.
///
/// Pre-commit rejections (Ship not found, already in transit, etc.) are
/// expressed as `CommandRejected`, not as an event (INV-006). This event
/// only covers an abort *after* the transit was committed by Raft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectorTransitAborted {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub tick: Tick,
}

// ── Jump Gate Navigation (ADR-0009) ─────────────────────────────────────────────

/// A Ship used a Jump Gate to move to another Sector.
///
/// Like `SectorTransitCompleted`, this is the authoritative record of the
/// Sector change; `entry_pos` is required so Replay can place the Ship in
/// the destination Sector without re-running gate-proximity checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumpGateUsed {
    pub ship_id: ShipId,
    pub gate_id: JumpGateId,
    pub from_sector: SectorId,
    pub to_sector: SectorId,
    /// Authoritative destination position in the destination Sector frame.
    pub entry_pos: AbsolutePosition,
    pub tick: Tick,
}

/// A Ship moved to another Star System. Emitted alongside `JumpGateUsed`
/// only when `to_sector` belongs to a different `StarSystemId` than
/// `from_sector`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StarSystemChanged {
    pub ship_id: ShipId,
    pub from_system: StarSystemId,
    pub to_system: StarSystemId,
    pub tick: Tick,
}

// ── Tackle (ADR-0024) ─────────────────────────────────────────────────────────

/// A Fold Disruptor module began tackling a target ship.
///
/// Emitted when a Tackle module is active, the tackler has a lock on `ship_id`,
/// and `ship_id` is within `tackle_range`. The tackled ship cannot warp or jump
/// while at least one TackleApplied without a matching TackleReleased is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TackleApplied {
    /// Ship that is now tackled.
    pub ship_id: ShipId,
    /// Ship applying the tackle.
    pub by: ShipId,
    pub tick: Tick,
}

/// A tackle effect on a ship ended.
///
/// Emitted when a Tackle module is deactivated, the lock on `ship_id` is lost,
/// the tackler moves out of range, or the tackler is destroyed.
/// The tackled ship may still be tackled by other ships.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TackleReleased {
    /// Ship that was tackled.
    pub ship_id: ShipId,
    /// Ship that released (or lost) the tackle.
    pub by: ShipId,
    pub tick: Tick,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Position, Velocity};

    fn ship_id() -> ShipId {
        ShipId::new(NodeId(0), 1)
    }

    #[test]
    fn velocity_changed_event_carries_the_tick_at_which_it_occurred() {
        let event = DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ship_id(),
            velocity: Velocity::new(1.0, 0.0, 0.0),
            tick: Tick(42),
        });
        assert_eq!(event.tick(), Tick(42));
    }

    #[test]
    fn anchor_rebased_event_carries_ship_anchor_and_tick() {
        let event = DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: ship_id(),
            anchor: crate::AnchorId(3),
            offset: Position::new(10.0, 0.0, -5.0),
            tick: Tick(7),
        });
        assert_eq!(event.ship_id(), ship_id());
        assert_eq!(event.tick(), Tick(7));
    }

    #[test]
    fn domain_event_ship_id_accessor_returns_correct_id() {
        let id = ship_id();
        let event = DomainEvent::ShipSpawned(ShipSpawned {
            ship_id: id,
            sector_id: SectorId(0),
            initial_position: AbsolutePosition::ORIGIN,
            ship_type_id: crate::ship_type::ShipTypeId(1),
            tick: Tick::ZERO,
        });
        assert_eq!(event.ship_id(), id);
    }

    #[test]
    fn domain_event_is_serializable_and_round_trips_without_loss() {
        let original = DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ship_id(),
            velocity: Velocity::new(5.0, 3.0, 1.0),
            tick: Tick(100),
        });
        let bytes = bincode_roundtrip(&original);
        let restored = bincode_restore(&bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn sector_transit_requested_event_carries_ship_id_and_tick() {
        let id = ship_id();
        let event = DomainEvent::SectorTransitRequested(SectorTransitRequested {
            ship_id: id,
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick(7),
            gate_id: None,
            entry_pos: AbsolutePosition::ORIGIN,
            tick: Tick(7),
        });
        assert_eq!(event.ship_id(), id);
        assert_eq!(event.tick(), Tick(7));
    }

    #[test]
    fn repair_applied_event_carries_ship_id_layer_and_tick() {
        let id = ship_id();
        let event = DomainEvent::RepairApplied(RepairApplied {
            ship_id: id,
            amount: 25.0,
            layer: RepairLayer::Shield,
            current_shield: 75.0,
            current_armor: 50.0,
            current_hull: 40.0,
            tick: Tick(12),
        });
        assert_eq!(event.ship_id(), id);
        assert_eq!(event.tick(), Tick(12));
        match event {
            DomainEvent::RepairApplied(e) => assert_eq!(e.layer, RepairLayer::Shield),
            _ => panic!("expected RepairApplied"),
        }
    }

    #[test]
    fn sector_transit_completed_event_carries_entry_position_and_velocity() {
        let id = ship_id();
        let event = DomainEvent::SectorTransitCompleted(SectorTransitCompleted {
            handoff: crate::TransitHandoffState {
                ship_id: id,
                owner_player_id: None,
                resume_ticket: None,
                ship_type_id: ShipTypeId(1),
                velocity: Velocity::new(1.0, 0.0, 0.0),
                current_shield: 100.0,
                current_armor: 100.0,
                current_hull: 100.0,
                is_destroyed: false,
                capacitor: Some(50.0),
                fitting: FittingSnapshot::empty(),
                inventory: std::collections::BTreeMap::new(),
            },
            from: SectorId(0),
            to: SectorId(1),
            request_tick: Tick::ZERO,
            entry_pos: AbsolutePosition::new(100.0, 0.0, 0.0),
            tick: Tick(8),
        });
        match event {
            DomainEvent::SectorTransitCompleted(e) => {
                assert_eq!(e.entry_pos, AbsolutePosition::new(100.0, 0.0, 0.0));
                assert_eq!(e.to, SectorId(1));
            }
            _ => panic!("expected SectorTransitCompleted"),
        }
    }

    #[test]
    fn sector_transit_aborted_event_keeps_ownership_with_from_sector() {
        let id = ship_id();
        let event = DomainEvent::SectorTransitAborted(SectorTransitAborted {
            ship_id: id,
            from: SectorId(0),
            to: SectorId(1),
            tick: Tick(9),
        });
        assert_eq!(event.ship_id(), id);
        assert_eq!(event.tick(), Tick(9));
    }

    #[test]
    fn jump_gate_used_event_carries_destination_sector_and_entry_position() {
        let id = ship_id();
        let event = DomainEvent::JumpGateUsed(JumpGateUsed {
            ship_id: id,
            gate_id: crate::navigation::JumpGateId(0),
            from_sector: SectorId(0),
            to_sector: SectorId(1),
            entry_pos: AbsolutePosition::ORIGIN,
            tick: Tick(10),
        });
        assert_eq!(event.ship_id(), id);
        assert_eq!(event.tick(), Tick(10));
        match event {
            DomainEvent::JumpGateUsed(e) => assert_eq!(e.to_sector, SectorId(1)),
            _ => panic!("expected JumpGateUsed"),
        }
    }

    #[test]
    fn star_system_changed_event_carries_from_and_to_systems() {
        let id = ship_id();
        let event = DomainEvent::StarSystemChanged(StarSystemChanged {
            ship_id: id,
            from_system: crate::navigation::StarSystemId(0),
            to_system: crate::navigation::StarSystemId(1),
            tick: Tick(11),
        });
        assert_eq!(event.ship_id(), id);
        assert_eq!(event.tick(), Tick(11));
    }

    #[test]
    fn ship_assembled_event_carries_the_new_ships_identity() {
        let id = ship_id();
        let event = DomainEvent::ShipAssembled(ShipAssembled {
            ship_id: id,
            player_id: crate::PlayerId(3),
            station_id: StationId(0),
            ship_type_id: ShipTypeId(1),
            tick: Tick(12),
        });
        assert_eq!(event.ship_id(), id);
        assert_eq!(event.tick(), Tick(12));
        match event {
            DomainEvent::ShipAssembled(e) => assert_eq!(e.player_id, crate::PlayerId(3)),
            _ => panic!("expected ShipAssembled"),
        }
    }

    fn bincode_roundtrip(event: &DomainEvent) -> String {
        format!("{event:?}")
    }
    fn bincode_restore(s: &str) -> DomainEvent {
        let _ = s;
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id: ship_id(),
            velocity: Velocity::new(5.0, 3.0, 1.0),
            tick: Tick(100),
        })
    }
}
