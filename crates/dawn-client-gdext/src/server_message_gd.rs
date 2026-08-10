use crate::loadout_gd::{wire_to_loadout_msg, PlayerLoadout};
use crate::presentation_gd::{
    godot_i64, position_components, velocity_components, InitialStatePresentation, MarketSnapshot,
    MotionCorrectionPresentation, ShipPresentation,
};
use crate::server_message_validation::decode_server_message;
use crate::session_record_gd::DestructionOutcome;
use crate::world_session_gd::WorldSession;
use dawn_client_core::{
    BuildableShipTypeInput, CelestialBodyInput, ClientFact, GateInput, NavigationInput,
    PositionInput, ShipInput, ShipLeaveReason, ShipRegistration, StationInput, SystemNameInput,
    WorldSessionEffect,
};
use dawn_protocol::{InitialStateWire, ServerFact, ServerMessage, ShipStateWire};
use godot::prelude::*;

#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ServerMessageDecoder {}

#[godot_api]
impl ServerMessageDecoder {
    #[func]
    fn decode(&self, bytes: PackedByteArray) -> Option<Gd<ServerMessageOutcome>> {
        match decode_server_message(bytes.as_slice()) {
            Ok(message) => Some(Gd::from_object(ServerMessageOutcome { message })),
            Err(error) => {
                godot_error!("ServerMessageDecoder.decode: {error}");
                None
            }
        }
    }

