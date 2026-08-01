use crate::client_outcome::{ClientEventOutcome, ClientOutcome};
use crate::json_variant::Dict;
use dawn_wire::{
    AbsPosWire, EventWire, InitialStateWire, ItemWire, MarketOrderWire, MarketSnapshotWire,
    ServerMessage, ShipStateWire, VelWire,
};
use godot::prelude::*;

/// Decodes one binary WebSocket frame into a typed client outcome.
///
/// The decoder no longer builds a compatibility Dictionary or a string
/// `"type"` tag. The returned outcome owns the Rust-side projection and
/// dispatches it to a fixed GDScript receiver method.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ServerMessageDecoder {}

#[godot_api]
impl ServerMessageDecoder {
    #[func]
    fn decode(&self, bytes: PackedByteArray) -> Option<Gd<ServerMessageOutcome>> {
        match ClientOutcome::decode(bytes.as_slice()) {
            Ok(outcome) => Some(Gd::from_object(ServerMessageOutcome {
                outcome,
                raw_bytes: bytes,
            })),
            Err(error) => {
                godot_error!("ServerMessageDecoder.decode: {error}");
                None
            }
        }
    }

    /// Debug-build fixture used by GdUnit to exercise the actual
    /// raw-frame -> typed-outcome -> dispatch boundary.
    #[cfg(debug_assertions)]
    #[func]
    fn test_outcome(&self, kind: GString) -> Option<Gd<ServerMessageOutcome>> {
        let message = match kind.to_string().as_str() {
            "Welcome" => ServerMessage::Welcome {
                player_id: 5,
                ship_id: 11,
            },
            "Redirect" => ServerMessage::Redirect {
                ws_addr: "127.0.0.1:7880".to_owned(),
                player_id: 5,
                ship_id: 11,
            },
            "AoiLeave" => ServerMessage::AoiLeave { ship_id: 19 },
            "InitialState" => ServerMessage::InitialState(InitialStateWire {
                ships: Vec::new(),
                system_name: "Alpha".to_owned(),
                systems: Vec::new(),
                jump_gates: Vec::new(),
                stations: Vec::new(),
                celestial_bodies: Vec::new(),
                buildable_ship_types: Vec::new(),
            }),
            "MarketSnapshot" => ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 250,
                orders: vec![
                    MarketOrderWire {
                        order_id: 1,
                        item_id: ItemWire::ScrapMetal,
                        side: "Ask".to_owned(),
                        price: 10,
                        quantity: 2,
                        is_own: true,
                    },
                    MarketOrderWire {
                        order_id: 2,
                        item_id: ItemWire::Module { module_id: 3 },
                        side: "Bid".to_owned(),
                        price: 20,
                        quantity: 1,
                        is_own: false,
                    },
                    MarketOrderWire {
                        order_id: 3,
                        item_id: ItemWire::PackagedShip { ship_type_id: 7 },
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

/// One typed top-level server outcome.
#[derive(GodotClass)]
#[class(no_init)]
pub struct ServerMessageOutcome {
    outcome: ClientOutcome,
    raw_bytes: PackedByteArray,
}

#[godot_api]
impl ServerMessageOutcome {
    /// Dispatch to `connection.gd` without exposing wire variant names or
    /// field-key matching to GDScript.
    #[func]
    fn dispatch(&self, mut target: Gd<Object>) -> bool {
        if !ensure_handler(&mut target, server_outcome_handler(&self.outcome)) {
            return false;
        }
        match &self.outcome {
            ClientOutcome::Welcome { player_id, ship_id } => {
                target.call(
                    "_accept_welcome",
                    vslice![godot_i64(*player_id), godot_i64(*ship_id)],
                );
            }
            ClientOutcome::Redirect {
                ws_addr,
                player_id,
                ship_id,
            } => {
                target.call(
                    "_accept_redirect",
                    vslice![ws_addr.as_str(), godot_i64(*player_id), godot_i64(*ship_id)],
                );
            }
            ClientOutcome::Event(event) => {
                let event = Gd::from_object(ServerEventOutcome {
                    outcome: event.clone(),
                });
                target.call("_accept_event", vslice![event]);
            }
            ClientOutcome::PlayerLoadout => {
                target.call("_accept_player_loadout", vslice![self.raw_bytes.clone()]);
            }
            ClientOutcome::InitialState(state) => {
                target.call(
                    "_accept_initial_state",
                    vslice![initial_state_to_dict(state)],
                );
            }
            ClientOutcome::ModuleActivated {
                ship_id,
                module_id,
                slot,
            } => {
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
                target.call(
                    "_accept_module_deactivated",
                    vslice![
                        godot_i64(*ship_id),
                        i64::from(*module_id),
                        slot.as_str(),
                        reason.as_deref().unwrap_or("")
                    ],
                );
            }
            ClientOutcome::MotionCorrection {
                ship_id,
                position,
                velocity,
                tick,
            } => {
                let mut payload = Dict::new();
                payload.set("ship_id", godot_i64(*ship_id));
                payload.set("position", &position_to_dict(position));
                payload.set("velocity", &velocity_to_dict(velocity));
                payload.set("tick", godot_i64(*tick));
                target.call("_accept_motion_correction", vslice![payload]);
            }
            ClientOutcome::MarketSnapshot(snapshot) => {
                target.call(
                    "_accept_market_snapshot",
                    vslice![market_snapshot_to_dict(snapshot)],
                );
            }
        }
        true
    }
}

/// One typed world/event outcome.
#[derive(GodotClass)]
#[class(no_init)]
pub struct ServerEventOutcome {
    outcome: ClientEventOutcome,
}

#[godot_api]
impl ServerEventOutcome {
    /// Dispatch to the established world handler for this concrete event.
    /// GDScript never reads a variant-name string.
    #[func]
    fn dispatch(&self, mut target: Gd<Object>) -> bool {
        if let Some(method) = event_outcome_handler(&self.outcome) {
            if !ensure_handler(&mut target, method) {
                return false;
            }
        }
        match &self.outcome {
            ClientEventOutcome::Domain(event) => dispatch_domain_event(&mut target, event),
            ClientEventOutcome::AoiEnter(ship) => {
                let mut payload = Dict::new();
                payload.set("ship", &ship_state_to_dict(ship));
                call_dict(&mut target, "_handle_aoi_enter", payload);
            }
            ClientEventOutcome::AoiLeave { ship_id } => {
                let mut payload = Dict::new();
                payload.set("ship_id", godot_i64(*ship_id));
                call_dict(&mut target, "_handle_aoi_leave", payload);
            }
            ClientEventOutcome::PositionSnap { ship_id, position } => {
                let mut payload = Dict::new();
                payload.set("ship_id", godot_i64(*ship_id));
                payload.set("position", &position_to_dict(position));
                call_dict(&mut target, "_handle_position_snap", payload);
            }
        }
        true
    }
}

fn server_outcome_handler(outcome: &ClientOutcome) -> &'static str {
    match outcome {
        ClientOutcome::Welcome { .. } => "_accept_welcome",
        ClientOutcome::Redirect { .. } => "_accept_redirect",
        ClientOutcome::Event(_) => "_accept_event",
        ClientOutcome::PlayerLoadout => "_accept_player_loadout",
        ClientOutcome::InitialState(_) => "_accept_initial_state",
        ClientOutcome::ModuleActivated { .. } => "_accept_module_activated",
        ClientOutcome::ModuleDeactivated { .. } => "_accept_module_deactivated",
        ClientOutcome::MotionCorrection { .. } => "_accept_motion_correction",
        ClientOutcome::MarketSnapshot(_) => "_accept_market_snapshot",
    }
}

fn event_outcome_handler(outcome: &ClientEventOutcome) -> Option<&'static str> {
    match outcome {
        ClientEventOutcome::Domain(event) => domain_event_handler(event),
        ClientEventOutcome::AoiEnter(_) => Some("_handle_aoi_enter"),
        ClientEventOutcome::AoiLeave { .. } => Some("_handle_aoi_leave"),
        ClientEventOutcome::PositionSnap { .. } => Some("_handle_position_snap"),
    }
}

fn domain_event_handler(event: &EventWire) -> Option<&'static str> {
    Some(match event {
        EventWire::ShipSpawned { .. } => "_handle_ship_spawned",
        EventWire::VelocityChanged { .. } => "_handle_velocity_changed",
        EventWire::ShipDespawned { .. } => "_handle_ship_despawned",
        EventWire::ShipDocked { .. } => "_handle_ship_docked",
        EventWire::ShipUndocked { .. } => "_handle_ship_undocked",
        EventWire::ShipAssembled { .. } => return None,
        EventWire::DamageTaken { .. } => "_handle_damage_taken",
        EventWire::RepairApplied { .. } => "_handle_repair_applied",
        EventWire::ShipDestroyed { .. } => "_handle_ship_destroyed",
        EventWire::TargetLocked { .. } => "_handle_target_locked",
        EventWire::LockLost { .. } => "_handle_lock_lost",
        EventWire::ModuleActivated { .. } => "_on_module_activated",
        EventWire::ModuleDeactivated { .. } => "_on_module_deactivated",
        EventWire::JumpGateUsed { .. } => "_handle_jump_gate_used",
        EventWire::StarSystemChanged { .. } => "_handle_star_system_changed",
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

fn dispatch_domain_event(target: &mut Gd<Object>, event: &EventWire) {
    match event {
        EventWire::ShipSpawned {
            ship_id,
            position,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("position", &position_to_dict(position));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_ship_spawned", payload);
        }
        EventWire::VelocityChanged {
            ship_id,
            velocity,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("velocity", &velocity_to_dict(velocity));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_velocity_changed", payload);
        }
        EventWire::ShipDespawned { ship_id, tick } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_ship_despawned", payload);
        }
        EventWire::ShipDocked {
            ship_id,
            station_id,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("station_id", i64::from(*station_id));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_ship_docked", payload);
        }
        EventWire::ShipUndocked {
            ship_id,
            station_id,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("station_id", i64::from(*station_id));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_ship_undocked", payload);
        }
        // Existing GDScript intentionally has no direct ShipAssembled handler.
        // The following PlayerLoadout refresh owns the visible roster update.
        EventWire::ShipAssembled { .. } => {}
        EventWire::DamageTaken {
            ship_id,
            damage,
            current_shield,
            current_armor,
            current_hull,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("damage", f64::from(*damage));
            payload.set("current_shield", f64::from(*current_shield));
            payload.set("current_armor", f64::from(*current_armor));
            payload.set("current_hull", f64::from(*current_hull));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_damage_taken", payload);
        }
        EventWire::RepairApplied {
            ship_id,
            amount,
            layer,
            current_shield,
            current_armor,
            current_hull,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("amount", f64::from(*amount));
            payload.set("layer", layer.as_str());
            payload.set("current_shield", f64::from(*current_shield));
            payload.set("current_armor", f64::from(*current_armor));
            payload.set("current_hull", f64::from(*current_hull));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_repair_applied", payload);
        }
        EventWire::ShipDestroyed {
            ship_id,
            killer_id,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("killer_id", godot_i64(*killer_id));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_ship_destroyed", payload);
        }
        EventWire::TargetLocked {
            locker_id,
            target_id,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("locker_id", godot_i64(*locker_id));
            payload.set("target_id", godot_i64(*target_id));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_target_locked", payload);
        }
        EventWire::LockLost {
            locker_id,
            target_id,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("locker_id", godot_i64(*locker_id));
            payload.set("target_id", godot_i64(*target_id));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_lock_lost", payload);
        }
        // These normally project to dedicated top-level outcomes before a
        // ServerEventOutcome is built. Keep the exhaustive fallback safe.
        EventWire::ModuleActivated {
            ship_id,
            module_id,
            slot,
            ..
        } => {
            target.call(
                "_on_module_activated",
                vslice![godot_i64(*ship_id), i64::from(*module_id), slot.as_str()],
            );
        }
        EventWire::ModuleDeactivated {
            ship_id,
            module_id,
            slot,
            reason,
            ..
        } => {
            target.call(
                "_on_module_deactivated",
                vslice![
                    godot_i64(*ship_id),
                    i64::from(*module_id),
                    slot.as_str(),
                    reason.as_deref().unwrap_or("")
                ],
            );
        }
        EventWire::JumpGateUsed {
            ship_id,
            gate_id,
            from_sector,
            to_sector,
            entry_pos,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("gate_id", i64::from(*gate_id));
            payload.set("from_sector", i64::from(*from_sector));
            payload.set("to_sector", i64::from(*to_sector));
            payload.set("entry_pos", &position_to_dict(entry_pos));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_jump_gate_used", payload);
        }
        EventWire::StarSystemChanged {
            ship_id,
            from_system,
            to_system,
            tick,
        } => {
            let mut payload = Dict::new();
            payload.set("ship_id", godot_i64(*ship_id));
            payload.set("from_system", i64::from(*from_system));
            payload.set("to_system", i64::from(*to_system));
            payload.set("tick", godot_i64(*tick));
            call_dict(target, "_handle_star_system_changed", payload);
        }
    }
}

fn initial_state_to_dict(state: &InitialStateWire) -> Dict {
    let mut payload = Dict::new();

    let mut ships = Array::<Variant>::new();
    for ship in &state.ships {
        ships.push(&Variant::from(ship_state_to_dict(ship)));
    }
    payload.set("ships", &ships);
    payload.set("system_name", state.system_name.as_str());

    let mut systems = Array::<Variant>::new();
    for system in &state.systems {
        let mut item = Dict::new();
        item.set("id", i64::from(system.id));
        item.set("name", system.name.as_str());
        systems.push(&Variant::from(item));
    }
    payload.set("systems", &systems);

    let mut gates = Array::<Variant>::new();
    for gate in &state.jump_gates {
        let mut item = Dict::new();
        item.set("gate_id", i64::from(gate.gate_id));
        item.set("position", &position_to_dict(&gate.position));
        item.set("activation_radius", gate.activation_radius);
        item.set("to_system_name", gate.to_system_name.as_str());
        gates.push(&Variant::from(item));
    }
    payload.set("jump_gates", &gates);

    let mut stations = Array::<Variant>::new();
    for station in &state.stations {
        let mut item = Dict::new();
        item.set("station_id", i64::from(station.station_id));
        item.set("name", station.name.as_str());
        item.set("position", &position_to_dict(&station.position));
        item.set("docking_radius", station.docking_radius);
        stations.push(&Variant::from(item));
    }
    payload.set("stations", &stations);

    let mut bodies = Array::<Variant>::new();
    for body in &state.celestial_bodies {
        let mut item = Dict::new();
        item.set("id", i64::from(body.id));
        let kind = format!("{:?}", body.kind);
        item.set("kind", kind.as_str());
        item.set("name", body.name.as_str());
        item.set("position", &position_to_dict(&body.position));
        item.set("radius", body.radius);
        item.set("spectral_type", f64::from(body.spectral_type));
        bodies.push(&Variant::from(item));
    }
    payload.set("celestial_bodies", &bodies);

    let mut buildable = Array::<Variant>::new();
    for ship_type in &state.buildable_ship_types {
        let mut item = Dict::new();
        item.set("ship_type_id", i64::from(ship_type.ship_type_id));
        item.set("name", ship_type.name.as_str());
        buildable.push(&Variant::from(item));
    }
    payload.set("buildable_ship_types", &buildable);

    payload
}

fn ship_state_to_dict(ship: &ShipStateWire) -> Dict {
    let mut payload = Dict::new();
    payload.set("ship_id", godot_i64(ship.ship_id));
    payload.set("ship_type_name", ship.ship_type_name.as_str());
    payload.set("position", &position_to_dict(&ship.position));
    payload.set("velocity", &velocity_to_dict(&ship.velocity));
    payload.set("max_speed", ship.max_speed);
    payload.set("mass", ship.mass);
    payload.set("inertia_modifier", ship.inertia_modifier);
    payload.set("max_shield", f64::from(ship.max_shield));
    payload.set("max_armor", f64::from(ship.max_armor));
    payload.set("max_hull", f64::from(ship.max_hull));
    payload.set("current_shield", f64::from(ship.current_shield));
    payload.set("current_armor", f64::from(ship.current_armor));
    payload.set("current_hull", f64::from(ship.current_hull));
    payload.set("cap_max", f64::from(ship.cap_max));
    payload.set(
        "cap_recharge_per_tick",
        f64::from(ship.cap_recharge_per_tick),
    );
    payload.set("is_player", ship.is_player);
    payload
}

fn market_snapshot_to_dict(snapshot: &MarketSnapshotWire) -> Dict {
    let mut payload = Dict::new();
    payload.set("balance", godot_i64(snapshot.balance));
    payload.set("notice", snapshot.notice.as_str());

    let mut orders = Array::<Variant>::new();
    for order in &snapshot.orders {
        let mut item = Dict::new();
        item.set("order_id", godot_i64(order.order_id));
        item.set("item_id", &item_to_variant(&order.item_id));
        item.set("side", order.side.as_str());
        item.set("price", godot_i64(order.price));
        item.set("quantity", godot_i64(order.quantity));
        item.set("is_own", order.is_own);
        orders.push(&Variant::from(item));
    }
    payload.set("orders", &orders);
    payload
}

fn item_to_variant(item: &ItemWire) -> Variant {
    match item {
        ItemWire::ScrapMetal => Variant::from("ScrapMetal"),
        ItemWire::Module { module_id } => {
            let mut fields = Dict::new();
            fields.set("module_id", i64::from(*module_id));
            let mut tagged = Dict::new();
            tagged.set("Module", &fields);
            Variant::from(tagged)
        }
        ItemWire::PackagedShip { ship_type_id } => {
            let mut fields = Dict::new();
            fields.set("ship_type_id", i64::from(*ship_type_id));
            let mut tagged = Dict::new();
            tagged.set("PackagedShip", &fields);
            Variant::from(tagged)
        }
    }
}

fn position_to_dict(position: &AbsPosWire) -> Dict {
    let mut value = Dict::new();
    value.set("x", position.x);
    value.set("y", position.y);
    value.set("z", position.z);
    value
}

fn velocity_to_dict(velocity: &VelWire) -> Dict {
    let mut value = Dict::new();
    value.set("dx", velocity.dx);
    value.set("dy", velocity.dy);
    value.set("dz", velocity.dz);
    value
}

fn call_dict(target: &mut Gd<Object>, method: &str, payload: Dict) {
    target.call(method, vslice![payload]);
}

fn godot_i64(value: u64) -> i64 {
    i64::try_from(value).expect("ClientOutcome validates every Godot-facing u64")
}
