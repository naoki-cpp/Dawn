use dawn_core::DomainEvent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{PosWire, VelWire};

/// Every message the server sends to a client over the WebSocket connection,
/// wrapped by `ServerMessage::Event` and postcard-encoded (ADR-0042).
///
/// This enum is the schema-of-record for the server -> client half of the
/// wire protocol: [`event_wire_json_schema()`] renders it to a JSON Schema
/// document that `docs/architecture/wire-protocol.md` is generated from.
/// Adding, removing, or renaming a field here changes the wire format for
/// every client (Godot today; any future client written against
/// `docs/architecture/wire-protocol.md`).
///
/// Externally tagged (serde's default enum representation), not
/// `#[serde(tag = "type")]` -- `postcard` cannot deserialize an internally
/// tagged enum (no `deserialize_any`). `dawn-client-gdext`'s
/// `ServerMessageDecoder` converts the externally tagged shape back into a
/// `{"type": ..., ...}` Dictionary so existing GDScript consumers (written
/// against the old JSON shape) don't need to change.
///
/// `Deserialize` (ADR-0042) exists so `dawn-client-gdext` can decode a
/// `ServerMessage` it receives; the server itself only ever serializes this
/// type, never parses it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum EventWire {
    ShipSpawned {
        ship_id: u64,
        position: PosWire,
        tick: u64,
    },
    VelocityChanged {
        ship_id: u64,
        velocity: VelWire,
        tick: u64,
    },
    ShipDespawned {
        ship_id: u64,
        tick: u64,
    },
    ShipDocked {
        ship_id: u64,
        station_id: u32,
        tick: u64,
    },
    ShipUndocked {
        ship_id: u64,
        station_id: u32,
        tick: u64,
    },
    /// A station-inventory Packaged Ship item became a new live docked ship,
    /// owned by the caller (ADR-0034 9B, ADR-0037). `active_ship` is
    /// unchanged -- the client must send `SelectActiveShipCommand` to fly it.
    ShipAssembled {
        ship_id: u64,
        station_id: u32,
        ship_type_id: u32,
        tick: u64,
    },
    DamageTaken {
        ship_id: u64,
        damage: f32,
        current_shield: f32,
        current_armor: f32,
        current_hull: f32,
        tick: u64,
    },
    RepairApplied {
        ship_id: u64,
        amount: f32,
        layer: String,
        current_shield: f32,
        current_armor: f32,
        current_hull: f32,
        tick: u64,
    },
    ShipDestroyed {
        ship_id: u64,
        killer_id: u64,
        tick: u64,
    },
    TargetLocked {
        locker_id: u64,
        target_id: u64,
        tick: u64,
    },
    LockLost {
        locker_id: u64,
        target_id: u64,
        tick: u64,
    },
    ModuleActivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        /// Target of a targeted module (Weapon/Tackle), per ADR-0035.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        target_ship_id: Option<u64>,
        tick: u64,
    },
    ModuleDeactivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        /// Why the system forced this off ("cap" | "range"); omitted for a
        /// player-issued deactivation (ADR-0035).
        #[serde(skip_serializing_if = "Option::is_none", default)]
        reason: Option<String>,
        tick: u64,
    },
    JumpGateUsed {
        ship_id: u64,
        gate_id: u32,
        from_sector: u8,
        to_sector: u8,
        entry_pos: PosWire,
        tick: u64,
    },
    StarSystemChanged {
        ship_id: u64,
        from_system: u32,
        to_system: u32,
        tick: u64,
    },
}

/// Render the server -> client wire schema (see [`EventWire`]) as a JSON
/// Schema document.
pub fn event_wire_json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(EventWire)
}

/// Convert a [`DomainEvent`] to its [`EventWire`] wire representation.
/// Returns `None` for internal events that are not forwarded to clients
/// (transit internals, combat bookkeeping).
pub fn domain_event_to_event_wire(event: &DomainEvent) -> Option<EventWire> {
    Some(match event {
        DomainEvent::ShipSpawned(e) => EventWire::ShipSpawned {
            ship_id: e.ship_id.raw(),
            position: e.initial_position.into(),
            tick: e.tick.value(),
        },
        DomainEvent::VelocityChanged(e) => EventWire::VelocityChanged {
            ship_id: e.ship_id.raw(),
            velocity: e.velocity.into(),
            tick: e.tick.value(),
        },
        DomainEvent::ShipDespawned(e) => EventWire::ShipDespawned {
            ship_id: e.ship_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::ShipDocked(e) => EventWire::ShipDocked {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::ShipUndocked(e) => EventWire::ShipUndocked {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::ShipAssembled(e) => EventWire::ShipAssembled {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            ship_type_id: e.ship_type_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::DamageTaken(e) => EventWire::DamageTaken {
            ship_id: e.ship_id.raw(),
            damage: e.damage,
            current_shield: e.current_shield,
            current_armor: e.current_armor,
            current_hull: e.current_hull,
            tick: e.tick.value(),
        },
        DomainEvent::RepairApplied(e) => EventWire::RepairApplied {
            ship_id: e.ship_id.raw(),
            amount: e.amount,
            layer: format!("{:?}", e.layer),
            current_shield: e.current_shield,
            current_armor: e.current_armor,
            current_hull: e.current_hull,
            tick: e.tick.value(),
        },
        DomainEvent::ShipDestroyed(e) => EventWire::ShipDestroyed {
            ship_id: e.ship_id.raw(),
            killer_id: e.killer_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::TargetLocked(e) => EventWire::TargetLocked {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::LockLost(e) => EventWire::LockLost {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::ModuleActivated(e) => EventWire::ModuleActivated {
            ship_id: e.ship_id.raw(),
            module_id: e.module_id.0,
            slot: format!("{:?}", e.slot),
            target_ship_id: e.target_ship_id.map(|t| t.raw()),
            tick: e.tick.value(),
        },
        DomainEvent::ModuleDeactivated(e) => EventWire::ModuleDeactivated {
            ship_id: e.ship_id.raw(),
            module_id: e.module_id.0,
            slot: format!("{:?}", e.slot),
            reason: e.forced_reason.map(|r| match r {
                dawn_core::events::ModuleDeactivationReason::CapacitorExhausted => {
                    "cap".to_string()
                }
                dawn_core::events::ModuleDeactivationReason::OutOfRange => "range".to_string(),
            }),
            tick: e.tick.value(),
        },
        DomainEvent::JumpGateUsed(e) => EventWire::JumpGateUsed {
            ship_id: e.ship_id.raw(),
            gate_id: e.gate_id.0,
            from_sector: e.from_sector.0,
            to_sector: e.to_sector.0,
            entry_pos: e.entry_pos.into(),
            tick: e.tick.value(),
        },
        DomainEvent::StarSystemChanged(e) => EventWire::StarSystemChanged {
            ship_id: e.ship_id.raw(),
            from_system: e.from_system.0,
            to_system: e.to_system.0,
            tick: e.tick.value(),
        },
        DomainEvent::ShipFitted(_) => return None,
        DomainEvent::WeaponFired(_) => return None,
        DomainEvent::TackleApplied(_) => return None,
        DomainEvent::TackleReleased(_) => return None,
        DomainEvent::SectorTransitRequested(_) => return None,
        DomainEvent::SectorTransitCompleted(_) => return None,
        DomainEvent::SectorTransitAborted(_) => return None,
        DomainEvent::AnchorRebased(_) => return None,
        DomainEvent::PackagedShipBuilt(_) => return None,
        DomainEvent::ShipDisassembled(_) => return None,
    })
}