    #[cfg(debug_assertions)]
    #[func]
    fn test_outcome(&self, kind: GString) -> Option<Gd<ServerMessageOutcome>> {
        use dawn_core::CelestialBodyKind;
        use dawn_protocol::{
            AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, JumpGateWire, MarketOrderWire,
            MarketSnapshotWire, PlayerLoadoutWire, SlotCapacityWire, StationWire, SystemWire,
            VelWire,
        };

        let position = AbsPosWire {
            x: 5.0 * 149_597_870_700.0 + 10.0,
            y: 20.0,
            z: 30.0,
        };
        let ship = ShipStateWire {
            ship_id: 11,
            ship_type_name: "Magpie".to_owned(),
            position,
            velocity: VelWire {
                dx: 4.0,
                dy: 5.0,
                dz: 6.0,
            },
            max_speed: 500.0,
            mass: 10_000.0,
            inertia_modifier: 0.3,
            max_shield: 100.0,
            max_armor: 90.0,
            max_hull: 80.0,
            current_shield: 70.0,
            current_armor: 60.0,
            current_hull: 50.0,
            cap_max: 40.0,
            cap_recharge_per_tick: 1.0,
            is_player: true,
        };
        let mut other_ship = ship.clone();
        other_ship.ship_id = 22;
        other_ship.ship_type_name = "Venture".to_owned();
        other_ship.max_shield = 250.0;
        other_ship.max_armor = 180.0;
        other_ship.max_hull = 120.0;
        other_ship.current_shield = 210.0;
        other_ship.current_armor = 160.0;
        other_ship.current_hull = 110.0;
        other_ship.cap_max = 80.0;
        other_ship.cap_recharge_per_tick = 4.0;
        let mut pending_ship = other_ship.clone();
        pending_ship.ship_id = 33;
        pending_ship.ship_type_name = "Prospect".to_owned();

        let loadout = |tick, active_ship_id| PlayerLoadoutWire {
            tick,
            modules: Vec::new(),
            inventory: Vec::new(),
            station_inventory: Vec::new(),
            docked_station_id: None,
            docked_station_name: None,
            slot_capacity: SlotCapacityWire {
                high: 0,
                mid: 0,
                low: 0,
                rig: 0,
            },
            active_ship_id,
            owned_ships: Vec::new(),
        };
        let docked_loadout = |tick, active_ship_id| {
            let mut result = loadout(tick, active_ship_id);
            result.docked_station_id = Some(5);
            result.docked_station_name = Some("Forge Station".to_owned());
            result
        };

        let message = match kind.to_string().as_str() {
            "Welcome" => ServerMessage::Welcome {
                player_id: 5,
                ship_id: 11,
                resume_ticket: dawn_protocol::ResumeTicket::from_bytes([5; 32]),
            },
            "Redirect" => ServerMessage::Redirect {
                ws_addr: "127.0.0.1:7880".to_owned(),
                resume_ticket: dawn_protocol::ResumeTicket::from_bytes([5; 32]),
            },
            "AoiLeave" => ServerMessage::AoiLeave { ship_id: 19 },
            "AoiEnterPending" => ServerMessage::AoiEnter(pending_ship.clone()),
            "InitialState" => ServerMessage::InitialState(InitialStateWire {
                ships: vec![ship, other_ship],
                system_name: "Alpha".to_owned(),
                systems: vec![SystemWire {
                    id: 2,
                    name: "Beta".to_owned(),
                }],
                jump_gates: vec![JumpGateWire {
                    gate_id: 7,
                    position,
                    activation_radius: 1000.0,
                    to_system_name: "Beta".to_owned(),
                }],
                stations: vec![StationWire {
                    station_id: 5,
                    name: "Forge Station".to_owned(),
                    position,
                    docking_radius: 5000.0,
                }],
                celestial_bodies: vec![CelestialBodyWire {
                    id: 9,
                    kind: CelestialBodyKind::Star,
                    name: "Sun".to_owned(),
                    position,
                    radius: 42.0,
                    spectral_type: 0.5,
                }],
                buildable_ship_types: vec![BuildableShipTypeWire {
                    ship_type_id: 7,
                    name: "Magpie".to_owned(),
                }],
            }),
            "PlayerLoadoutSwitch" => ServerMessage::PlayerLoadout(loadout(12, Some(22))),
            "PlayerLoadoutUnknown" => ServerMessage::PlayerLoadout(loadout(13, Some(33))),
            "PlayerLoadoutUnknownDocked" => {
                ServerMessage::PlayerLoadout(docked_loadout(13, Some(33)))
            }
            "PlayerLoadoutDisembark" => ServerMessage::PlayerLoadout(loadout(14, None)),
            "MotionCorrection" => ServerMessage::MotionCorrection {
                ship_id: 11,
                position,
                velocity: VelWire {
                    dx: 4.0,
                    dy: 5.0,
                    dz: -6.0,
                },
                tick: 42,
            },
            "ShipDocked" => ServerMessage::Fact(ServerFact::ShipDocked {
                ship_id: 11,
                station_id: 5,
                tick: 12,
            }),
            "ShipSpawnedPending" => ServerMessage::Fact(ServerFact::ShipSpawned {
                ship_id: 33,
                position,
                tick: 14,
            }),
            "MarketSnapshot" => ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 250,
                orders: vec![
                    MarketOrderWire {
                        order_id: 1,
                        item_id: dawn_protocol::ItemWire::ScrapMetal,
                        side: "Ask".to_owned(),
                        price: 10,
                        quantity: 2,
                        is_own: true,
                    },
                    MarketOrderWire {
                        order_id: 2,
                        item_id: dawn_protocol::ItemWire::Module { module_id: 3 },
                        side: "Bid".to_owned(),
                        price: 20,
                        quantity: 1,
                        is_own: false,
                    },
                    MarketOrderWire {
                        order_id: 3,
                        item_id: dawn_protocol::ItemWire::PackagedShip { ship_type_id: 7 },
                        side: "Ask".to_owned(),
                        price: 30,
                        quantity: 1,
                        is_own: false,
                    },
                ],
                notice: "Ready".to_owned(),
            }),
            _ => return None,
        };
        self.decode(PackedByteArray::from(
            message
                .encode()
                .expect("typed server message must encode")
                .as_slice(),
        ))
    }
}

/// Validated server message and the receive path's sole presentation seam.
#[derive(GodotClass)]
#[class(no_init)]
pub struct ServerMessageOutcome {
    message: ServerMessage,
}

