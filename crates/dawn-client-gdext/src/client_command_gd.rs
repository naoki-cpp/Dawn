use dawn_wire::{ClientCommandJson, PosJson, WarpTargetJson};
use godot::prelude::*;

fn to_json_line(cmd: ClientCommandJson) -> GString {
    (&serde_json::to_string(&cmd).unwrap_or_default()).into()
}

/// Range/radius sentinel used throughout `connection.gd`'s public API:
/// `<= 0.0` means "no override, let the server pick its default"
/// (ADR-0031). Kept as a free function so every `*_command` below applies
/// the same rule instead of repeating the comparison.
fn positive_or_none(value: f32) -> Option<f32> {
    if value > 0.0 {
        Some(value)
    } else {
        None
    }
}

/// Ship-id sentinel used by `ActivateModuleCommand`'s optional target:
/// `< 0` means "no target" (ADR-0035).
fn non_negative_or_none(value: i64) -> Option<u64> {
    if value >= 0 {
        Some(value as u64)
    } else {
        None
    }
}

/// Builds the client -> server wire JSON line for every command the Godot
/// client can send (ADR-0041). Each method mirrors one of the old
/// `connection.gd::send_*_command` functions' argument shape and returns
/// the exact line `connection.gd` should hand to `WebSocketPeer.send_text`
/// (already `"type"`-tagged, no further assembly needed) -- replacing the
/// old pattern of hand-building a matching `Dictionary` + `JSON.stringify`
/// per command, which had no compile-time check against
/// [`dawn_wire::ClientCommandJson`]'s actual field names/shape.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientCommand {}

#[godot_api]
impl ClientCommand {
    #[func]
    fn move_command(&self, x: f32, y: f32, z: f32) -> GString {
        to_json_line(ClientCommandJson::MoveCommand {
            target: PosJson { x, y, z },
        })
    }

    #[func]
    fn lock_on_command(&self, target_id: i64) -> GString {
        to_json_line(ClientCommandJson::LockOnCommand {
            target_id: target_id as u64,
        })
    }

    /// `target_ship_id < 0` means no target (ADR-0035) -- required only for
    /// targeted module kinds (Weapon/Tackle), validated server-side.
    #[func]
    fn activate_module_command(
        &self,
        module_id: i64,
        slot: GString,
        target_ship_id: i64,
    ) -> GString {
        to_json_line(ClientCommandJson::ActivateModuleCommand {
            module_id: module_id as u32,
            slot: slot.to_string(),
            target_ship_id: non_negative_or_none(target_ship_id),
        })
    }

    #[func]
    fn deactivate_module_command(&self, module_id: i64, slot: GString) -> GString {
        to_json_line(ClientCommandJson::DeactivateModuleCommand {
            module_id: module_id as u32,
            slot: slot.to_string(),
        })
    }

    #[func]
    fn stop_command(&self) -> GString {
        to_json_line(ClientCommandJson::StopCommand {})
    }

    #[func]
    fn jump_command(&self, gate_id: i64) -> GString {
        to_json_line(ClientCommandJson::JumpCommand {
            gate_id: gate_id as u32,
        })
    }

