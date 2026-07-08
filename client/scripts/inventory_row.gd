## inventory_row.gd
##
## Typed shape for one row of the HUD inventory panel (FITTED/SHIP CARGO/
## STATION/SHIPS columns), replacing the bare Dictionary `hud_manager.gd`
## used to hand back to `main.gd` (architecture-review-client.md C-8).
## Unlike `ModuleRow`/`ItemRow` (typed PlayerLoadout wire rows), this is not
## wire-sourced -- it wraps a UI `Panel` reference plus whichever of the
## fields below a given row kind (module fit/unfit, ship cargo, station
## inventory, ship roster) actually uses; unused fields keep their default.
##
## Note on typing: because this script has no class_name, it has no way to
## name its own type inside its own body -- see module_row.gd's header for
## the full explanation. Constructors below use bare `new()` and an untyped
## return for that reason; external callers use
## `const InventoryRow = preload(...)` + `-> InventoryRow` as normal.
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
## row belongs to. Originally only distinguished SHIP CARGO/STATION (for the
## ship-cargo right-click transfer gesture); now also covers FITTED/SHIPS so
## the drag-and-drop dispatch matrix can tell which column a drag started or
## ended in without re-deriving it from `action`.
const SOURCE_NONE := ""
const SOURCE_SHIP_CARGO := "ship_cargo"
const SOURCE_STATION := "station"
const SOURCE_FITTED := "fitted"
const SOURCE_SHIPS := "ships"

var panel: Control = null
var module_id: int = 0
var slot: String = ""
var action: String = ACTION_NONE
var ship_type_id: int = 0
var item_type: String = ""
var count: int = 0
var source: String = SOURCE_NONE
var ship_id: int = 0
## Position within this module's own slot kind (ModuleRow.index / the
## server's per-slot-kind array position, not a global row index). Only
## meaningful for FITTED rows -- drag-and-drop reorder needs it to build
## ReorderFittedModuleCommand's from_index/to_index.
var slot_index: int = 0


## FITTED/SHIP CARGO/STATION rows (module fit/unfit, ship cargo item, station
## inventory item). `ship_id` stays at its default (0) -- these rows are
## never a ship-roster row.
static func for_item(
	panel: Control, module_id: int, slot: String, action: String, ship_type_id: int = 0,
	item_type: String = "", count: int = 0, source: String = SOURCE_NONE, slot_index: int = 0
) -> Variant:
	var row = new()
	row.panel = panel
	row.module_id = module_id
	row.slot = slot
	row.action = action
	row.ship_type_id = ship_type_id
	row.item_type = item_type
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