#[godot_api]
impl ServerMessageOutcome {
    #[func]
    fn dispatch(
        &self,
        mut connection_target: Gd<Object>,
        mut world_target: Gd<Object>,
        mut session: Gd<WorldSession>,
        mut loadout: Gd<PlayerLoadout>,
        connection_ship_id: i64,
    ) -> bool {
        match &self.message {
            ServerMessage::Welcome {
                player_id,
                ship_id,
                resume_ticket,
            } => call_connection(
                &mut connection_target,
                "_accept_welcome",
                vslice![
                    godot_i64(*player_id),
                    godot_i64(*ship_id),
                    PackedByteArray::from(resume_ticket.as_bytes().as_slice()),
                ],
            ),
            ServerMessage::Redirect {
                ws_addr,
                resume_ticket,
            } => call_connection(
                &mut connection_target,
                "_accept_redirect",
                vslice![
                    ws_addr.as_str(),
                    PackedByteArray::from(resume_ticket.as_bytes().as_slice()),
                ],
            ),
            ServerMessage::Fact(ServerFact::ModuleActivated {
                ship_id,
                module_id,
                slot,
                ..
            }) => {
                apply_fact(
                    &mut session,
                    &mut loadout,
                    ClientFact::ModuleActivation {
                        ship_id: godot_i64(*ship_id),
                        module_id: *module_id,
                        active: true,
                        forced_reason: String::new(),
                    },
                );
                call_connection(
                    &mut connection_target,
                    "_accept_module_activated",
                    vslice![godot_i64(*ship_id), i64::from(*module_id), slot.as_str()],
                )
            }
            ServerMessage::Fact(ServerFact::ModuleDeactivated {
                ship_id,
                module_id,
                slot,
                reason,
                ..
            }) => {
                let reason = reason.map(|reason| reason.as_str()).unwrap_or("");
                apply_fact(
                    &mut session,
                    &mut loadout,
                    ClientFact::ModuleActivation {
                        ship_id: godot_i64(*ship_id),
                        module_id: *module_id,
                        active: false,
                        forced_reason: reason.to_owned(),
                    },
                );
                call_connection(
                    &mut connection_target,
                    "_accept_module_deactivated",
                    vslice![
                        godot_i64(*ship_id),
                        i64::from(*module_id),
                        slot.as_str(),
                        reason
                    ],
                )
            }
            ServerMessage::Fact(fact) => {
                let presentation =
                    apply_server_fact(&mut session, &mut loadout, fact, connection_ship_id);
                dispatch_world_event(&presentation, &mut world_target)
            }
            ServerMessage::PlayerLoadout(wire) => {
                apply_fact(
                    &mut session,
                    &mut loadout,
                    ClientFact::PlayerLoadout(wire_to_loadout_msg(wire.clone())),
                );
                call_connection(&mut connection_target, "_accept_player_loadout", vslice![])
            }
            ServerMessage::InitialState(state) => {
                apply_fact(
                    &mut session,
                    &mut loadout,
                    initial_state_fact(state, connection_ship_id),
                );
                call_connection(
                    &mut connection_target,
                    "_accept_initial_state",
                    vslice![InitialStatePresentation::wrap(&state.ships)],
                )
            }
            ServerMessage::AoiEnter(ship) => {
                let effect = apply_fact(
                    &mut session,
                    &mut loadout,
                    ClientFact::ShipEntered {
                        ship: ship_registration(ship),
                        connection_ship_id,
                    },
                );
                let (registered, became_player) = registered_effect(effect);
                dispatch_world_event(
                    &EventPresentation::AoiEnter {
                        ship: ShipPresentation::wrap(ship),
                        registered,
                        became_player,
                    },
                    &mut world_target,
                )
            }
            ServerMessage::AoiLeave { ship_id } => {
                let effect = apply_fact(
                    &mut session,
                    &mut loadout,
                    ClientFact::ShipLeft {
                        ship_id: godot_i64(*ship_id),
                        reason: ShipLeaveReason::AreaOfInterest,
                    },
                );
                dispatch_world_event(
                    &EventPresentation::AoiLeave {
                        ship_id: godot_i64(*ship_id),
                        removed: removed_effect(effect),
                    },
                    &mut world_target,
                )
            }
            ServerMessage::PositionSnap { ship_id, position } => {
                apply_fact(&mut session, &mut loadout, ClientFact::ObservedEvent);
                dispatch_world_event(
                    &EventPresentation::PositionSnap {
                        ship_id: godot_i64(*ship_id),
                        position: position_components(*position),
                    },
                    &mut world_target,
                )
            }
            ServerMessage::MotionCorrection {
                ship_id,
                position,
                velocity,
                tick,
            } => call_connection(
                &mut connection_target,
                "_accept_motion_correction",
                vslice![MotionCorrectionPresentation::wrap(
                    *ship_id, *position, *velocity, *tick
                )],
            ),
            ServerMessage::MarketSnapshot(snapshot) => call_connection(
                &mut connection_target,
                "_accept_market_snapshot",
                vslice![MarketSnapshot::wrap(snapshot)],
            ),
            ServerMessage::ClientRequestRejected(rejection) => call_connection(
                &mut connection_target,
                "_accept_client_request_rejected",
                vslice![format!("{:?}", rejection.code), rejection.message.as_str()],
            ),
        }
    }
}

