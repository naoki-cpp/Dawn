use dawn_client_core::{
    FittedModuleRow, StationInventoryAction as CoreAction, StationInventoryColumn,
    StationInventoryContext, StationInventoryInteraction as CoreInteraction,
    StationInventoryLocalAction, StationInventoryRow as CoreRow,
};
use dawn_core::{ModuleId, ShipTypeId};
use godot::prelude::*;

use crate::client_command_gd::{
    request_result_from_request, slot_kind_from_str, ClientCommandResult,
};
use crate::id_boundary::{ship_id_from_godot, station_id_from_godot};
use crate::item_identity_gd::ItemIdentity;
use crate::module_row_gd::ModuleRow;

const COLUMN_NONE: i64 = -1;
const COLUMN_FITTED: i64 = 0;
const COLUMN_SHIP_CARGO: i64 = 1;
const COLUMN_STATION: i64 = 2;
const COLUMN_SHIPS: i64 = 3;

/// Typed policy metadata attached to one rendered station inventory row.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct StationInventoryRow {
    row: CoreRow,
}

impl StationInventoryRow {
    fn wrap(row: CoreRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { row })
    }

    pub(crate) fn core_row(&self) -> CoreRow {
        self.row
    }
}

#[godot_api]
impl StationInventoryRow {
    #[func]
    fn none() -> Gd<Self> {
        Self::wrap(CoreRow::None)
    }

    #[func]
    fn fitted_with_index(module_id: i64, slot: GString, index: i64) -> Variant {
        let Some(module) = nonzero_module_id(module_id) else {
            return Variant::nil();
        };
        let Ok(index) = u32::try_from(index) else {
            return Variant::nil();
        };
        let Some(slot) = slot_kind_from_str(&slot.to_string()) else {
            return Variant::nil();
        };
        Self::wrap(CoreRow::Fitted(FittedModuleRow {
            module,
            slot,
            index,
        }))
        .to_variant()
    }

    #[func]
    fn unfit_all() -> Gd<Self> {
        Self::wrap(CoreRow::UnfitAll)
    }

    #[func]
    fn cargo(item_id: Gd<ItemIdentity>, slot: GString) -> Variant {
        let slot_text = slot.to_string();
        let slot = if slot_text.is_empty() {
            Some(None)
        } else {
            slot_kind_from_str(&slot_text).map(Some)
        };
        let Some(slot) = slot else {
            return Variant::nil();
        };
        Self::wrap(CoreRow::Cargo {
            item: item_id.bind().get(),
            slot,
        })
        .to_variant()
    }

    #[func]
    fn station(item_id: Gd<ItemIdentity>) -> Gd<Self> {
        Self::wrap(CoreRow::Station(item_id.bind().get()))
    }

    #[func]
    fn disassemble() -> Gd<Self> {
        Self::wrap(CoreRow::Disassemble)
    }

    #[func]
    fn build_toggle() -> Gd<Self> {
        Self::wrap(CoreRow::BuildToggle)
    }

    #[func]
    fn build_ship_type(ship_type_id: i64) -> Variant {
        let Some(ship_type) = nonzero_ship_type_id(ship_type_id) else {
            return Variant::nil();
        };
        Self::wrap(CoreRow::BuildShipType(ship_type)).to_variant()
    }

    #[func]
    fn owned_ship(raw_ship_id: i64, active: bool) -> Variant {
        let Some(ship) = ship_id_from_godot(raw_ship_id) else {
            return Variant::nil();
        };
        Self::wrap(CoreRow::OwnedShip { ship, active }).to_variant()
    }

    #[func]
    fn is_disassemble(&self) -> bool {
        matches!(self.row, CoreRow::Disassemble)
    }

    #[func]
    fn is_build_toggle(&self) -> bool {
        matches!(self.row, CoreRow::BuildToggle)
    }

    #[func]
    fn is_build_ship_type(&self) -> bool {
        matches!(self.row, CoreRow::BuildShipType(_))
    }

    #[func]
    fn is_unfit_all(&self) -> bool {
        matches!(self.row, CoreRow::UnfitAll)
    }

    #[func]
    fn is_fitted(&self) -> bool {
        matches!(self.row, CoreRow::Fitted(_))
    }

    #[func]
    fn is_cargo(&self) -> bool {
        matches!(self.row, CoreRow::Cargo { .. })
    }

    #[func]
    fn is_station_item(&self) -> bool {
        matches!(self.row, CoreRow::Station(_))
    }

    #[func]
    fn is_owned_ship_active(&self) -> bool {
        matches!(self.row, CoreRow::OwnedShip { active: true, .. })
    }

    #[func]
    fn is_owned_ship_selectable(&self) -> bool {
        matches!(self.row, CoreRow::OwnedShip { active: false, .. })
    }
}

