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

## Column codes are used only by Godot hit-testing. Row meaning itself lives in
## the typed `StationInventoryRow` GDExtension object.
const SOURCE_NONE := -1
const SOURCE_FITTED := 0
const SOURCE_SHIP_CARGO := 1
const SOURCE_STATION := 2
const SOURCE_SHIPS := 3

var panel: Control = null
## Typed station policy input. It is created by HudManager while rendering and
## consumed by main.gd only through StationInventoryInteraction.
var action: StationInventoryRow = null
var item_id: ItemIdentity = null
var count: int = 0
var source: int = SOURCE_NONE
var ship_id: int = 0
## Position within this module's own slot kind (ModuleRow.index / the
## server's per-slot-kind array position, not a global row index).
var slot_index: int = 0


## FITTED/SHIP CARGO/STATION rows. Non-inventory action rows pass `null` for
## `item_id`; actual cargo/station stacks pass the canonical typed identity.
static func for_item(
	p_panel: Control, p_action: StationInventoryRow, p_item_id: ItemIdentity = null,
	p_count: int = 0, p_source: int = SOURCE_NONE, p_slot_index: int = 0
) -> Variant:
	var row = new()
	row.panel = p_panel
	row.action = p_action
	row.item_id = p_item_id
	row.count = p_count
	row.source = p_source
	row.slot_index = p_slot_index
	return row


## SHIPS roster row (ADR-0037). Only `ship_id`/`action` are meaningful.
static func for_ship(p_panel: Control, p_ship_id: int, p_action: StationInventoryRow) -> Variant:
	var row = new()
	row.panel = p_panel
	row.ship_id = p_ship_id
	row.action = p_action
	row.source = SOURCE_SHIPS
	return row