fn call_connection(target: &mut Gd<Object>, method: &str, arguments: &[Variant]) -> bool {
    if !ensure_handler(target, method) {
        return false;
    }
    target.call(method, arguments);
    true
}

enum EventPresentation {
    ShipSpawned {
        ship_id: i64,
        position: PackedFloat64Array,
        registered: bool,
        became_player: bool,
    },
    VelocityChanged {
        ship_id: i64,
        velocity: PackedFloat64Array,
        tick: i64,
    },
    ShipDespawned {
        ship_id: i64,
        removed: bool,
    },
    ShipDocked {
        ship_id: i64,
        station_id: i64,
        tick: i64,
        accepted: bool,
    },
    ShipUndocked {
        ship_id: i64,
        station_id: i64,
        tick: i64,
        accepted: bool,
    },
    ShipAssembled,
    DamageTaken {
        ship_id: i64,
    },
    RepairApplied {
        ship_id: i64,
    },
    ShipDestroyed {
        ship_id: i64,
        outcome: Gd<DestructionOutcome>,
    },
    TargetLocked {
        locker_id: i64,
        target_id: i64,
        changed: bool,
    },
    LockLost {
        locker_id: i64,
        target_id: i64,
        changed: bool,
    },
    JumpGateUsed {
        ship_id: i64,
        gate_id: i64,
        entry_pos: PackedFloat64Array,
        tick: i64,
    },
    StarSystemChanged {
        ship_id: i64,
        to_system: i64,
        name: Option<String>,
    },
    AoiEnter {
        ship: Gd<ShipPresentation>,
        registered: bool,
        became_player: bool,
    },
    AoiLeave {
        ship_id: i64,
        removed: bool,
    },
    PositionSnap {
        ship_id: i64,
        position: PackedFloat64Array,
    },
}

fn dispatch_world_event(event: &EventPresentation, target: &mut Gd<Object>) -> bool {
    let Some(method) = event_handler(event) else {
        return true;
    };
    if !ensure_handler(target, method) {
        return false;
    }
    match event {
        EventPresentation::ShipSpawned {
            ship_id,
            position,
            registered,
            became_player,
        } => {
            target.call(
                method,
                vslice![*ship_id, position.clone(), *registered, *became_player],
            );
        }
        EventPresentation::VelocityChanged {
            ship_id,
            velocity,
            tick,
        } => {
            target.call(method, vslice![*ship_id, velocity.clone(), *tick]);
        }
        EventPresentation::ShipDespawned { ship_id, removed }
        | EventPresentation::AoiLeave { ship_id, removed } => {
            target.call(method, vslice![*ship_id, *removed]);
        }
        EventPresentation::ShipDocked {
            ship_id,
            station_id,
            tick,
            accepted,
        }
        | EventPresentation::ShipUndocked {
            ship_id,
            station_id,
            tick,
            accepted,
        } => {
            target.call(method, vslice![*ship_id, *station_id, *tick, *accepted]);
        }
        EventPresentation::ShipAssembled => {}
        EventPresentation::DamageTaken { ship_id }
        | EventPresentation::RepairApplied { ship_id } => {
            target.call(method, vslice![*ship_id]);
        }
        EventPresentation::ShipDestroyed { ship_id, outcome } => {
            target.call(method, vslice![*ship_id, outcome.clone()]);
        }
        EventPresentation::TargetLocked {
            locker_id,
            target_id,
            changed,
        }
        | EventPresentation::LockLost {
            locker_id,
            target_id,
            changed,
        } => {
            target.call(method, vslice![*locker_id, *target_id, *changed]);
        }
        EventPresentation::JumpGateUsed {
            ship_id,
            gate_id,
            entry_pos,
            tick,
        } => {
            target.call(
                method,
                vslice![*ship_id, *gate_id, entry_pos.clone(), *tick],
            );
        }
        EventPresentation::StarSystemChanged {
            ship_id,
            to_system,
            name,
        } => {
            let name = name
                .as_ref()
                .map(|name| GString::from(name).to_variant())
                .unwrap_or_else(Variant::nil);
            target.call(method, vslice![*ship_id, *to_system, name]);
        }
        EventPresentation::AoiEnter {
            ship,
            registered,
            became_player,
        } => {
            target.call(method, vslice![ship.clone(), *registered, *became_player]);
        }
        EventPresentation::PositionSnap { ship_id, position } => {
            target.call(method, vslice![*ship_id, position.clone()]);
        }
    }
    true
}

