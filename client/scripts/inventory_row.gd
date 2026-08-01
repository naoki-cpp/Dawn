## inventory_row.gd
##
## Typed shape for one row of the HUD inventory panel (FITTED/SHIP CARGO/
## STATION/SHIPS columns), replacing the bare Dictionary `hud_manager.gd`
## used to hand back to `main.gd` (architecture-review/client.md C-8).
## Unlike `ModuleRow`/`ItemRow` (GDExtension classes for typed PlayerLoadout
## wire rows, dawn-client-gdext/ADR-0039/ADR-0040), this is not wire-sourced
## -- it wraps a UI `Panel` reference plus whichever of the fields below a
## given row kind (module fit/unfit, ship cargo, station inventory, ship
## roster) actually uses; unused fields keep their default.
##
## Item-bearing rows retain the canonical `ItemIdentity` object. They never
## flatten it back into `item_type` plus mutually exclusive numeric sentinels.
extends RefCounted

## action vocabulary -- named so a typo is an unknown-identifier error at
## parse time instead of a silently-never-matching string literal.
const ACTION_NONE := ""
const ACTION_FIT := "fit"
const ACTION_UNFIT := "unfit"
## Unfits every currently-fitted module in one click (e.g. to clear the way
## for Disassemble, which requires a fully unfitted ship).
const ACTION_UNFIT_ALL := "unfit_all"
const ACTION_ASSEMBLE := "assemble"
const ACTION_SELECT_ACTIVE_SHIP := "select_active_ship"
const ACTION_DISASSEMBLE := "disassemble"
## Toggles the Build ship-type picker open/closed (Phase 9B task 10).
const ACTION_BUILD_TOGGLE := "build_toggle"
## One picker sub-row for a specific buildable ship type; `ship_type_id`
## carries which one. Distinct from ACTION_BUILD_TOGGLE so main.gd can tell
## "open the picker" apart from "build this type" with the same action prefix.
const ACTION_BUILD_SHIP_TYPE := "build_ship_type"

## `source` vocabulary -- tags which of the four inventory-panel columns a
## row belongs to.
const SOURCE_NONE := ""
const SOURCE_SHIP_CARGO := "ship_cargo"
const SOURCE_STATION := "station"
const SOURCE_FITTED := "fitted"
const SOURCE_SHIPS := "ships"

var panel: Control = null
## Fitted-module and fit/unfit action payload. For an inventory Module row this
## is derived from `item_id`; for a fitted row it comes from `ModuleRow`.
var module_id: int = 0
var slot: String = ""
var action: String = ACTION_NONE
## Build/assemble action payload. For a PackagedShip row this is derived from
## `item_id`; build-picker rows carry a ship type without being inventory.
var ship_type_id: int = 0
var item_id: ItemIdentity = null
var count: int = 0
var source: String = SOURCE_NONE
var ship_id: int = 0
## Position within this module's own slot kind (ModuleRow.index / the
## server's per-slot-kind array position, not a global row index).
var slot_index: int = 0


## FITTED/SHIP CARGO/STATION rows. Non-inventory action rows pass `null` for
## `item_id`; actual cargo/station stacks pass the canonical typed identity.
static func for_item(
	panel: Control, module_id: int, slot: String, action: String, ship_type_id: int = 0,
	item_id: ItemIdentity = null, count: int = 0, source: String = SOURCE_NONE,
	slot_index: int = 0
) -> Variant:
	var row = new()
	row.panel = panel
	row.module_id = module_id
	row.slot = slot
	row.action = action
	row.ship_type_id = ship_type_id
	row.item_id = item_id
	row.count = count
	row.source = source
	row.slot_index = slot_index
	return row


## SHIPS roster row (ADR-0037). Only `ship_id`/`action` are meaningful.
static func for_ship(panel: Control, ship_id: int, action: String) -> Variant:
	var row = new()
	row.panel = panel
	row.ship_id = ship_id
	row.action = action
	row.source = SOURCE_SHIPS
	return row
