use crate::client_outcome::{ClientEventOutcome, ClientOutcome};
use crate::loadout_gd::PlayerLoadout;
use crate::presentation_gd::{
    godot_i64, position_components, velocity_components, InitialStatePresentation, MarketSnapshot,
    MotionCorrectionPresentation, ShipPresentation,
};
use crate::session_record_gd::DestructionOutcome;
use crate::world_session_gd::WorldSession;
use dawn_client_core::{
    BuildableShipTypeInput, CelestialBodyInput, GateInput, NavigationInput, PositionInput,
    ShipInput, ShipRegistration, StationInput, SystemNameInput, WorldSessionEffect,
    WorldSessionUpdate,
};
use dawn_wire::{EventWire, InitialStateWire, ServerMessage, ShipStateWire};
use godot::prelude::*;

/// Decodes one binary WebSocket frame into a typed client outcome.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ServerMessageDecoder {}

#[godot_api]
impl ServerMessageDecoder {
    #[func]
    fn decode(&self, bytes: PackedByteArray) -> Option<Gd<ServerMessageOutcome>> {
        match ClientOutcome::decode(bytes.as_slice()) {
            Ok(outcome) => Some(Gd::from_object(ServerMessageOutcome { outcome })),
            Err(error) => {
                godot_error!("ServerMessageDecoder.decode: {error}");
                None
            }
        }
    }

