use dawn_core::{DomainEvent, PlayerId, ShipId};
use schemars::JsonSchema;
use serde::Serialize;

use super::{PosJson, VelJson};

/// Every message the server sends to a client over the WebSocket connection.
/// Serialized as a single JSON line tagged by `"type"`.
///
/// This enum is the schema-of-record for the server -> client half of the
/// wire protocol: [`event_json_schema()`] renders it to a JSON Schema document that
/// `docs/architecture/wire-protocol.md` is generated from. Adding, removing,
/// or renaming a field here changes the wire format for every client
/// (Godot today; any future client written against
/// `docs/architecture/wire-protocol.md`).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "type")]
pub enum EventJson {
    ShipSpawned {
        ship_id: u64,
        position: PosJson,
        tick: u64,
    },
    VelocityChanged {
        ship_id: u64,
        velocity: VelJson,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        target_ship_id: Option<u64>,
        tick: u64,
    },
    ModuleDeactivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        /// Why the system forced this off ("cap" | "range"); omitted for a
        /// player-issued deactivation (ADR-0035).
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        tick: u64,
    },
    JumpGateUsed {
        ship_id: u64,
        gate_id: u32,
        from_sector: u8,
        to_sector: u8,
        entry_pos: PosJson,
        tick: u64,
    },
    StarSystemChanged {
        ship_id: u64,
        from_system: u32,
        to_system: u32,
        tick: u64,
    },
    Redirect {
        ws_addr: String,
        player_id: u64,
        ship_id: u64,
    },
}

/// Render the server -> client wire schema (see [`EventJson`]) as a JSON
/// Schema document.
pub fn event_json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(EventJson)
}

/// Serialize a [`DomainEvent`] to the JSON line the Godot client expects.
/// Returns `None` for internal events that are not forwarded to clients
/// (transit internals, combat bookkeeping).
pub fn domain_event_to_json(event: &DomainEvent) -> Option<String> {
    let j = match event {
        DomainEvent::ShipSpawned(e) => EventJson::ShipSpawned {
            ship_id: e.ship_id.raw(),
            position: e.initial_position.into(),
            tick: e.tick.value(),
        },
        DomainEvent::VelocityChanged(e) => EventJson::VelocityChanged {
            ship_id: e.ship_id.raw(),
            velocity: e.velocity.into(),
            tick: e.tick.value(),
        },
        DomainEvent::ShipDespawned(e) => EventJson::ShipDespawned {
            ship_id: e.ship_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::ShipDocked(e) => EventJson::ShipDocked {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::ShipUndocked(e) => EventJson::ShipUndocked {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::ShipAssembled(e) => EventJson::ShipAssembled {
            ship_id: e.ship_id.raw(),
            station_id: e.station_id.0,
            ship_type_id: e.ship_type_id.0,
            tick: e.tick.value(),
        },
        DomainEvent::DamageTaken(e) => EventJson::DamageTaken {
            ship_id: e.ship_id.raw(),
            damage: e.damage,
            current_shield: e.current_shield,
            current_armor: e.current_armor,
            current_hull: e.current_hull,
            tick: e.tick.value(),
        },
        DomainEvent::RepairApplied(e) => EventJson::RepairApplied {
            ship_id: e.ship_id.raw(),
            amount: e.amount,
            layer: format!("{:?}", e.layer),
            current_shield: e.current_shield,
            current_armor: e.current_armor,
            current_hull: e.current_hull,
            tick: e.tick.value(),
        },
        DomainEvent::ShipDestroyed(e) => EventJson::ShipDestroyed {
            ship_id: e.ship_id.raw(),
            killer_id: e.killer_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::TargetLocked(e) => EventJson::TargetLocked {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::LockLost(e) => EventJson::LockLost {
            locker_id: e.locker_id.raw(),
            target_id: e.target_id.raw(),
            tick: e.tick.value(),
        },
        DomainEvent::ModuleActivated(e) => EventJson::ModuleActivated {
            ship_id: e.ship_id.raw(),
            module_id: e.module_id.0,
            slot: format!("{:?}", e.slot),
            target_ship_id: e.target_ship_id.map(|t| t.raw()),
            tick: e.tick.value(),
        },
        DomainEvent::ModuleDeactivated(e) => EventJson::ModuleDeactivated {
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
        DomainEvent::JumpGateUsed(e) => EventJson::JumpGateUsed {
            ship_id: e.ship_id.raw(),
            gate_id: e.gate_id.0,
            from_sector: e.from_sector.0,
            to_sector: e.to_sector.0,
            entry_pos: e.entry_pos.into(),
            tick: e.tick.value(),
        },
        DomainEvent::StarSystemChanged(e) => EventJson::StarSystemChanged {
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
    };
    serde_json::to_string(&j).ok()
}

/// Build a `{"type":"Redirect","ws_addr":"..."}` JSON line for a client whose
/// ship just jumped to a sector owned by a different physical node.
pub fn redirect_json(
    ws_addr: std::net::SocketAddr,
    player_id: PlayerId,
    ship_id: ShipId,
) -> String {
    let j = EventJson::Redirect {
        ws_addr: ws_addr.to_string(),
        player_id: player_id.raw(),
        ship_id: ship_id.raw(),
    };
    serde_json::to_string(&j).unwrap_or_default()
}