fn apply_server_fact(
    session: &mut Gd<WorldSession>,
    loadout: &mut Gd<PlayerLoadout>,
    fact: &ServerFact,
    connection_ship_id: i64,
) -> EventPresentation {
    match fact {
        ServerFact::ShipSpawned {
            ship_id, position, ..
        } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::ShipSpawned {
                    ship_id: godot_i64(*ship_id),
                    connection_ship_id,
                },
            );
            let (registered, became_player) = registered_effect(effect);
            EventPresentation::ShipSpawned {
                ship_id: godot_i64(*ship_id),
                position: position_components(*position),
                registered,
                became_player,
            }
        }
        ServerFact::VelocityChanged {
            ship_id,
            velocity,
            tick,
        } => {
            apply_fact(
                session,
                loadout,
                ClientFact::Tick {
                    tick: godot_i64(*tick),
                },
            );
            EventPresentation::VelocityChanged {
                ship_id: godot_i64(*ship_id),
                velocity: velocity_components(*velocity),
                tick: godot_i64(*tick),
            }
        }
        ServerFact::ShipDespawned { ship_id, .. } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::ShipLeft {
                    ship_id: godot_i64(*ship_id),
                    reason: ShipLeaveReason::Despawn,
                },
            );
            EventPresentation::ShipDespawned {
                ship_id: godot_i64(*ship_id),
                removed: removed_effect(effect),
            }
        }
        ServerFact::ShipDocked {
            ship_id,
            station_id,
            tick,
        } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::Docked {
                    ship_id: godot_i64(*ship_id),
                    station_id: i64::from(*station_id),
                    tick: godot_i64(*tick),
                },
            );
            EventPresentation::ShipDocked {
                ship_id: godot_i64(*ship_id),
                station_id: i64::from(*station_id),
                tick: godot_i64(*tick),
                accepted: dock_effect(effect),
            }
        }
        ServerFact::ShipUndocked {
            ship_id,
            station_id,
            tick,
        } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::Undocked {
                    ship_id: godot_i64(*ship_id),
                    tick: godot_i64(*tick),
                },
            );
            EventPresentation::ShipUndocked {
                ship_id: godot_i64(*ship_id),
                station_id: i64::from(*station_id),
                tick: godot_i64(*tick),
                accepted: dock_effect(effect),
            }
        }
        ServerFact::ShipAssembled { .. } => {
            apply_fact(session, loadout, ClientFact::ObservedEvent);
            EventPresentation::ShipAssembled
        }
        ServerFact::DamageTaken {
            ship_id,
            current_shield,
            current_armor,
            current_hull,
            ..
        } => {
            apply_fact(
                session,
                loadout,
                ClientFact::HealthChanged {
                    ship_id: godot_i64(*ship_id),
                    shield: f64::from(*current_shield),
                    armor: f64::from(*current_armor),
                    hull: f64::from(*current_hull),
                },
            );
            EventPresentation::DamageTaken {
                ship_id: godot_i64(*ship_id),
            }
        }
        ServerFact::RepairApplied {
            ship_id,
            current_shield,
            current_armor,
            current_hull,
            ..
        } => {
            apply_fact(
                session,
                loadout,
                ClientFact::HealthChanged {
                    ship_id: godot_i64(*ship_id),
                    shield: f64::from(*current_shield),
                    armor: f64::from(*current_armor),
                    hull: f64::from(*current_hull),
                },
            );
            EventPresentation::RepairApplied {
                ship_id: godot_i64(*ship_id),
            }
        }
        ServerFact::ShipDestroyed { ship_id, .. } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::ShipDestroyed {
                    ship_id: godot_i64(*ship_id),
                },
            );
            let WorldSessionEffect::ShipDestroyed(outcome) = effect else {
                unreachable!("ShipDestroyed fact must return destruction state")
            };
            EventPresentation::ShipDestroyed {
                ship_id: godot_i64(*ship_id),
                outcome: DestructionOutcome::wrap(outcome),
            }
        }
        ServerFact::TargetLocked {
            locker_id,
            target_id,
            ..
        } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::TargetLocked {
                    locker_id: godot_i64(*locker_id),
                    target_id: godot_i64(*target_id),
                },
            );
            EventPresentation::TargetLocked {
                locker_id: godot_i64(*locker_id),
                target_id: godot_i64(*target_id),
                changed: lock_effect(effect),
            }
        }
        ServerFact::LockLost {
            locker_id,
            target_id,
            ..
        } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::LockLost {
                    locker_id: godot_i64(*locker_id),
                    target_id: godot_i64(*target_id),
                },
            );
            EventPresentation::LockLost {
                locker_id: godot_i64(*locker_id),
                target_id: godot_i64(*target_id),
                changed: lock_effect(effect),
            }
        }
        ServerFact::ModuleActivated { .. } | ServerFact::ModuleDeactivated { .. } => {
            unreachable!("module events dispatch at the connection boundary")
        }
        ServerFact::JumpGateUsed {
            ship_id,
            gate_id,
            entry_pos,
            tick,
            ..
        } => {
            apply_fact(session, loadout, ClientFact::ObservedEvent);
            EventPresentation::JumpGateUsed {
                ship_id: godot_i64(*ship_id),
                gate_id: i64::from(*gate_id),
                entry_pos: position_components(*entry_pos),
                tick: godot_i64(*tick),
            }
        }
        ServerFact::StarSystemChanged {
            ship_id, to_system, ..
        } => {
            let effect = apply_fact(
                session,
                loadout,
                ClientFact::SystemChanged {
                    ship_id: godot_i64(*ship_id),
                    to_system: i64::from(*to_system),
                },
            );
            EventPresentation::StarSystemChanged {
                ship_id: godot_i64(*ship_id),
                to_system: i64::from(*to_system),
                name: system_effect(effect),
            }
        }
    }
}