    /// Debug-build fixture used by GdUnit to exercise the real binary path.
    #[cfg(debug_assertions)]
    #[func]
    fn test_outcome(&self, kind: GString) -> Option<Gd<ServerMessageOutcome>> {
        use dawn_core::CelestialBodyKind;
        use dawn_wire::{
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

        let message = match kind.to_string().as_str() {
            "Welcome" => ServerMessage::Welcome {
                player_id: 5,
                ship_id: 11,
                resume_ticket: dawn_wire::ResumeTicket::from_bytes([5; 32]),
            },
            "Redirect" => ServerMessage::Redirect {
                ws_addr: "127.0.0.1:7880".to_owned(),
                resume_ticket: dawn_wire::ResumeTicket::from_bytes([5; 32]),
            },
            "AoiLeave" => ServerMessage::AoiLeave { ship_id: 19 },
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
            "PlayerLoadoutSwitch" => ServerMessage::PlayerLoadout(PlayerLoadoutWire {
                tick: 12,
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
                active_ship_id: Some(22),
                owned_ships: Vec::new(),
            }),
            "PlayerLoadoutUnknown" => ServerMessage::PlayerLoadout(PlayerLoadoutWire {
                tick: 13,
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
                active_ship_id: Some(33),
                owned_ships: Vec::new(),
            }),
            "PlayerLoadoutDisembark" => ServerMessage::PlayerLoadout(PlayerLoadoutWire {
                tick: 14,
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
                active_ship_id: None,
                owned_ships: Vec::new(),
            }),
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
            "ShipDocked" => ServerMessage::Event(EventWire::ShipDocked {
                ship_id: 11,
                station_id: 5,
                tick: 12,
            }),
            "MarketSnapshot" => ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 250,
                orders: vec![
                    MarketOrderWire {
                        order_id: 1,
                        item_id: dawn_wire::ItemWire::ScrapMetal,
                        side: "Ask".to_owned(),
                        price: 10,
                        quantity: 2,
                        is_own: true,
                    },
                    MarketOrderWire {
                        order_id: 2,
                        item_id: dawn_wire::ItemWire::Module { module_id: 3 },
                        side: "Bid".to_owned(),
                        price: 20,
                        quantity: 1,
                        is_own: false,
                    },
                    MarketOrderWire {
                        order_id: 3,
                        item_id: dawn_wire::ItemWire::PackagedShip { ship_type_id: 7 },
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
        self.decode(PackedByteArray::from(message.encode().as_slice()))
    }
}

/// One typed top-level server outcome. Applying it mutates the Rust-owned
/// session/loadout before any presentation callback runs.
#[derive(GodotClass)]
#[class(no_init)]
pub struct ServerMessageOutcome {
    outcome: ClientOutcome,
}

#[godot_api]
impl ServerMessageOutcome {
    #[func]
    fn dispatch(
        &self,
        mut target: Gd<Object>,
        mut session: Gd<WorldSession>,
        mut loadout: Gd<PlayerLoadout>,
        connection_ship_id: i64,
    ) -> bool {
        match &self.outcome {
            ClientOutcome::Welcome {
                player_id,
                ship_id,
                resume_ticket,
            } => {
                if !ensure_handler(&mut target, "_accept_welcome") {
                    return false;
                }
                target.call(
                    "_accept_welcome",
                    vslice![
                        godot_i64(*player_id),
                        godot_i64(*ship_id),
                        PackedByteArray::from(resume_ticket.as_bytes().as_slice()),
                    ],
                );
            }
            ClientOutcome::Redirect {
                ws_addr,
                resume_ticket,
            } => {
                if !ensure_handler(&mut target, "_accept_redirect") {
                    return false;
                }
                target.call(
                    "_accept_redirect",
                    vslice![
                        ws_addr.as_str(),
                        PackedByteArray::from(resume_ticket.as_bytes().as_slice()),
                    ],
                );
            }
            ClientOutcome::Event(event) => {
                let presentation =
                    apply_event(&mut session, &mut loadout, event, connection_ship_id);
                if !ensure_handler(&mut target, "_accept_event") {
                    return false;
                }
                target.call(
                    "_accept_event",
                    vslice![Gd::from_object(ServerEventOutcome { presentation })],
                );
            }
            ClientOutcome::PlayerLoadout(wire) => {
                {
                    let mut loadout = loadout.bind_mut();
                    loadout.replace_wire(wire.clone());
                }
                let update = WorldSessionUpdate::PlayerLoadout {
                    active_ship_id: wire.active_ship_id.map(godot_i64),
                    docked_station_id: wire.docked_station_id.map(i64::from),
                    docked_station_name: wire.docked_station_name.clone(),
                    tick: godot_i64(wire.tick),
                };
                apply_update(&mut session, &mut loadout, update);
                if !ensure_handler(&mut target, "_accept_player_loadout") {
                    return false;
                }
                target.call("_accept_player_loadout", vslice![]);
            }
            ClientOutcome::InitialState(state) => {
                apply_update(
                    &mut session,
                    &mut loadout,
                    initial_state_update(state, connection_ship_id),
                );
                if !ensure_handler(&mut target, "_accept_initial_state") {
                    return false;
                }
                target.call(
                    "_accept_initial_state",
                    vslice![InitialStatePresentation::wrap(&state.ships)],
                );
            }
            ClientOutcome::ModuleActivated {
                ship_id,
                module_id,
                slot,
            } => {
                loadout
                    .bind_mut()
                    .apply_activation(*module_id, true, String::new());
                if !ensure_handler(&mut target, "_accept_module_activated") {
                    return false;
                }
                target.call(
                    "_accept_module_activated",
                    vslice![godot_i64(*ship_id), i64::from(*module_id), slot.as_str()],
                );
            }
            ClientOutcome::ModuleDeactivated {
                ship_id,
                module_id,
                slot,
                reason,
            } => {
                let reason = reason.as_deref().unwrap_or("");
                loadout
                    .bind_mut()
                    .apply_activation(*module_id, false, reason.to_owned());
                if !ensure_handler(&mut target, "_accept_module_deactivated") {
                    return false;
                }
                target.call(
                    "_accept_module_deactivated",
                    vslice![
                        godot_i64(*ship_id),
                        i64::from(*module_id),
                        slot.as_str(),
                        reason
                    ],
                );
            }
            ClientOutcome::MotionCorrection {
                ship_id,
                position,
                velocity,
                tick,
            } => {
                if !ensure_handler(&mut target, "_accept_motion_correction") {
                    return false;
                }
                target.call(
                    "_accept_motion_correction",
                    vslice![MotionCorrectionPresentation::wrap(
                        *ship_id, *position, *velocity, *tick
                    )],
                );
            }
            ClientOutcome::MarketSnapshot(snapshot) => {
                if !ensure_handler(&mut target, "_accept_market_snapshot") {
                    return false;
                }
                target.call(
                    "_accept_market_snapshot",
                    vslice![MarketSnapshot::wrap(snapshot)],
                );
            }
        }
        true
    }
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
        session_accepted: bool,
    },
    ShipUndocked {
        ship_id: i64,
        station_id: i64,
        tick: i64,
        session_accepted: bool,
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

#[derive(GodotClass)]
#[class(no_init)]
pub struct ServerEventOutcome {
    presentation: EventPresentation,
}

#[godot_api]
impl ServerEventOutcome {
    #[func]
    fn dispatch(&self, mut target: Gd<Object>) -> bool {
        let Some(method) = event_handler(&self.presentation) else {
            return true;
        };
        if !ensure_handler(&mut target, method) {
            return false;
        }
        match &self.presentation {
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
                session_accepted,
            }
            | EventPresentation::ShipUndocked {
                ship_id,
                station_id,
                tick,
                session_accepted,
            } => {
                target.call(
                    method,
                    vslice![*ship_id, *station_id, *tick, *session_accepted],
                );
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
}

fn apply_event(
    session: &mut Gd<WorldSession>,
    loadout: &mut Gd<PlayerLoadout>,
    event: &ClientEventOutcome,
    connection_ship_id: i64,
) -> EventPresentation {
    match event {
        ClientEventOutcome::Domain(event) => {
            apply_domain_event(session, loadout, event, connection_ship_id)
        }
        ClientEventOutcome::AoiEnter(ship) => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::ShipEntered {
                    ship: ship_registration(ship),
                    connection_ship_id,
                },
            );
            let (registered, became_player) = registered_effect(effect);
            EventPresentation::AoiEnter {
                ship: ShipPresentation::wrap(ship),
                registered,
                became_player,
            }
        }
        ClientEventOutcome::AoiLeave { ship_id } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::ShipLeft {
                    ship_id: godot_i64(*ship_id),
                    clear_lock: false,
                },
            );
            EventPresentation::AoiLeave {
                ship_id: godot_i64(*ship_id),
                removed: removed_effect(effect),
            }
        }
        ClientEventOutcome::PositionSnap { ship_id, position } => {
            apply_update(session, loadout, WorldSessionUpdate::ObservedEvent);
            EventPresentation::PositionSnap {
                ship_id: godot_i64(*ship_id),
                position: position_components(*position),
            }
        }
    }
}

fn apply_domain_event(
    session: &mut Gd<WorldSession>,
    loadout: &mut Gd<PlayerLoadout>,
    event: &EventWire,
    connection_ship_id: i64,
) -> EventPresentation {
    match event {
        EventWire::ShipSpawned {
            ship_id, position, ..
        } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::ShipSpawned {
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
        EventWire::VelocityChanged {
            ship_id,
            velocity,
            tick,
        } => {
            apply_update(
                session,
                loadout,
                WorldSessionUpdate::Tick {
                    tick: godot_i64(*tick),
                },
            );
            EventPresentation::VelocityChanged {
                ship_id: godot_i64(*ship_id),
                velocity: velocity_components(*velocity),
                tick: godot_i64(*tick),
            }
        }
        EventWire::ShipDespawned { ship_id, .. } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::ShipLeft {
                    ship_id: godot_i64(*ship_id),
                    clear_lock: true,
                },
            );
            EventPresentation::ShipDespawned {
                ship_id: godot_i64(*ship_id),
                removed: removed_effect(effect),
            }
        }
        EventWire::ShipDocked {
            ship_id,
            station_id,
            tick,
        } => {
            let station_id = i64::from(*station_id);
            let station_name = session.bind().station_name(station_id);
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::Docked {
                    ship_id: godot_i64(*ship_id),
                    station_id,
                    station_name,
                    tick: godot_i64(*tick),
                },
            );
            EventPresentation::ShipDocked {
                ship_id: godot_i64(*ship_id),
                station_id,
                tick: godot_i64(*tick),
                session_accepted: dock_effect(effect),
            }
        }
        EventWire::ShipUndocked {
            ship_id,
            station_id,
            tick,
        } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::Undocked {
                    ship_id: godot_i64(*ship_id),
                    tick: godot_i64(*tick),
                },
            );
            EventPresentation::ShipUndocked {
                ship_id: godot_i64(*ship_id),
                station_id: i64::from(*station_id),
                tick: godot_i64(*tick),
                session_accepted: dock_effect(effect),
            }
        }
        EventWire::ShipAssembled { .. } => {
            apply_update(session, loadout, WorldSessionUpdate::ObservedEvent);
            EventPresentation::ShipAssembled
        }
        EventWire::DamageTaken {
            ship_id,
            current_shield,
            current_armor,
            current_hull,
            ..
        } => {
            apply_update(
                session,
                loadout,
                WorldSessionUpdate::HealthChanged {
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
        EventWire::RepairApplied {
            ship_id,
            current_shield,
            current_armor,
            current_hull,
            ..
        } => {
            apply_update(
                session,
                loadout,
                WorldSessionUpdate::HealthChanged {
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
        EventWire::ShipDestroyed { ship_id, .. } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::ShipDestroyed {
                    ship_id: godot_i64(*ship_id),
                },
            );
            let outcome = match effect {
                WorldSessionEffect::ShipDestroyed(outcome) => outcome,
                _ => unreachable!("ShipDestroyed update must return its outcome"),
            };
            EventPresentation::ShipDestroyed {
                ship_id: godot_i64(*ship_id),
                outcome: DestructionOutcome::wrap(outcome),
            }
        }
        EventWire::TargetLocked {
            locker_id,
            target_id,
            ..
        } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::TargetLocked {
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
        EventWire::LockLost {
            locker_id,
            target_id,
            ..
        } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::LockLost {
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
        EventWire::ModuleActivated { .. } | EventWire::ModuleDeactivated { .. } => {
            unreachable!("module events project to top-level ClientOutcome variants")
        }
        EventWire::JumpGateUsed {
            ship_id,
            gate_id,
            entry_pos,
            tick,
            ..
        } => {
            apply_update(session, loadout, WorldSessionUpdate::ObservedEvent);
            EventPresentation::JumpGateUsed {
                ship_id: godot_i64(*ship_id),
                gate_id: i64::from(*gate_id),
                entry_pos: position_components(*entry_pos),
                tick: godot_i64(*tick),
            }
        }
        EventWire::StarSystemChanged {
            ship_id, to_system, ..
        } => {
            let effect = apply_update(
                session,
                loadout,
                WorldSessionUpdate::SystemChanged {
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

fn apply_update(
    session: &mut Gd<WorldSession>,
    loadout: &mut Gd<PlayerLoadout>,
    update: WorldSessionUpdate,
) -> WorldSessionEffect {
    let mut session = session.bind_mut();
    let mut loadout = loadout.bind_mut();
    session.apply_update(update, loadout.core_mut())
}

fn initial_state_update(state: &InitialStateWire, connection_ship_id: i64) -> WorldSessionUpdate {
    WorldSessionUpdate::InitialState {
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

fn position_input(position: dawn_wire::AbsPosWire) -> PositionInput {
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
        _ => unreachable!("Ship registration update must return registration state"),
    }
}

fn removed_effect(effect: WorldSessionEffect) -> bool {
    match effect {
        WorldSessionEffect::ShipRemoved { removed } => removed,
        _ => unreachable!("Ship removal update must return removal state"),
    }
}

fn lock_effect(effect: WorldSessionEffect) -> bool {
    match effect {
        WorldSessionEffect::LockChanged { changed } => changed,
        _ => unreachable!("Lock update must return lock state"),
    }
}

fn dock_effect(effect: WorldSessionEffect) -> bool {
    match effect {
        WorldSessionEffect::DockState { accepted } => accepted,
        _ => unreachable!("Dock update must return acceptance state"),
    }
}

fn system_effect(effect: WorldSessionEffect) -> Option<String> {
    match effect {
        WorldSessionEffect::SystemChanged { name } => name,
        _ => unreachable!("System update must return the display name"),
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
