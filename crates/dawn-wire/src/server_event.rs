use dawn_core::DomainEvent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AbsPosWire, VelWire};

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
/// tagged enum (no `deserialize_any`). `dawn-client-gdext` decodes this
/// enum into a typed client outcome and performs all variant dispatch in
/// Rust; GDScript never reconstructs this enum from a string tag.
///
/// `Deserialize` (ADR-0042) exists so `dawn-client-gdext` can decode a
/// `ServerMessage` it receives; the server itself only ever serializes this
/// type, never parses it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum EventWire {
    ShipSpawned {
        ship_id: u64,
        position: AbsPosWire,
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
        #[serde(default)]
        target_ship_id: Option<u64>,
        tick: u64,
    },
    ModuleDeactivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        /// Why the system forced this off ("cap" | "range"); `None` for a
        /// player-issued deactivation (ADR-0035).
        #[serde(default)]
        reason: Option<String>,
        tick: u64,
    },
    JumpGateUsed {
        ship_id: u64,
        gate_id: u32,
        from_sector: u8,
        to_sector: u8,
        entry_pos: AbsPosWire,
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
pub fn event_wire_json_schema() -> schemars::Schema {
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
        DomainEvent::ClientAdmissionIdentityReserved(_) => return None,
        DomainEvent::ClientAdmissionCommitted(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{EntityId, JumpGateId, ModuleId, NodeId, ShipId, SlotKind};

    fn ship_id(n: u64) -> ShipId {
        ShipId(EntityId::new(NodeId(0), n))
    }

    #[test]
    fn ship_docked_event_is_serialized_for_clients() {
        let wire =
            domain_event_to_event_wire(&DomainEvent::ShipDocked(dawn_core::events::ShipDocked {
                ship_id: ship_id(42),
                station_id: dawn_core::StationId(3),
                tick: dawn_core::Tick(9),
            }))
            .expect("ShipDocked should be forwarded");
        assert_eq!(
            wire,
            EventWire::ShipDocked {
                ship_id: ship_id(42).raw(),
                station_id: 3,
                tick: 9,
            }
        );
    }

    #[test]
    fn ship_assembled_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::ShipAssembled(
            dawn_core::events::ShipAssembled {
                ship_id: ship_id(99),
                player_id: dawn_core::PlayerId(1),
                station_id: dawn_core::StationId(3),
                ship_type_id: dawn_core::ShipTypeId(1),
                tick: dawn_core::Tick(9),
            },
        ))
        .expect("ShipAssembled should be forwarded");
        assert_eq!(
            wire,
            EventWire::ShipAssembled {
                ship_id: ship_id(99).raw(),
                station_id: 3,
                ship_type_id: 1,
                tick: 9,
            }
        );
    }

    #[test]
    fn ship_spawned_event_is_serialized_for_clients() {
        let wire =
            domain_event_to_event_wire(&DomainEvent::ShipSpawned(dawn_core::events::ShipSpawned {
                ship_id: ship_id(1),
                sector_id: dawn_core::SectorId(0),
                initial_position: dawn_core::AbsolutePosition::new(1.0, 2.0, 3.0),
                ship_type_id: dawn_core::ShipTypeId(7),
                tick: dawn_core::Tick(1),
            }))
            .expect("ShipSpawned should be forwarded");
        assert_eq!(
            wire,
            EventWire::ShipSpawned {
                ship_id: ship_id(1).raw(),
                position: AbsPosWire {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                tick: 1,
            }
        );
    }

    #[test]
    fn velocity_changed_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::VelocityChanged(
            dawn_core::events::VelocityChanged {
                ship_id: ship_id(1),
                velocity: dawn_core::Velocity {
                    dx: 1.0,
                    dy: 0.0,
                    dz: -2.0,
                },
                tick: dawn_core::Tick(2),
            },
        ))
        .expect("VelocityChanged should be forwarded");
        assert_eq!(
            wire,
            EventWire::VelocityChanged {
                ship_id: ship_id(1).raw(),
                velocity: VelWire {
                    dx: 1.0,
                    dy: 0.0,
                    dz: -2.0,
                },
                tick: 2,
            }
        );
    }

    #[test]
    fn ship_despawned_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::ShipDespawned(
            dawn_core::events::ShipDespawned {
                ship_id: ship_id(5),
                tick: dawn_core::Tick(3),
            },
        ))
        .expect("ShipDespawned should be forwarded");
        assert_eq!(
            wire,
            EventWire::ShipDespawned {
                ship_id: ship_id(5).raw(),
                tick: 3,
            }
        );
    }

    #[test]
    fn ship_undocked_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::ShipUndocked(
            dawn_core::events::ShipUndocked {
                ship_id: ship_id(5),
                station_id: dawn_core::StationId(2),
                tick: dawn_core::Tick(4),
            },
        ))
        .expect("ShipUndocked should be forwarded");
        assert_eq!(
            wire,
            EventWire::ShipUndocked {
                ship_id: ship_id(5).raw(),
                station_id: 2,
                tick: 4,
            }
        );
    }

    #[test]
    fn damage_taken_event_is_serialized_for_clients() {
        let wire =
            domain_event_to_event_wire(&DomainEvent::DamageTaken(dawn_core::events::DamageTaken {
                ship_id: ship_id(1),
                damage: 25.0,
                current_shield: 10.0,
                current_armor: 20.0,
                current_hull: 30.0,
                tick: dawn_core::Tick(5),
            }))
            .expect("DamageTaken should be forwarded");
        assert_eq!(
            wire,
            EventWire::DamageTaken {
                ship_id: ship_id(1).raw(),
                damage: 25.0,
                current_shield: 10.0,
                current_armor: 20.0,
                current_hull: 30.0,
                tick: 5,
            }
        );
    }

    #[test]
    fn repair_applied_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::RepairApplied(
            dawn_core::events::RepairApplied {
                ship_id: ship_id(1),
                amount: 15.0,
                layer: dawn_core::events::RepairLayer::Armor,
                current_shield: 10.0,
                current_armor: 25.0,
                current_hull: 30.0,
                tick: dawn_core::Tick(6),
            },
        ))
        .expect("RepairApplied should be forwarded");
        assert_eq!(
            wire,
            EventWire::RepairApplied {
                ship_id: ship_id(1).raw(),
                amount: 15.0,
                layer: "Armor".to_string(),
                current_shield: 10.0,
                current_armor: 25.0,
                current_hull: 30.0,
                tick: 6,
            }
        );
    }

    #[test]
    fn ship_destroyed_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::ShipDestroyed(
            dawn_core::events::ShipDestroyed {
                ship_id: ship_id(1),
                killer_id: ship_id(2),
                tick: dawn_core::Tick(7),
            },
        ))
        .expect("ShipDestroyed should be forwarded");
        assert_eq!(
            wire,
            EventWire::ShipDestroyed {
                ship_id: ship_id(1).raw(),
                killer_id: ship_id(2).raw(),
                tick: 7,
            }
        );
    }

    #[test]
    fn target_locked_and_lock_lost_events_are_serialized_for_clients() {
        let locked = domain_event_to_event_wire(&DomainEvent::TargetLocked(
            dawn_core::events::TargetLocked {
                locker_id: ship_id(1),
                target_id: ship_id(2),
                tick: dawn_core::Tick(8),
            },
        ))
        .expect("TargetLocked should be forwarded");
        assert_eq!(
            locked,
            EventWire::TargetLocked {
                locker_id: ship_id(1).raw(),
                target_id: ship_id(2).raw(),
                tick: 8,
            }
        );

        let lost =
            domain_event_to_event_wire(&DomainEvent::LockLost(dawn_core::events::LockLost {
                locker_id: ship_id(1),
                target_id: ship_id(2),
                tick: dawn_core::Tick(9),
            }))
            .expect("LockLost should be forwarded");
        assert_eq!(
            lost,
            EventWire::LockLost {
                locker_id: ship_id(1).raw(),
                target_id: ship_id(2).raw(),
                tick: 9,
            }
        );
    }

    #[test]
    fn module_activated_event_with_a_target_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::ModuleActivated(
            dawn_core::events::ModuleActivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                target_ship_id: Some(ship_id(2)),
                tick: dawn_core::Tick(10),
            },
        ))
        .expect("ModuleActivated should be forwarded");
        assert_eq!(
            wire,
            EventWire::ModuleActivated {
                ship_id: ship_id(1).raw(),
                module_id: 3,
                slot: "High".to_string(),
                target_ship_id: Some(ship_id(2).raw()),
                tick: 10,
            }
        );
    }

    #[test]
    fn module_activated_event_without_a_target_has_none_target() {
        let wire = domain_event_to_event_wire(&DomainEvent::ModuleActivated(
            dawn_core::events::ModuleActivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                target_ship_id: None,
                tick: dawn_core::Tick(10),
            },
        ))
        .unwrap();
        match wire {
            EventWire::ModuleActivated { target_ship_id, .. } => {
                assert_eq!(target_ship_id, None)
            }
            other => panic!("expected ModuleActivated, got {other:?}"),
        }
    }

    #[test]
    fn module_deactivated_event_carries_the_forced_reason_when_present() {
        let cap = domain_event_to_event_wire(&DomainEvent::ModuleDeactivated(
            dawn_core::events::ModuleDeactivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                forced_reason: Some(
                    dawn_core::events::ModuleDeactivationReason::CapacitorExhausted,
                ),
                tick: dawn_core::Tick(11),
            },
        ))
        .unwrap();
        assert_eq!(
            cap,
            EventWire::ModuleDeactivated {
                ship_id: ship_id(1).raw(),
                module_id: 3,
                slot: "High".to_string(),
                reason: Some("cap".to_string()),
                tick: 11,
            }
        );

        let range = domain_event_to_event_wire(&DomainEvent::ModuleDeactivated(
            dawn_core::events::ModuleDeactivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                forced_reason: Some(dawn_core::events::ModuleDeactivationReason::OutOfRange),
                tick: dawn_core::Tick(11),
            },
        ))
        .unwrap();
        match range {
            EventWire::ModuleDeactivated { reason, .. } => {
                assert_eq!(reason, Some("range".to_string()))
            }
            other => panic!("expected ModuleDeactivated, got {other:?}"),
        }

        let player = domain_event_to_event_wire(&DomainEvent::ModuleDeactivated(
            dawn_core::events::ModuleDeactivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                forced_reason: None,
                tick: dawn_core::Tick(11),
            },
        ))
        .unwrap();
        match player {
            EventWire::ModuleDeactivated { reason, .. } => assert_eq!(reason, None),
            other => panic!("expected ModuleDeactivated, got {other:?}"),
        }
    }

    #[test]
    fn jump_gate_used_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::JumpGateUsed(
            dawn_core::events::JumpGateUsed {
                ship_id: ship_id(1),
                gate_id: JumpGateId(4),
                from_sector: dawn_core::SectorId(0),
                to_sector: dawn_core::SectorId(1),
                entry_pos: dawn_core::AbsolutePosition::new(5.0, 6.0, 7.0),
                tick: dawn_core::Tick(12),
            },
        ))
        .expect("JumpGateUsed should be forwarded");
        assert_eq!(
            wire,
            EventWire::JumpGateUsed {
                ship_id: ship_id(1).raw(),
                gate_id: 4,
                from_sector: 0,
                to_sector: 1,
                entry_pos: AbsPosWire {
                    x: 5.0,
                    y: 6.0,
                    z: 7.0,
                },
                tick: 12,
            }
        );
    }

    #[test]
    fn star_system_changed_event_is_serialized_for_clients() {
        let wire = domain_event_to_event_wire(&DomainEvent::StarSystemChanged(
            dawn_core::events::StarSystemChanged {
                ship_id: ship_id(1),
                from_system: dawn_core::StarSystemId(0),
                to_system: dawn_core::StarSystemId(2),
                tick: dawn_core::Tick(13),
            },
        ))
        .expect("StarSystemChanged should be forwarded");
        assert_eq!(
            wire,
            EventWire::StarSystemChanged {
                ship_id: ship_id(1).raw(),
                from_system: 0,
                to_system: 2,
                tick: 13,
            }
        );
    }

    #[test]
    fn internal_events_are_never_forwarded_to_clients() {
        let tick = dawn_core::Tick(1);
        let not_forwarded: Vec<DomainEvent> = vec![
            DomainEvent::ShipFitted(dawn_core::events::ShipFitted {
                ship_id: ship_id(1),
                fitting: dawn_core::fitting::FittingSnapshot::empty(),
                inventory: vec![],
                tick,
            }),
            DomainEvent::WeaponFired(dawn_core::events::WeaponFired {
                attacker_id: ship_id(1),
                target_id: ship_id(2),
                damage: 10.0,
                tick,
            }),
            DomainEvent::TackleApplied(dawn_core::events::TackleApplied {
                ship_id: ship_id(1),
                by: ship_id(2),
                tick,
            }),
            DomainEvent::TackleReleased(dawn_core::events::TackleReleased {
                ship_id: ship_id(1),
                by: ship_id(2),
                tick,
            }),
            DomainEvent::SectorTransitRequested(dawn_core::events::SectorTransitRequested {
                ship_id: ship_id(1),
                from: dawn_core::SectorId(0),
                to: dawn_core::SectorId(1),
                request_tick: tick,
                gate_id: None,
                entry_pos: dawn_core::Position::ORIGIN,
                entry_pos_abs: dawn_core::AbsolutePosition::ORIGIN,
                tick,
            }),
            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                handoff: dawn_core::TransitHandoffState {
                    ship_id: ship_id(1),
                    owner_player_id: None,
                    ship_type_id: dawn_core::ShipTypeId(1),
                    velocity: dawn_core::Velocity::ZERO,
                    current_shield: 100.0,
                    current_armor: 100.0,
                    current_hull: 100.0,
                    is_destroyed: false,
                    capacitor: Some(50.0),
                    fitting: dawn_core::fitting::FittingSnapshot::empty(),
                    inventory: std::collections::BTreeMap::new(),
                },
                from: dawn_core::SectorId(0),
                to: dawn_core::SectorId(1),
                request_tick: dawn_core::Tick::ZERO,
                entry_pos: dawn_core::AbsolutePosition::ORIGIN,
                tick,
            }),
            DomainEvent::SectorTransitAborted(dawn_core::events::SectorTransitAborted {
                ship_id: ship_id(1),
                from: dawn_core::SectorId(0),
                to: dawn_core::SectorId(1),
                tick,
            }),
            DomainEvent::AnchorRebased(dawn_core::events::AnchorRebased {
                ship_id: ship_id(1),
                anchor: dawn_core::AnchorId(0),
                offset: dawn_core::Position::ORIGIN,
                tick,
            }),
            DomainEvent::PackagedShipBuilt(dawn_core::events::PackagedShipBuilt {
                ship_id: ship_id(1),
                player_id: dawn_core::PlayerId(1),
                station_id: dawn_core::StationId(0),
                ship_type_id: dawn_core::ShipTypeId(1),
                scrap_cost: 1,
                tick,
            }),
            DomainEvent::ShipDisassembled(dawn_core::events::ShipDisassembled {
                ship_id: ship_id(1),
                player_id: dawn_core::PlayerId(1),
                station_id: dawn_core::StationId(0),
                ship_type_id: dawn_core::ShipTypeId(1),
                tick,
            }),
        ];
        for event in not_forwarded {
            assert!(
                domain_event_to_event_wire(&event).is_none(),
                "{event:?} must not be forwarded to clients"
            );
        }
    }
}
