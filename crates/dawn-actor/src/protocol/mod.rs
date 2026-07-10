//! Wire protocol translation between DomainEvents / ClientCommands and JSON.
//!
//! This module is the single place that knows both the Rust domain types and
//! the JSON keys the Godot client expects. Keeping it separate from
//! `ws_server.rs` means WebSocket transport logic never changes when the JSON
//! schema evolves, and vice versa. Both binaries (`dawn-simulation`,
//! `dawn-sector-node`) share this one definition (previously duplicated).
//!
//! # Responsibilities
//! - [`domain_event_to_json`]: DomainEvent -> newline-delimited JSON (server -> client).
//! - [`redirect_json`]: tell a client to reconnect to another node's WS (multi-node jump).
//! - [`parse_client_command`]: JSON line -> ClientCommand (client -> server).

mod client_command;
mod hello_resume;
mod server_event;

pub use client_command::{
    client_command_json_schema, parse_client_command, ClientCommandJson, PosJson, VelJson,
    WarpTargetJson,
};
pub use hello_resume::{parse_hello, HelloMessage, ResumeIdentity};
pub use server_event::{domain_event_to_json, event_json_schema, redirect_json, EventJson};

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{
        ApproachTarget, DomainEvent, EntityId, JumpGateId, ModuleId, NodeId, ShipId, SlotKind,
    };

    fn ship_id(n: u64) -> ShipId {
        ShipId(EntityId::new(NodeId(0), n))
    }

    #[test]
    fn lock_on_command_json_is_parsed_into_client_command_lock_on() {
        let line = r#"{"type":"LockOnCommand","target_id":7}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::LockOn(c) => {
                assert_eq!(c.target_id, ship_id(7));
            }
            other => panic!("expected LockOn, got {other:?}"),
        }
    }

    #[test]
    fn activate_module_command_json_is_parsed_with_and_without_a_target() {
        let line =
            r#"{"type":"ActivateModuleCommand","module_id":3,"slot":"High","target_ship_id":9}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Activate(c) => {
                assert_eq!(c.module_id, ModuleId(3));
                assert_eq!(c.slot, SlotKind::High);
                assert_eq!(c.target_ship_id, Some(ship_id(9)));
            }
            other => panic!("expected Activate, got {other:?}"),
        }

        let line_no_target = r#"{"type":"ActivateModuleCommand","module_id":3,"slot":"High"}"#;
        let cmd_no_target = parse_client_command(line_no_target).expect("must parse");
        match cmd_no_target {
            dawn_core::ClientCommand::Activate(c) => assert_eq!(c.target_ship_id, None),
            other => panic!("expected Activate, got {other:?}"),
        }
    }

    #[test]
    fn deactivate_module_command_json_is_parsed_into_client_command_deactivate() {
        let line = r#"{"type":"DeactivateModuleCommand","module_id":3,"slot":"Mid"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Deactivate(c) => {
                assert_eq!(c.module_id, ModuleId(3));
                assert_eq!(c.slot, SlotKind::Mid);
            }
            other => panic!("expected Deactivate, got {other:?}"),
        }
    }

    #[test]
    fn attack_command_json_is_parsed_into_client_command_attack() {
        let line = r#"{"type":"AttackCommand","attacker_id":1,"target_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Attack(c) => {
                assert_eq!(c.attacker_id, ship_id(1));
                assert_eq!(c.target_id, ship_id(2));
            }
            other => panic!("expected Attack, got {other:?}"),
        }
    }

    #[test]
    fn stop_command_json_is_parsed_into_client_command_stop() {
        let line = r#"{"type":"StopCommand"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        assert!(matches!(cmd, dawn_core::ClientCommand::Stop(_)));
    }

    #[test]
    fn undock_command_json_is_parsed_into_client_command_undock() {
        let line = r#"{"type":"UndockCommand"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        assert!(matches!(cmd, dawn_core::ClientCommand::Undock(_)));
    }

    #[test]
    fn build_packaged_ship_command_json_is_parsed_into_client_command_build_packaged_ship() {
        let line =
            r#"{"type":"BuildPackagedShipCommand","ship_id":1,"station_id":2,"ship_type_id":7}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::BuildPackagedShip(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.station_id, dawn_core::StationId(2));
                assert_eq!(c.ship_type_id, dawn_core::ShipTypeId(7));
            }
            other => panic!("expected BuildPackagedShip, got {other:?}"),
        }
    }

    #[test]
    fn select_active_ship_command_json_is_parsed_into_client_command_select_active_ship() {
        let line = r#"{"type":"SelectActiveShipCommand","ship_id":5}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::SelectActiveShip(c) => {
                assert_eq!(c.ship_id, ship_id(5));
            }
            other => panic!("expected SelectActiveShip, got {other:?}"),
        }
    }

    #[test]
    fn move_command_json_is_parsed_into_client_command_move() {
        let line = r#"{"type":"MoveCommand","target":{"x":10.0,"y":0.0,"z":-5.0}}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Move(c) => {
                assert!((c.target_position.x - 10.0).abs() < 1e-6);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    /// security-review.md SEC-5: a non-finite coordinate must be rejected at
    /// the wire boundary instead of flowing into position/velocity math.
    /// JSON has no `NaN`/`Infinity` literals, so the attack shape a real
    /// client can actually send is a magnitude that overflows `f32` on
    /// parse (`1e40` is valid JSON but exceeds `f32::MAX`, so serde_json
    /// hands back `f32::INFINITY`) -- literal `NaN`/`Infinity` tokens would
    /// just fail JSON parsing itself, which doesn't exercise `is_finite()`.
    #[test]
    fn move_command_json_with_an_overflowing_coordinate_fails_to_parse() {
        let line = r#"{"type":"MoveCommand","target":{"x":1e40,"y":0.0,"z":0.0}}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn orbit_command_json_with_an_overflowing_radius_fails_to_parse() {
        let line = r#"{"type":"OrbitCommand","gate_id":2,"radius":1e40}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn keep_at_range_command_json_with_an_overflowing_range_fails_to_parse() {
        let line = r#"{"type":"KeepAtRangeCommand","gate_id":2,"range":1e40}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn warp_command_json_is_parsed_into_client_command_warp() {
        let line = r#"{"type":"WarpCommand","gate_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }

        let line2 = r#"{"type":"WarpCommand","target":{"Gate":2}}"#;
        let cmd2 = parse_client_command(line2).expect("must parse");
        match cmd2 {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(c.target, dawn_core::WarpTarget::Gate(JumpGateId(2)));
            }
            other => panic!("expected Warp, got {other:?}"),
        }

        let line3 = r#"{"type":"WarpCommand","target":{"Body":1}}"#;
        let cmd3 = parse_client_command(line3).expect("must parse");
        match cmd3 {
            dawn_core::ClientCommand::Warp(c) => {
                assert_eq!(
                    c.target,
                    dawn_core::WarpTarget::Body(dawn_core::CelestialBodyId(1))
                );
            }
            other => panic!("expected Warp, got {other:?}"),
        }
    }

    #[test]
    fn dock_command_json_is_parsed_into_client_command_dock() {
        let line = r#"{"type":"DockCommand","station_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Dock(c) => {
                assert_eq!(c.station_id, dawn_core::StationId(2));
            }
            other => panic!("expected Dock, got {other:?}"),
        }
    }

    #[test]
    fn disassemble_ship_command_json_is_parsed_into_client_command_disassemble_ship() {
        let line = r#"{"type":"DisassembleShipCommand","ship_id":42,"station_id":2}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::DisassembleShip(c) => {
                assert_eq!(c.ship_id, ship_id(42));
                assert_eq!(c.station_id, dawn_core::StationId(2));
            }
            other => panic!("expected DisassembleShip, got {other:?}"),
        }
    }

    #[test]
    fn assemble_command_json_is_parsed_into_client_command_assemble() {
        let line = r#"{"type":"AssembleCommand","station_id":2,"ship_type_id":1}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Assemble(c) => {
                assert_eq!(c.station_id, dawn_core::StationId(2));
                assert_eq!(c.ship_type_id, dawn_core::ShipTypeId(1));
            }
            other => panic!("expected Assemble, got {other:?}"),
        }
    }

    #[test]
    fn disembark_command_json_is_parsed_into_client_command_disembark() {
        let line = r#"{"type":"DisembarkCommand"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        assert!(matches!(cmd, dawn_core::ClientCommand::Disembark(_)));
    }

    #[test]
    fn transfer_to_station_command_json_with_scrap_metal_is_parsed() {
        let line = r#"{"type":"TransferToStationCommand","ship_id":42,"station_id":2,"item_type":"ScrapMetal","module_id":0,"ship_type_id":0,"direction":"ToStation"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::TransferToStation(c) => {
                assert_eq!(c.ship_id, ship_id(42));
                assert_eq!(c.station_id, dawn_core::StationId(2));
                assert_eq!(c.item_id, dawn_core::ItemId::ScrapMetal);
                assert_eq!(c.direction, dawn_core::TransferDirection::ToStation);
            }
            other => panic!("expected TransferToStation, got {other:?}"),
        }
    }

    #[test]
    fn transfer_to_station_command_json_with_module_is_parsed() {
        let line = r#"{"type":"TransferToStationCommand","ship_id":42,"station_id":2,"item_type":"Module","module_id":7,"ship_type_id":0,"direction":"ToStation"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::TransferToStation(c) => {
                assert_eq!(c.item_id, dawn_core::ItemId::Module(ModuleId(7)));
            }
            other => panic!("expected TransferToStation, got {other:?}"),
        }
    }

    #[test]
    fn transfer_to_station_command_json_with_to_ship_direction_is_parsed() {
        let line = r#"{"type":"TransferToStationCommand","ship_id":42,"station_id":2,"item_type":"ScrapMetal","module_id":0,"ship_type_id":0,"direction":"ToShip"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::TransferToStation(c) => {
                assert_eq!(c.direction, dawn_core::TransferDirection::ToShip);
            }
            other => panic!("expected TransferToStation, got {other:?}"),
        }
    }

    #[test]
    fn transfer_to_station_command_json_with_unknown_item_type_fails_to_parse() {
        let line = r#"{"type":"TransferToStationCommand","ship_id":42,"station_id":2,"item_type":"Bogus","module_id":0,"ship_type_id":0,"direction":"ToStation"}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn transfer_to_station_command_json_with_unknown_direction_fails_to_parse() {
        let line = r#"{"type":"TransferToStationCommand","ship_id":42,"station_id":2,"item_type":"ScrapMetal","module_id":0,"ship_type_id":0,"direction":"Bogus"}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn reorder_fitted_module_command_json_is_parsed() {
        let line = r#"{"type":"ReorderFittedModuleCommand","ship_id":1,"slot":"Mid","from_index":0,"to_index":1}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::ReorderFittedModule(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.slot, dawn_core::SlotKind::Mid);
                assert_eq!(c.from_index, 0);
                assert_eq!(c.to_index, 1);
            }
            other => panic!("expected ReorderFittedModule, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_target_id_is_parsed_into_client_command_orbit() {
        let line = r#"{"type":"OrbitCommand","target_id":2,"radius":3000.0}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.radius, Some(3000.0));
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn orbit_command_json_with_gate_id_and_no_radius_is_parsed() {
        let line = r#"{"type":"OrbitCommand","gate_id":4}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Orbit(c) => {
                assert_eq!(c.target, ApproachTarget::Gate(JumpGateId(4)));
                assert_eq!(c.radius, None);
            }
            other => panic!("expected Orbit, got {other:?}"),
        }
    }

    #[test]
    fn keep_at_range_command_json_is_parsed_into_client_command_keep_at_range() {
        let line = r#"{"type":"KeepAtRangeCommand","target_id":2,"range":5000.0}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::KeepAtRange(c) => {
                assert_eq!(c.target, ApproachTarget::Ship(ship_id(2)));
                assert_eq!(c.range, Some(5000.0));
            }
            other => panic!("expected KeepAtRange, got {other:?}"),
        }
    }

    #[test]
    fn fit_module_command_json_is_parsed_into_client_command_fit() {
        let line = r#"{"type":"FitModuleCommand","ship_id":1,"module_id":2,"slot":"High"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Fit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.module_id, ModuleId(2));
                assert_eq!(c.slot, SlotKind::High);
            }
            other => panic!("expected Fit, got {other:?}"),
        }
    }

    #[test]
    fn unfit_module_command_json_is_parsed_into_client_command_unfit() {
        let line = r#"{"type":"UnfitModuleCommand","ship_id":1,"module_id":2,"slot":"Mid"}"#;
        let cmd = parse_client_command(line).expect("must parse");
        match cmd {
            dawn_core::ClientCommand::Unfit(c) => {
                assert_eq!(c.ship_id, ship_id(1));
                assert_eq!(c.module_id, ModuleId(2));
                assert_eq!(c.slot, SlotKind::Mid);
            }
            other => panic!("expected Unfit, got {other:?}"),
        }
    }

    #[test]
    fn unknown_command_type_returns_none() {
        let line = r#"{"type":"UnknownCommand","ship_id":1}"#;
        assert!(parse_client_command(line).is_none());
    }

    #[test]
    fn ship_docked_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ShipDocked(dawn_core::events::ShipDocked {
            ship_id: ship_id(42),
            station_id: dawn_core::StationId(3),
            tick: dawn_core::Tick(9),
        }))
        .expect("ShipDocked should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipDocked");
        assert_eq!(v["ship_id"], ship_id(42).raw());
        assert_eq!(v["station_id"], 3);
        assert_eq!(v["tick"], 9);
    }

    #[test]
    fn ship_assembled_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ShipAssembled(
            dawn_core::events::ShipAssembled {
                ship_id: ship_id(99),
                player_id: dawn_core::PlayerId(1),
                station_id: dawn_core::StationId(3),
                ship_type_id: dawn_core::ShipTypeId(1),
                tick: dawn_core::Tick(9),
            },
        ))
        .expect("ShipAssembled should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipAssembled");
        assert_eq!(v["ship_id"], ship_id(99).raw());
        assert_eq!(v["station_id"], 3);
        assert_eq!(v["ship_type_id"], 1);
        assert_eq!(v["tick"], 9);
    }

    #[test]
    fn ship_spawned_event_is_serialized_for_clients() {
        let json =
            domain_event_to_json(&DomainEvent::ShipSpawned(dawn_core::events::ShipSpawned {
                ship_id: ship_id(1),
                sector_id: dawn_core::SectorId(0),
                initial_position: dawn_core::Position::new(1.0, 2.0, 3.0),
                ship_type_id: dawn_core::ShipTypeId(7),
                tick: dawn_core::Tick(1),
            }))
            .expect("ShipSpawned should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipSpawned");
        assert_eq!(v["ship_id"], ship_id(1).raw());
        assert_eq!(v["position"]["x"], 1.0);
        assert_eq!(v["position"]["y"], 2.0);
        assert_eq!(v["position"]["z"], 3.0);
        assert_eq!(v["tick"], 1);
        assert!(v.get("ship_type_id").is_none());
    }

    #[test]
    fn velocity_changed_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::VelocityChanged(
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
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "VelocityChanged");
        assert_eq!(v["velocity"]["dx"], 1.0);
        assert_eq!(v["velocity"]["dz"], -2.0);
    }

    #[test]
    fn ship_despawned_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ShipDespawned(
            dawn_core::events::ShipDespawned {
                ship_id: ship_id(5),
                tick: dawn_core::Tick(3),
            },
        ))
        .expect("ShipDespawned should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipDespawned");
        assert_eq!(v["ship_id"], ship_id(5).raw());
    }

    #[test]
    fn ship_undocked_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ShipUndocked(
            dawn_core::events::ShipUndocked {
                ship_id: ship_id(5),
                station_id: dawn_core::StationId(2),
                tick: dawn_core::Tick(4),
            },
        ))
        .expect("ShipUndocked should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipUndocked");
        assert_eq!(v["station_id"], 2);
    }

    #[test]
    fn damage_taken_event_is_serialized_for_clients() {
        let json =
            domain_event_to_json(&DomainEvent::DamageTaken(dawn_core::events::DamageTaken {
                ship_id: ship_id(1),
                damage: 25.0,
                current_shield: 10.0,
                current_armor: 20.0,
                current_hull: 30.0,
                tick: dawn_core::Tick(5),
            }))
            .expect("DamageTaken should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "DamageTaken");
        assert_eq!(v["damage"], 25.0);
        assert_eq!(v["current_hull"], 30.0);
    }

    #[test]
    fn repair_applied_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::RepairApplied(
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
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "RepairApplied");
        assert_eq!(v["layer"], "Armor");
        assert_eq!(v["current_armor"], 25.0);
    }

    #[test]
    fn ship_destroyed_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ShipDestroyed(
            dawn_core::events::ShipDestroyed {
                ship_id: ship_id(1),
                killer_id: ship_id(2),
                tick: dawn_core::Tick(7),
            },
        ))
        .expect("ShipDestroyed should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ShipDestroyed");
        assert_eq!(v["killer_id"], ship_id(2).raw());
    }

    #[test]
    fn target_locked_and_lock_lost_events_are_serialized_for_clients() {
        let locked = domain_event_to_json(&DomainEvent::TargetLocked(
            dawn_core::events::TargetLocked {
                locker_id: ship_id(1),
                target_id: ship_id(2),
                tick: dawn_core::Tick(8),
            },
        ))
        .expect("TargetLocked should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&locked).unwrap();
        assert_eq!(v["type"], "TargetLocked");
        assert_eq!(v["locker_id"], ship_id(1).raw());
        assert_eq!(v["target_id"], ship_id(2).raw());

        let lost = domain_event_to_json(&DomainEvent::LockLost(dawn_core::events::LockLost {
            locker_id: ship_id(1),
            target_id: ship_id(2),
            tick: dawn_core::Tick(9),
        }))
        .expect("LockLost should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&lost).unwrap();
        assert_eq!(v["type"], "LockLost");
    }

    #[test]
    fn module_activated_event_with_a_target_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::ModuleActivated(
            dawn_core::events::ModuleActivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                target_ship_id: Some(ship_id(2)),
                tick: dawn_core::Tick(10),
            },
        ))
        .expect("ModuleActivated should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "ModuleActivated");
        assert_eq!(v["module_id"], 3);
        assert_eq!(v["slot"], "High");
        assert_eq!(v["target_ship_id"], ship_id(2).raw());
    }

    #[test]
    fn module_activated_event_without_a_target_omits_the_field() {
        let json = domain_event_to_json(&DomainEvent::ModuleActivated(
            dawn_core::events::ModuleActivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                target_ship_id: None,
                tick: dawn_core::Tick(10),
            },
        ))
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("target_ship_id").is_none());
    }

    #[test]
    fn module_deactivated_event_carries_the_forced_reason_when_present() {
        let json = domain_event_to_json(&DomainEvent::ModuleDeactivated(
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
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["reason"], "cap");

        let json_range = domain_event_to_json(&DomainEvent::ModuleDeactivated(
            dawn_core::events::ModuleDeactivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                forced_reason: Some(dawn_core::events::ModuleDeactivationReason::OutOfRange),
                tick: dawn_core::Tick(11),
            },
        ))
        .unwrap();
        let v_range: serde_json::Value = serde_json::from_str(&json_range).unwrap();
        assert_eq!(v_range["reason"], "range");

        let json_player = domain_event_to_json(&DomainEvent::ModuleDeactivated(
            dawn_core::events::ModuleDeactivated {
                ship_id: ship_id(1),
                module_id: ModuleId(3),
                slot: SlotKind::High,
                forced_reason: None,
                tick: dawn_core::Tick(11),
            },
        ))
        .unwrap();
        let v_player: serde_json::Value = serde_json::from_str(&json_player).unwrap();
        assert!(v_player.get("reason").is_none());
    }

    #[test]
    fn jump_gate_used_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::JumpGateUsed(
            dawn_core::events::JumpGateUsed {
                ship_id: ship_id(1),
                gate_id: JumpGateId(4),
                from_sector: dawn_core::SectorId(0),
                to_sector: dawn_core::SectorId(1),
                entry_pos: dawn_core::Position::new(5.0, 6.0, 7.0),
                tick: dawn_core::Tick(12),
            },
        ))
        .expect("JumpGateUsed should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "JumpGateUsed");
        assert_eq!(v["gate_id"], 4);
        assert_eq!(v["from_sector"], 0);
        assert_eq!(v["to_sector"], 1);
        assert_eq!(v["entry_pos"]["x"], 5.0);
    }

    #[test]
    fn star_system_changed_event_is_serialized_for_clients() {
        let json = domain_event_to_json(&DomainEvent::StarSystemChanged(
            dawn_core::events::StarSystemChanged {
                ship_id: ship_id(1),
                from_system: dawn_core::StarSystemId(0),
                to_system: dawn_core::StarSystemId(2),
                tick: dawn_core::Tick(13),
            },
        ))
        .expect("StarSystemChanged should be forwarded");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "StarSystemChanged");
        assert_eq!(v["from_system"], 0);
        assert_eq!(v["to_system"], 2);
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
                tick,
            }),
            DomainEvent::SectorTransitCompleted(dawn_core::events::SectorTransitCompleted {
                ship_id: ship_id(1),
                from: dawn_core::SectorId(0),
                to: dawn_core::SectorId(1),
                entry_pos: dawn_core::Position::ORIGIN,
                velocity: dawn_core::Velocity::ZERO,
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
                domain_event_to_json(&event).is_none(),
                "{event:?} must not be forwarded to clients"
            );
        }
    }

    #[test]
    fn redirect_json_carries_resume_identity() {
        let addr: std::net::SocketAddr = "127.0.0.1:7880".parse().unwrap();
        let json = redirect_json(addr, dawn_core::PlayerId(7), ship_id(42));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "Redirect");
        assert_eq!(v["ws_addr"], "127.0.0.1:7880");
        assert_eq!(v["player_id"], 7);
        assert_eq!(v["ship_id"], ship_id(42).raw());
    }

    #[test]
    fn hello_json_can_carry_resume_identity() {
        let line = format!(
            r#"{{"type":"Hello","player_id":7,"ship_id":{}}}"#,
            ship_id(42).raw()
        );
        let hello = parse_hello(&line).expect("must parse Hello");
        assert_eq!(hello.resume.unwrap().player_id, dawn_core::PlayerId(7));
        assert_eq!(hello.resume.unwrap().ship_id, ship_id(42));
    }

    #[test]
    fn hello_json_without_resume_stays_fresh() {
        let hello = parse_hello(r#"{"type":"Hello"}"#).expect("must parse Hello");
        assert!(hello.resume.is_none());
    }

    #[test]
    fn wire_schema_doc_is_up_to_date() {
        assert_schema_file_matches(
            &event_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol.schema.json"
            ),
        );
        assert_schema_file_matches(
            &client_command_json_schema(),
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/architecture/wire-protocol-commands.schema.json"
            ),
        );
    }

    fn assert_schema_file_matches(schema: &schemars::schema::RootSchema, path: &str) {
        let current = serde_json::to_string_pretty(schema).unwrap() + "\n";
        let checked_in =
            std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{path} must exist"));
        assert_eq!(
            current, checked_in,
            "{path} is stale -- regenerate with \
             `cargo run -p dawn-actor --example gen_wire_schema`"
        );
    }
}