fn apply_fact(
    session: &mut Gd<WorldSession>,
    loadout: &mut Gd<PlayerLoadout>,
    fact: ClientFact,
) -> WorldSessionEffect {
    let mut session = session.bind_mut();
    let mut loadout = loadout.bind_mut();
    session
        .apply_fact(fact, loadout.core_slot_mut())
        .expect("validated server facts must fit the client state")
}

fn initial_state_fact(state: &InitialStateWire, connection_ship_id: i64) -> ClientFact {
    ClientFact::InitialState {
        navigation: NavigationInput {
            system_name: state.system_name.clone(),
            systems: state
                .systems
                .iter()
                .map(|system| SystemNameInput {
                    id: i64::from(system.id),
                    name: system.name.clone(),
                })
                .collect(),
            jump_gates: state
                .jump_gates
                .iter()
                .map(|gate| GateInput {
                    gate_id: i64::from(gate.gate_id),
                    position: position_input(gate.position),
                    activation_radius: gate.activation_radius,
                    to_system_name: gate.to_system_name.clone(),
                })
                .collect(),
            stations: state
                .stations
                .iter()
                .map(|station| StationInput {
                    station_id: i64::from(station.station_id),
                    name: station.name.clone(),
                    position: position_input(station.position),
                    docking_radius: station.docking_radius,
                })
                .collect(),
            celestial_bodies: state
                .celestial_bodies
                .iter()
                .map(|body| CelestialBodyInput {
                    id: i64::from(body.id),
                    kind: format!("{:?}", body.kind),
                    name: body.name.clone(),
                    position: position_input(body.position),
                    radius: body.radius,
                    spectral_type: f64::from(body.spectral_type),
                })
                .collect(),
            buildable_ship_types: state
                .buildable_ship_types
                .iter()
                .map(|ship| BuildableShipTypeInput {
                    ship_type_id: i64::from(ship.ship_type_id),
                    name: ship.name.clone(),
                })
                .collect(),
        },
        ships: state.ships.iter().map(ship_registration).collect(),
        connection_ship_id,
    }
}