/// Typed result of a station inventory interaction.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct StationInventoryAction {
    action: CoreAction,
}

impl StationInventoryAction {
    fn from_core(action: CoreAction) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { action })
    }
}

#[godot_api]
impl StationInventoryAction {
    #[func]
    fn is_build_picker_toggle(&self) -> bool {
        matches!(
            self.action,
            CoreAction::Local(StationInventoryLocalAction::ToggleBuildPicker)
        )
    }

    #[func]
    fn request_count(&self) -> i64 {
        i64::try_from(self.action.request_count()).expect("station action request count fits i64")
    }

    #[func]
    fn request_result_at(&self, index: i64) -> Gd<ClientCommandResult> {
        let Ok(index) = usize::try_from(index) else {
            return ClientCommandResult::failure(
                "invalid_request_index",
                "station action request index must be non-negative",
            );
        };
        self.action
            .request_at(index)
            .map(request_result_from_request)
            .unwrap_or_else(|| {
                ClientCommandResult::failure(
                    "invalid_request_index",
                    "station action has no request at this index",
                )
            })
    }
}

/// Thin GDExtension adapter around the engine-independent station policy.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct StationInventoryInteraction {
    core: CoreInteraction,
}

#[godot_api]
impl StationInventoryInteraction {
    #[func]
    fn column_none() -> i64 {
        COLUMN_NONE
    }

    #[func]
    fn column_fitted() -> i64 {
        COLUMN_FITTED
    }

    #[func]
    fn column_ship_cargo() -> i64 {
        COLUMN_SHIP_CARGO
    }

    #[func]
    fn column_station() -> i64 {
        COLUMN_STATION
    }

    #[func]
    fn column_ships() -> i64 {
        COLUMN_SHIPS
    }

    #[func]
    fn click(
        &self,
        row: Gd<StationInventoryRow>,
        active_ship_id: i64,
        docked_station_id: i64,
        fitted_modules: Array<Gd<ModuleRow>>,
    ) -> Gd<StationInventoryAction> {
        let row = row.bind().core_row();
        let fitted = if matches!(row, CoreRow::UnfitAll) {
            let Some(fitted) = fitted_modules_from_godot(&fitted_modules) else {
                return StationInventoryAction::from_core(CoreAction::None);
            };
            fitted
        } else {
            Vec::new()
        };
        StationInventoryAction::from_core(
            self.core
                .click(row, context(active_ship_id, docked_station_id, &fitted)),
        )
    }

    #[func]
    fn resolve_drop(
        &self,
        row: Gd<StationInventoryRow>,
        target_column: i64,
        target_row: Gd<StationInventoryRow>,
        active_ship_id: i64,
        docked_station_id: i64,
    ) -> Gd<StationInventoryAction> {
        let Some(target_column) = column_from_code(target_column) else {
            return StationInventoryAction::from_core(CoreAction::None);
        };
        StationInventoryAction::from_core(self.core.drop(
            row.bind().core_row(),
            target_column,
            target_row.bind().core_row(),
            context(active_ship_id, docked_station_id, &[]),
        ))
    }
}

fn context<'a>(
    active_ship_id: i64,
    docked_station_id: i64,
    fitted_modules: &'a [FittedModuleRow],
) -> StationInventoryContext<'a> {
    StationInventoryContext::new(
        ship_id_from_godot(active_ship_id),
        station_id_from_godot(docked_station_id),
        fitted_modules,
    )
}

fn fitted_modules_from_godot(rows: &Array<Gd<ModuleRow>>) -> Option<Vec<FittedModuleRow>> {
    rows.iter_shared()
        .map(|row| {
            let inner = row.bind().inner_clone();
            let module = nonzero_module_id(i64::from(inner.module_id.0))?;
            let slot = slot_kind_from_str(&inner.slot)?;
            Some(FittedModuleRow {
                module,
                slot,
                index: inner.index,
            })
        })
        .collect()
}

fn column_from_code(code: i64) -> Option<StationInventoryColumn> {
    match code {
        COLUMN_FITTED => Some(StationInventoryColumn::Fitted),
        COLUMN_SHIP_CARGO => Some(StationInventoryColumn::ShipCargo),
        COLUMN_STATION => Some(StationInventoryColumn::Station),
        COLUMN_SHIPS => Some(StationInventoryColumn::Ships),
        _ => None,
    }
}

fn nonzero_module_id(value: i64) -> Option<ModuleId> {
    u32::try_from(value)
        .ok()
        .filter(|id| *id != 0)
        .map(ModuleId)
}

fn nonzero_ship_type_id(value: i64) -> Option<ShipTypeId> {
    u32::try_from(value)
        .ok()
        .filter(|id| *id != 0)
        .map(ShipTypeId)
}