    #[func]
    fn approach_command(&self, target_id: i64) -> GString {
        to_json_line(ClientCommandJson::ApproachCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
        })
    }

    #[func]
    fn approach_gate_command(&self, gate_id: i64) -> GString {
        to_json_line(ClientCommandJson::ApproachCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
        })
    }

    #[func]
    fn warp_command(&self, gate_id: i64) -> GString {
        to_json_line(ClientCommandJson::WarpCommand {
            target: Some(WarpTargetJson::Gate(gate_id as u32)),
            gate_id: None,
        })
    }

    #[func]
    fn warp_to_body_command(&self, body_id: i64) -> GString {
        to_json_line(ClientCommandJson::WarpCommand {
            target: Some(WarpTargetJson::Body(body_id as u32)),
            gate_id: None,
        })
    }

    /// `range_m <= 0.0` falls back to the server-side default (weapon
    /// range, ADR-0031).
    #[func]
    fn orbit_command(&self, target_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::OrbitCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
            radius: positive_or_none(range_m),
        })
    }

    #[func]
    fn orbit_gate_command(&self, gate_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::OrbitCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
            radius: positive_or_none(range_m),
        })
    }

    #[func]
    fn keep_at_range_command(&self, target_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::KeepAtRangeCommand {
            gate_id: None,
            target_id: Some(target_id as u64),
            range: positive_or_none(range_m),
        })
    }

    #[func]
    fn keep_at_range_gate_command(&self, gate_id: i64, range_m: f32) -> GString {
        to_json_line(ClientCommandJson::KeepAtRangeCommand {
            gate_id: Some(gate_id as u32),
            target_id: None,
            range: positive_or_none(range_m),
        })
    }

    #[func]
    fn fit_module_command(&self, ship_id: i64, module_id: i64, slot: GString) -> GString {
        to_json_line(ClientCommandJson::FitModuleCommand {
            ship_id: ship_id as u64,
            module_id: module_id as u32,
            slot: slot.to_string(),
        })
    }

    #[func]
    fn unfit_module_command(&self, ship_id: i64, module_id: i64, slot: GString) -> GString {
        to_json_line(ClientCommandJson::UnfitModuleCommand {
            ship_id: ship_id as u64,
            module_id: module_id as u32,
            slot: slot.to_string(),
        })
    }

    #[func]
    fn reorder_fitted_module_command(
        &self,
        ship_id: i64,
        slot: GString,
        from_index: i64,
        to_index: i64,
    ) -> GString {
        to_json_line(ClientCommandJson::ReorderFittedModuleCommand {
            ship_id: ship_id as u64,
            slot: slot.to_string(),
            from_index: from_index as u32,
            to_index: to_index as u32,
        })
    }

    #[func]
    fn dock_command(&self, station_id: i64) -> GString {
        to_json_line(ClientCommandJson::DockCommand {
            station_id: station_id as u32,
        })
    }

    #[func]
    fn undock_command(&self) -> GString {
        to_json_line(ClientCommandJson::UndockCommand {})
    }

    #[func]
    fn build_packaged_ship_command(
        &self,
        ship_id: i64,
        station_id: i64,
        ship_type_id: i64,
    ) -> GString {
        to_json_line(ClientCommandJson::BuildPackagedShipCommand {
            ship_id: ship_id as u64,
            station_id: station_id as u32,
            ship_type_id: ship_type_id as u32,
        })
    }

    #[func]
    fn disassemble_ship_command(&self, ship_id: i64, station_id: i64) -> GString {
        to_json_line(ClientCommandJson::DisassembleShipCommand {
            ship_id: ship_id as u64,
            station_id: station_id as u32,
        })
    }

    #[func]
    fn assemble_command(&self, station_id: i64, ship_type_id: i64) -> GString {
        to_json_line(ClientCommandJson::AssembleCommand {
            station_id: station_id as u32,
            ship_type_id: ship_type_id as u32,
        })
    }

    #[func]
    fn disembark_command(&self) -> GString {
        to_json_line(ClientCommandJson::DisembarkCommand {})
    }

    #[func]
    fn select_active_ship_command(&self, ship_id: i64) -> GString {
        to_json_line(ClientCommandJson::SelectActiveShipCommand {
            ship_id: ship_id as u64,
        })
    }

    /// Move the entire stack of an item out of a docked ship's own cargo
    /// into the caller's station inventory (ADR-0034 9B). `item_type` is
    /// one of `"Module"`/`"PackagedShip"`/`"ScrapMetal"` (matches
    /// `ItemRow.item_type`); `module_id`/`ship_type_id` are only meaningful
    /// for the matching variant (`0` otherwise).
    #[func]
    fn transfer_to_station_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
    ) -> GString {
        self.transfer_command(
            ship_id,
            station_id,
            item_type,
            module_id,
            ship_type_id,
            "ToStation",
        )
    }

    /// The reverse of `transfer_to_station_command`: move the entire stack
    /// back into the docked ship's own cargo.
    #[func]
    fn transfer_from_station_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
    ) -> GString {
        self.transfer_command(
            ship_id,
            station_id,
            item_type,
            module_id,
            ship_type_id,
            "ToShip",
        )
    }
}

impl ClientCommand {
    fn transfer_command(
        &self,
        ship_id: i64,
        station_id: i64,
        item_type: GString,
        module_id: i64,
        ship_type_id: i64,
        direction: &str,
    ) -> GString {
        to_json_line(ClientCommandJson::TransferToStationCommand {
            ship_id: ship_id as u64,
            station_id: station_id as u32,
            item_type: item_type.to_string(),
            module_id: module_id as u32,
            ship_type_id: ship_type_id as u32,
            direction: direction.to_string(),
        })
    }
}
