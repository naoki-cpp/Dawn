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

use crate::fitting::{FittingSnapshot, SlotKind, ModuleId};
use crate::ship_type::ShipTypeId;
use crate::{Position, SectorId, ShipId, Tick, Velocity};
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

    /// A Ship's equipment loadout changed.
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

    /// A Ship was destroyed.
    ShipDestroyed(ShipDestroyed),
}

impl DomainEvent {
    /// The `ShipId` that this event relates to.
    pub fn ship_id(&self) -> ShipId {
        match self {
            Self::ShipSpawned(e)        => e.ship_id,
            Self::VelocityChanged(e)    => e.ship_id,
            Self::ShipDespawned(e)      => e.ship_id,
            Self::ShipFitted(e)         => e.ship_id,
            Self::ModuleActivated(e)    => e.ship_id,
            Self::ModuleDeactivated(e)  => e.ship_id,
            Self::TargetLocked(e)  => e.locker_id,
            Self::LockLost(e)      => e.locker_id,
            Self::WeaponFired(e)   => e.attacker_id,
            Self::DamageTaken(e)   => e.ship_id,
            Self::ShipDestroyed(e) => e.ship_id,
        }
    }

    /// The logical tick at which this event was produced.
    /// `Tick::ZERO` for creation events that precede the tick loop.
    pub fn tick(&self) -> Tick {
        match self {
            Self::ShipSpawned(e)        => e.tick,
            Self::VelocityChanged(e)    => e.tick,
            Self::ShipDespawned(e)      => e.tick,
            Self::ShipFitted(e)         => e.tick,
            Self::ModuleActivated(e)    => e.tick,
            Self::ModuleDeactivated(e)  => e.tick,
            Self::TargetLocked(e)  => e.tick,
            Self::LockLost(e)      => e.tick,
            Self::WeaponFired(e)   => e.tick,
            Self::DamageTaken(e)   => e.tick,
            Self::ShipDestroyed(e) => e.tick,
        }
    }
}

// ── ShipSpawned ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipSpawned {
    pub ship_id          : ShipId,
    pub sector_id        : SectorId,
    pub initial_position : Position,
    /// 船種 ID。Replay 時に base_stats を復元するために必須（INV-002）。
    pub ship_type_id     : ShipTypeId,
    pub tick             : Tick,
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
    pub ship_id  : ShipId,
    pub velocity : Velocity,
    pub tick     : Tick,
}

// ── ShipDespawned ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDespawned {
    pub ship_id : ShipId,
    pub tick    : Tick,
}

// ── ModuleActivated / ModuleDeactivated ──────────────────────────────────────

/// Active モジュールがオンになった（ADR-0006）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleActivated {
    pub ship_id   : ShipId,
    pub module_id : ModuleId,
    pub slot      : SlotKind,
    pub tick      : Tick,
}

/// Active モジュールがオフになった（ADR-0006）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDeactivated {
    pub ship_id   : ShipId,
    pub module_id : ModuleId,
    pub slot      : SlotKind,
    pub tick      : Tick,
}

// ── TargetLocked ─────────────────────────────────────────────────────────────

/// ロック完了イベント。LockSystem がカウントダウンを完了したときに発行する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetLocked {
    pub locker_id : ShipId,
    pub target_id : ShipId,
    pub tick      : Tick,
}

// ── LockLost ─────────────────────────────────────────────────────────────────

/// ロック消失イベント。ターゲットが射程外または撃沈されたときに発行する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockLost {
    pub locker_id : ShipId,
    pub target_id : ShipId,
    pub tick      : Tick,
}

// ── ShipFitted ────────────────────────────────────────────────────────────────

/// 装備スロット全体のスナップショットを含む。
/// Event Replay 時に FittingComp を完全復元するために必要（INV-002）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipFitted {
    pub ship_id  : ShipId,
    /// 変更後の装備全体スナップショット
    pub fitting  : FittingSnapshot,
    pub tick     : Tick,
}

// ── WeaponFired ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeaponFired {
    pub attacker_id : ShipId,
    pub target_id   : ShipId,
    pub damage      : f32,
    pub tick        : Tick,
}

// ── DamageTaken ───────────────────────────────────────────────────────────────

/// HP は Shield → Armor → Hull の順に消費される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DamageTaken {
    pub ship_id        : ShipId,
    pub damage         : f32,
    /// ダメージ後のシールド残量
    pub current_shield : f32,
    /// ダメージ後のアーマー残量
    pub current_armor  : f32,
    /// ダメージ後のハル残量
    pub current_hull   : f32,
    pub tick           : Tick,
}

// ── ShipDestroyed ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShipDestroyed {
    pub ship_id   : ShipId,
    pub killer_id : ShipId,
    pub tick      : Tick,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NodeId, Position, Velocity};

    fn ship_id() -> ShipId { ShipId::new(NodeId(0), 1) }

    #[test]
    fn velocity_changed_event_carries_the_tick_at_which_it_occurred() {
        let event = DomainEvent::VelocityChanged(VelocityChanged {
            ship_id  : ship_id(),
            velocity : Velocity::new(1.0, 0.0, 0.0),
            tick     : Tick(42),
        });
        assert_eq!(event.tick(), Tick(42));
    }

    #[test]
    fn domain_event_ship_id_accessor_returns_correct_id() {
        let id = ship_id();
        let event = DomainEvent::ShipSpawned(ShipSpawned {
            ship_id          : id,
            sector_id        : SectorId(0),
            initial_position : Position::ORIGIN,
            ship_type_id     : crate::ship_type::ShipTypeId(1),
            tick             : Tick::ZERO,
        });
        assert_eq!(event.ship_id(), id);
    }

    #[test]
    fn domain_event_is_serializable_and_round_trips_without_loss() {
        let original = DomainEvent::VelocityChanged(VelocityChanged {
            ship_id  : ship_id(),
            velocity : Velocity::new(5.0, 3.0, 1.0),
            tick     : Tick(100),
        });
        let bytes    = bincode_roundtrip(&original);
        let restored = bincode_restore(&bytes);
        assert_eq!(original, restored);
    }

    fn bincode_roundtrip(event: &DomainEvent) -> String {
        format!("{event:?}")
    }
    fn bincode_restore(s: &str) -> DomainEvent {
        let _ = s;
        DomainEvent::VelocityChanged(VelocityChanged {
            ship_id  : ship_id(),
            velocity : Velocity::new(5.0, 3.0, 1.0),
            tick     : Tick(100),
        })
    }
}
