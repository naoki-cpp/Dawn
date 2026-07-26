//! Decoders for the pre-ADR-0044 event payload shape.
//!
//! The event log has no self-describing schema header. Keep these types
//! independent from the current spatial types so old f32 records can be
//! widened to the current f64 domain model during replay.

use dawn_core::events::{
    AnchorRebased, DamageTaken, DomainEvent, JumpGateUsed, LockLost, ModuleActivated,
    ModuleDeactivated, PackagedShipBuilt, RepairApplied, SectorTransitAborted,
    SectorTransitCompleted, SectorTransitRequested, ShipAssembled, ShipDespawned, ShipDestroyed,
    ShipDisassembled, ShipDocked, ShipFitted, ShipSpawned, ShipUndocked, StarSystemChanged,
    TackleApplied, TackleReleased, TargetLocked, VelocityChanged, WeaponFired,
};
use dawn_core::{Position, Velocity};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct LegacyPositionF32 {
    x: f32,
    y: f32,
    z: f32,
}

impl From<LegacyPositionF32> for Position {
    fn from(value: LegacyPositionF32) -> Self {
        Self::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyVelocityF32 {
    dx: f32,
    dy: f32,
    dz: f32,
}

impl From<LegacyVelocityF32> for Velocity {
    fn from(value: LegacyVelocityF32) -> Self {
        Self::new(
            f64::from(value.dx),
            f64::from(value.dy),
            f64::from(value.dz),
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyVelocityChanged {
    ship_id: dawn_core::ShipId,
    velocity: LegacyVelocityF32,
    tick: dawn_core::Tick,
}

impl From<LegacyVelocityChanged> for VelocityChanged {
    fn from(value: LegacyVelocityChanged) -> Self {
        Self {
            ship_id: value.ship_id,
            velocity: value.velocity.into(),
            tick: value.tick,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacyAnchorRebased {
    ship_id: dawn_core::ShipId,
    anchor: dawn_core::AnchorId,
    offset: LegacyPositionF32,
    tick: dawn_core::Tick,
}

impl From<LegacyAnchorRebased> for AnchorRebased {
    fn from(value: LegacyAnchorRebased) -> Self {
        Self {
            ship_id: value.ship_id,
            anchor: value.anchor,
            offset: value.offset.into(),
            tick: value.tick,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacySectorTransitCompleted {
    ship_id: dawn_core::ShipId,
    from: dawn_core::SectorId,
    to: dawn_core::SectorId,
    entry_pos: dawn_core::AbsolutePosition,
    velocity: LegacyVelocityF32,
    tick: dawn_core::Tick,
}

impl From<LegacySectorTransitCompleted> for SectorTransitCompleted {
    fn from(value: LegacySectorTransitCompleted) -> Self {
        Self {
            ship_id: value.ship_id,
            from: value.from,
            to: value.to,
            entry_pos: value.entry_pos,
            velocity: value.velocity.into(),
            tick: value.tick,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
enum LegacyDomainEvent {
    ShipSpawned(ShipSpawned),
    VelocityChanged(LegacyVelocityChanged),
    ShipDespawned(ShipDespawned),
    ShipFitted(ShipFitted),
    ModuleActivated(ModuleActivated),
    ModuleDeactivated(ModuleDeactivated),
    TargetLocked(TargetLocked),
    LockLost(LockLost),
    WeaponFired(WeaponFired),
    DamageTaken(DamageTaken),
    RepairApplied(RepairApplied),
    ShipDestroyed(ShipDestroyed),
    SectorTransitRequested(SectorTransitRequested),
    SectorTransitCompleted(LegacySectorTransitCompleted),
    SectorTransitAborted(SectorTransitAborted),
    JumpGateUsed(JumpGateUsed),
    StarSystemChanged(StarSystemChanged),
    TackleApplied(TackleApplied),
    TackleReleased(TackleReleased),
    AnchorRebased(LegacyAnchorRebased),
    ShipDocked(ShipDocked),
    ShipUndocked(ShipUndocked),
    PackagedShipBuilt(PackagedShipBuilt),
    ShipDisassembled(ShipDisassembled),
    ShipAssembled(ShipAssembled),
}

impl From<LegacyDomainEvent> for DomainEvent {
    fn from(value: LegacyDomainEvent) -> Self {
        match value {
            LegacyDomainEvent::ShipSpawned(event) => DomainEvent::ShipSpawned(event),
            LegacyDomainEvent::VelocityChanged(event) => DomainEvent::VelocityChanged(event.into()),
            LegacyDomainEvent::ShipDespawned(event) => DomainEvent::ShipDespawned(event),
            LegacyDomainEvent::ShipFitted(event) => DomainEvent::ShipFitted(event),
            LegacyDomainEvent::ModuleActivated(event) => DomainEvent::ModuleActivated(event),
            LegacyDomainEvent::ModuleDeactivated(event) => DomainEvent::ModuleDeactivated(event),
            LegacyDomainEvent::TargetLocked(event) => DomainEvent::TargetLocked(event),
            LegacyDomainEvent::LockLost(event) => DomainEvent::LockLost(event),
            LegacyDomainEvent::WeaponFired(event) => DomainEvent::WeaponFired(event),
            LegacyDomainEvent::DamageTaken(event) => DomainEvent::DamageTaken(event),
            LegacyDomainEvent::RepairApplied(event) => DomainEvent::RepairApplied(event),
            LegacyDomainEvent::ShipDestroyed(event) => DomainEvent::ShipDestroyed(event),
            LegacyDomainEvent::SectorTransitRequested(event) => {
                DomainEvent::SectorTransitRequested(event)
            }
            LegacyDomainEvent::SectorTransitCompleted(event) => {
                DomainEvent::SectorTransitCompleted(event.into())
            }
            LegacyDomainEvent::SectorTransitAborted(event) => {
                DomainEvent::SectorTransitAborted(event)
            }
            LegacyDomainEvent::JumpGateUsed(event) => DomainEvent::JumpGateUsed(event),
            LegacyDomainEvent::StarSystemChanged(event) => DomainEvent::StarSystemChanged(event),
            LegacyDomainEvent::TackleApplied(event) => DomainEvent::TackleApplied(event),
            LegacyDomainEvent::TackleReleased(event) => DomainEvent::TackleReleased(event),
            LegacyDomainEvent::AnchorRebased(event) => DomainEvent::AnchorRebased(event.into()),
            LegacyDomainEvent::ShipDocked(event) => DomainEvent::ShipDocked(event),
            LegacyDomainEvent::ShipUndocked(event) => DomainEvent::ShipUndocked(event),
            LegacyDomainEvent::PackagedShipBuilt(event) => DomainEvent::PackagedShipBuilt(event),
            LegacyDomainEvent::ShipDisassembled(event) => DomainEvent::ShipDisassembled(event),
            LegacyDomainEvent::ShipAssembled(event) => DomainEvent::ShipAssembled(event),
        }
    }
}

pub(crate) fn decode_event(bytes: &[u8]) -> Result<DomainEvent, postcard::Error> {
    postcard::from_bytes::<DomainEvent>(bytes)
        .or_else(|_| postcard::from_bytes::<LegacyDomainEvent>(bytes).map(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::events::{AnchorRebased, VelocityChanged};

    #[test]
    fn old_velocity_event_is_upcast_to_f64() {
        let bytes =
            postcard::to_stdvec(&LegacyDomainEvent::VelocityChanged(LegacyVelocityChanged {
                ship_id: dawn_core::ShipId::new(dawn_core::NodeId(2), 7),
                velocity: LegacyVelocityF32 {
                    dx: 1.25,
                    dy: -2.5,
                    dz: 3.75,
                },
                tick: dawn_core::Tick(11),
            }))
            .unwrap();

        let event = decode_event(&bytes).unwrap();
        assert_eq!(
            event,
            DomainEvent::VelocityChanged(VelocityChanged {
                ship_id: dawn_core::ShipId::new(dawn_core::NodeId(2), 7),
                velocity: Velocity::new(1.25, -2.5, 3.75),
                tick: dawn_core::Tick(11),
            })
        );
    }

    #[test]
    fn old_anchor_rebase_event_is_upcast_to_f64() {
        let event = LegacyDomainEvent::AnchorRebased(LegacyAnchorRebased {
            ship_id: dawn_core::ShipId::new(dawn_core::NodeId(1), 4),
            anchor: dawn_core::AnchorId(9),
            offset: LegacyPositionF32 {
                x: 0.125,
                y: -0.25,
                z: 0.5,
            },
            tick: dawn_core::Tick(3),
        });
        let bytes = postcard::to_stdvec(&event).unwrap();

        assert_eq!(
            decode_event(&bytes).unwrap(),
            DomainEvent::AnchorRebased(AnchorRebased {
                ship_id: dawn_core::ShipId::new(dawn_core::NodeId(1), 4),
                anchor: dawn_core::AnchorId(9),
                offset: Position::new(0.125, -0.25, 0.5),
                tick: dawn_core::Tick(3),
            })
        );
    }

    #[test]
    fn current_position_event_still_uses_current_decoder() {
        let event = DomainEvent::AnchorRebased(AnchorRebased {
            ship_id: dawn_core::ShipId::new(dawn_core::NodeId(1), 4),
            anchor: dawn_core::AnchorId(9),
            offset: Position::new(0.125, -0.25, 0.5),
            tick: dawn_core::Tick(3),
        });
        let bytes = postcard::to_stdvec(&event).unwrap();

        assert_eq!(decode_event(&bytes).unwrap(), event);
    }
}
