use dawn_core::ItemId;
use dawn_wire::{EventWire, ItemWire, PlayerLoadoutWire, ServerMessage};

/// Decode one postcard frame and reject values that cannot cross the Godot
/// boundary without narrowing or losing canonical item identity.
pub(crate) fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage, String> {
    let message = ServerMessage::decode(bytes).map_err(|error| error.to_string())?;
    validate_godot_integer_range(&message)?;
    Ok(message)
}

fn ensure_godot_int(value: u64, field: &str) -> Result<(), String> {
    i64::try_from(value)
        .map(|_| ())
        .map_err(|_| format!("{field}={value} exceeds Godot's signed 64-bit integer range"))
}

fn ensure_client_u32(value: u64, field: &str) -> Result<(), String> {
    u32::try_from(value)
        .map(|_| ())
        .map_err(|_| format!("{field}={value} exceeds the client-side u32 range"))
}

fn ensure_canonical_item(item: ItemWire, field: &str) -> Result<(), String> {
    ItemId::try_from(item)
        .map(|_| ())
        .map_err(|error| format!("{field} has invalid canonical Item identity: {error:?}"))
}

fn validate_event(event: &EventWire) -> Result<(), String> {
    match event {
        EventWire::ShipSpawned { ship_id, tick, .. }
        | EventWire::VelocityChanged { ship_id, tick, .. }
        | EventWire::ShipDespawned { ship_id, tick }
        | EventWire::ShipDocked { ship_id, tick, .. }
        | EventWire::ShipUndocked { ship_id, tick, .. }
        | EventWire::ShipAssembled { ship_id, tick, .. }
        | EventWire::DamageTaken { ship_id, tick, .. }
        | EventWire::RepairApplied { ship_id, tick, .. }
        | EventWire::ModuleDeactivated { ship_id, tick, .. }
        | EventWire::JumpGateUsed { ship_id, tick, .. }
        | EventWire::StarSystemChanged { ship_id, tick, .. } => {
            ensure_godot_int(*ship_id, "event.ship_id")?;
            ensure_godot_int(*tick, "event.tick")?;
        }
        EventWire::ShipDestroyed {
            ship_id,
            killer_id,
            tick,
        } => {
            ensure_godot_int(*ship_id, "event.ship_id")?;
            ensure_godot_int(*killer_id, "event.killer_id")?;
            ensure_godot_int(*tick, "event.tick")?;
        }
        EventWire::TargetLocked {
            locker_id,
            target_id,
            tick,
        }
        | EventWire::LockLost {
            locker_id,
            target_id,
            tick,
        } => {
            ensure_godot_int(*locker_id, "event.locker_id")?;
            ensure_godot_int(*target_id, "event.target_id")?;
            ensure_godot_int(*tick, "event.tick")?;
        }
        EventWire::ModuleActivated {
            ship_id,
            target_ship_id,
            tick,
            ..
        } => {
            ensure_godot_int(*ship_id, "event.ship_id")?;
            if let Some(target_ship_id) = target_ship_id {
                ensure_godot_int(*target_ship_id, "event.target_ship_id")?;
            }
            ensure_godot_int(*tick, "event.tick")?;
        }
    }
    Ok(())
}

pub(crate) fn validate_player_loadout_godot_ranges(
    loadout: &PlayerLoadoutWire,
) -> Result<(), String> {
    ensure_godot_int(loadout.tick, "player_loadout.tick")?;
    if let Some(active_ship_id) = loadout.active_ship_id {
        ensure_godot_int(active_ship_id, "player_loadout.active_ship_id")?;
    }
    for ship in &loadout.owned_ships {
        ensure_godot_int(ship.ship_id, "player_loadout.owned_ships.ship_id")?;
    }
    for module in &loadout.modules {
        ensure_client_u32(
            module.cycle_time_ticks,
            "player_loadout.modules.cycle_time_ticks",
        )?;
    }
    for item in loadout.inventory.iter().chain(&loadout.station_inventory) {
        ensure_canonical_item(item.item_id, "player_loadout.inventory.item_id")?;
        ensure_godot_int(item.count, "player_loadout.inventory.count")?;
    }
    Ok(())
}

fn validate_godot_integer_range(message: &ServerMessage) -> Result<(), String> {
    match message {
        ServerMessage::Welcome { player_id, ship_id }
        | ServerMessage::Redirect {
            player_id, ship_id, ..
        } => {
            ensure_godot_int(*player_id, "player_id")?;
            ensure_godot_int(*ship_id, "ship_id")?;
        }
        ServerMessage::Event(event) => validate_event(event)?,
        ServerMessage::PlayerLoadout(loadout) => validate_player_loadout_godot_ranges(loadout)?,
        ServerMessage::InitialState(state) => {
            for ship in &state.ships {
                ensure_godot_int(ship.ship_id, "initial_state.ship_id")?;
            }
        }
        ServerMessage::AoiEnter(ship) => {
            ensure_godot_int(ship.ship_id, "aoi_enter.ship_id")?;
        }
        ServerMessage::AoiLeave { ship_id } | ServerMessage::PositionSnap { ship_id, .. } => {
            ensure_godot_int(*ship_id, "ship_id")?;
        }
        ServerMessage::MotionCorrection { ship_id, tick, .. } => {
            ensure_godot_int(*ship_id, "motion_correction.ship_id")?;
            ensure_godot_int(*tick, "motion_correction.tick")?;
        }
        ServerMessage::MarketSnapshot(snapshot) => {
            ensure_godot_int(snapshot.balance, "market.balance")?;
            for order in &snapshot.orders {
                ensure_canonical_item(order.item_id, "market.item_id")?;
                ensure_godot_int(order.order_id, "market.order_id")?;
                ensure_godot_int(order.price, "market.price")?;
                ensure_godot_int(order.quantity, "market.quantity")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frame_decodes_without_a_second_message_mirror() {
        let decoded = decode_server_message(
            &ServerMessage::Welcome {
                player_id: 5,
                ship_id: 11,
            }
            .encode(),
        )
        .unwrap();
        assert!(matches!(
            decoded,
            ServerMessage::Welcome {
                player_id: 5,
                ship_id: 11
            }
        ));
    }

    #[test]
    fn unsigned_ids_outside_godot_int_range_are_rejected() {
        let error = decode_server_message(
            &ServerMessage::Welcome {
                player_id: 1,
                ship_id: (i64::MAX as u64) + 1,
            }
            .encode(),
        )
        .unwrap_err();
        assert!(error.contains("ship_id"));
    }
}