fn ship_registration(ship: &ShipStateWire) -> ShipRegistration {
    ShipRegistration {
        ship_id: godot_i64(ship.ship_id),
        ship: ShipInput {
            is_player: ship.is_player,
            ship_type_name: ship.ship_type_name.clone(),
            max_shield: f64::from(ship.max_shield),
            max_armor: f64::from(ship.max_armor),
            max_hull: f64::from(ship.max_hull),
            current_shield: Some(f64::from(ship.current_shield)),
            current_armor: Some(f64::from(ship.current_armor)),
            current_hull: Some(f64::from(ship.current_hull)),
            cap_max: f64::from(ship.cap_max),
            cap_recharge_per_tick: f64::from(ship.cap_recharge_per_tick),
        },
    }
}

fn position_input(position: dawn_protocol::AbsPosWire) -> PositionInput {
    PositionInput {
        x: position.x,
        y: position.y,
        z: position.z,
    }
}

fn registered_effect(effect: WorldSessionEffect) -> (bool, bool) {
    match effect {
        WorldSessionEffect::ShipRegistered {
            registered,
            became_player,
        } => (registered, became_player),
        _ => unreachable!("ship registration fact returned an unexpected effect"),
    }
}

fn removed_effect(effect: WorldSessionEffect) -> bool {
    match effect {
        WorldSessionEffect::ShipRemoved { removed } => removed,
        _ => unreachable!("ship removal fact returned an unexpected effect"),
    }
}

fn lock_effect(effect: WorldSessionEffect) -> bool {
    match effect {
        WorldSessionEffect::LockChanged { changed } => changed,
        _ => unreachable!("lock fact returned an unexpected effect"),
    }
}

fn dock_effect(effect: WorldSessionEffect) -> bool {
    match effect {
        WorldSessionEffect::DockState { accepted } => accepted,
        _ => unreachable!("dock fact returned an unexpected effect"),
    }
}

fn system_effect(effect: WorldSessionEffect) -> Option<String> {
    match effect {
        WorldSessionEffect::SystemChanged { name } => name,
        _ => unreachable!("system fact returned an unexpected effect"),
    }
}

fn event_handler(event: &EventPresentation) -> Option<&'static str> {
    Some(match event {
        EventPresentation::ShipSpawned { .. } => "_handle_ship_spawned",
        EventPresentation::VelocityChanged { .. } => "_handle_velocity_changed",
        EventPresentation::ShipDespawned { .. } => "_handle_ship_despawned",
        EventPresentation::ShipDocked { .. } => "_handle_ship_docked",
        EventPresentation::ShipUndocked { .. } => "_handle_ship_undocked",
        EventPresentation::ShipAssembled => return None,
        EventPresentation::DamageTaken { .. } => "_handle_damage_taken",
        EventPresentation::RepairApplied { .. } => "_handle_repair_applied",
        EventPresentation::ShipDestroyed { .. } => "_handle_ship_destroyed",
        EventPresentation::TargetLocked { .. } => "_handle_target_locked",
        EventPresentation::LockLost { .. } => "_handle_lock_lost",
        EventPresentation::JumpGateUsed { .. } => "_handle_jump_gate_used",
        EventPresentation::StarSystemChanged { .. } => "_handle_star_system_changed",
        EventPresentation::AoiEnter { .. } => "_handle_aoi_enter",
        EventPresentation::AoiLeave { .. } => "_handle_aoi_leave",
        EventPresentation::PositionSnap { .. } => "_handle_position_snap",
    })
}

fn ensure_handler(target: &mut Gd<Object>, method: &str) -> bool {
    let exists = target
        .call("has_method", vslice![method])
        .try_to::<bool>()
        .unwrap_or(false);
    if !exists {
        godot_warn!("typed ServerMessage dispatch target is missing method '{method}'");
    }
    exists
}
