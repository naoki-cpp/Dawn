//! Canonical inbound delivery after wire decoding and Godot range validation.
//!
//! This module owns the exhaustive server-message decision. For state-bearing
//! world messages it converts to a typed client fact and commits state before
//! invoking one final handler; presentation-only messages are delivered once.

use crate::loadout_gd::{wire_to_loadout_msg, PlayerLoadout};
use crate::presentation_gd::{
    godot_i64, position_components, velocity_components, InitialStatePresentation, MarketSnapshot,
    MotionCorrectionPresentation, ShipPresentation,
};
use crate::session_record_gd::DestructionOutcome;
use crate::world_session_gd::WorldSession;
use dawn_client_core::{
    BuildableShipTypeInput, CelestialBodyInput, ClientFact, GateInput, NavigationInput,
    PositionInput, ShipInput, ShipLeaveReason, ShipRegistration, StationInput, SystemNameInput,
    WorldSessionEffect,
};
use dawn_protocol::{InitialStateWire, ServerFact, ServerMessage, ShipStateWire};
use godot::prelude::*;

pub(crate) fn dispatch(
    message: &ServerMessage,
    mut connection_target: Gd<Object>,
    mut world_target: Gd<Object>,
    mut session: Gd<WorldSession>,
    mut loadout: Gd<PlayerLoadout>,
    connection_ship_id: i64,
) -> bool {
    match message {
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
            call_world(
                &mut world_target,
                "_on_module_activated",
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
            call_world(
                &mut world_target,
                "_on_module_deactivated",
                vslice![
                    godot_i64(*ship_id),
                    i64::from(*module_id),
                    slot.as_str(),
                    reason
                ],
            )
        }
        ServerMessage::Fact(fact) => dispatch_server_fact(
            &mut session,
            &mut loadout,
            fact,
            connection_ship_id,
            &mut world_target,
        ),
        ServerMessage::PlayerLoadout(wire) => {
            apply_fact(
                &mut session,
                &mut loadout,
                ClientFact::PlayerLoadout(wire_to_loadout_msg(wire.clone())),
            );
            call_world(&mut world_target, "_on_player_fitting", vslice![])
        }
        ServerMessage::InitialState(state) => {
            apply_fact(
                &mut session,
                &mut loadout,
                initial_state_fact(state, connection_ship_id),
            );
            call_world(
                &mut world_target,
                "_on_initial_state",
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
            call_world(
                &mut world_target,
                "_handle_aoi_enter",
                vslice![ShipPresentation::wrap(ship), registered, became_player],
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
            call_world(
                &mut world_target,
                "_handle_aoi_leave",
                vslice![godot_i64(*ship_id), removed_effect(effect)],
            )
        }
        ServerMessage::PositionSnap { ship_id, position } => {
            apply_fact(&mut session, &mut loadout, ClientFact::ObservedEvent);
            call_world(
                &mut world_target,
                "_handle_position_snap",
                vslice![godot_i64(*ship_id), position_components(*position)],
            )
        }
        ServerMessage::MotionCorrection {
            ship_id,
            position,
            velocity,
            tick,
        } => call_world(
            &mut world_target,
            "_handle_motion_correction",
            vslice![MotionCorrectionPresentation::wrap(
                *ship_id, *position, *velocity, *tick
            )],
        ),
        ServerMessage::MarketSnapshot(snapshot) => call_world(
            &mut world_target,
            "_on_market_snapshot",
            vslice![MarketSnapshot::wrap(snapshot)],
        ),
        ServerMessage::ClientRequestRejected(rejection) => call_connection(
            &mut connection_target,
            "_accept_client_request_rejected",
            vslice![format!("{:?}", rejection.code), rejection.message.as_str()],
        ),
    }
}

fn call_connection(target: &mut Gd<Object>, method: &str, arguments: &[Variant]) -> bool {
    call_handler(target, method, arguments)
}

fn call_world(target: &mut Gd<Object>, method: &str, arguments: &[Variant]) -> bool {
    call_handler(target, method, arguments)
}

fn call_handler(target: &mut Gd<Object>, method: &str, arguments: &[Variant]) -> bool {
    if !ensure_handler(target, method) {
        return false;
    }
    target.call(method, arguments);
    true
}

fn dispatch_server_fact(
    session: &mut Gd<WorldSession>,
    loadout: &mut Gd<PlayerLoadout>,
    fact: &ServerFact,
    connection_ship_id: i64,
    target: &mut Gd<Object>,
) -> bool {
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
            call_world(
                target,
                "_handle_ship_spawned",
                vslice![
                    godot_i64(*ship_id),
                    position_components(*position),
                    registered,
                    became_player
                ],
            )
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
            call_world(
                target,
                "_handle_velocity_changed",
                vslice![
                    godot_i64(*ship_id),
                    velocity_components(*velocity),
                    godot_i64(*tick)
                ],
            )
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
            call_world(
                target,
                "_handle_ship_despawned",
                vslice![godot_i64(*ship_id), removed_effect(effect)],
            )
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
            call_world(
                target,
                "_handle_ship_docked",
                vslice![
                    godot_i64(*ship_id),
                    i64::from(*station_id),
                    godot_i64(*tick),
                    dock_effect(effect)
                ],
            )
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
            call_world(
                target,
                "_handle_ship_undocked",
                vslice![
                    godot_i64(*ship_id),
                    i64::from(*station_id),
                    godot_i64(*tick),
                    dock_effect(effect)
                ],
            )
        }
        ServerFact::ShipAssembled { .. } => {
            apply_fact(session, loadout, ClientFact::ObservedEvent);
            true
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
            call_world(target, "_handle_damage_taken", vslice![godot_i64(*ship_id)])
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
            call_world(
                target,
                "_handle_repair_applied",
                vslice![godot_i64(*ship_id)],
            )
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
            call_world(
                target,
                "_handle_ship_destroyed",
                vslice![godot_i64(*ship_id), DestructionOutcome::wrap(outcome)],
            )
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
            call_world(
                target,
                "_handle_target_locked",
                vslice![
                    godot_i64(*locker_id),
                    godot_i64(*target_id),
                    lock_effect(effect)
                ],
            )
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
            call_world(
                target,
                "_handle_lock_lost",
                vslice![
                    godot_i64(*locker_id),
                    godot_i64(*target_id),
                    lock_effect(effect)
                ],
            )
        }
        ServerFact::ModuleActivated { .. } | ServerFact::ModuleDeactivated { .. } => {
            unreachable!("module events dispatch in the top-level message match")
        }
        ServerFact::JumpGateUsed {
            ship_id,
            gate_id,
            entry_pos,
            tick,
            ..
        } => {
            apply_fact(session, loadout, ClientFact::ObservedEvent);
            call_world(
                target,
                "_handle_jump_gate_used",
                vslice![
                    godot_i64(*ship_id),
                    i64::from(*gate_id),
                    position_components(*entry_pos),
                    godot_i64(*tick)
                ],
            )
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
            let name = system_effect(effect)
                .map(|name| GString::from(name.as_str()).to_variant())
                .unwrap_or_else(Variant::nil);
            call_world(
                target,
                "_handle_star_system_changed",
                vslice![godot_i64(*ship_id), i64::from(*to_system), name],
            )
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
