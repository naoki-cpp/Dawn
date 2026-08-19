## hud_hit_test.gd
##
## Hit-testing for the HUD panels HudManager builds: given a screen position,
## answers "what's under the cursor" (a module slot, an inventory row, a
## column, or the panel's swallow-area). Split out of hud_manager.gd
## (architecture-review/client.md C-9) -- building a Control subtree and
## testing screen positions against already-built Controls are different
## responsibilities that happened to share a file; a change to one (e.g. the
## `fitted_header.clip_text` incident, where a display-only tweak silently
## broke a hit-test that shared the same node) shouldn't require touching the
## other. Stateless static methods, same calling convention as HudManager:
## callers (hud_surface.gd) pass back the typed refs HudManager handed them
## when it built the panel.
class_name HudHitTest
extends RefCounted

## ModuleRow/ItemRow are GDExtension classes (dawn-client-gdext,
## ADR-0039/ADR-0040) -- globally registered, no preload needed.
const InventoryRow = preload("res://scripts/inventory_row.gd")


## Returns the F-key index (0-based, i.e. the position in module_slots) of
## the module slot under a screen position, or -1. Used so a click on the
## bar toggles the module instead of the world.
static func module_slot_at(module_slots: Array[HudManager.ModuleSlotRefs], pos: Vector2) -> int:
	for i: int in range(module_slots.size()):
		if module_slots[i].panel.get_global_rect().has_point(pos):
			return i
	return -1


## Returns the row under `pos` from either column, or `null` if none.
## Callers distinguish a miss from a hit with a plain `null` check instead of a
## sentinel-key lookup.
static func inventory_panel_row_at(refs: HudManager.InventoryPanelRefs, pos: Vector2) -> InventoryRow:
	if not refs.panel.visible:
		return null
	for row: InventoryRow in refs.fitted_rows:
		if row.panel.get_global_rect().has_point(pos):
			return row
	for row: InventoryRow in refs.inventory_rows:
		if row.panel.get_global_rect().has_point(pos):
			return row
	for row: InventoryRow in refs.station_rows:
		if row.panel.get_global_rect().has_point(pos):
			return row
	for row: InventoryRow in refs.ship_rows:
		if row.panel.get_global_rect().has_point(pos):
			return row
	return null


## Which of the four inventory-panel columns `pos` falls in (an
## `InventoryRow.SOURCE_*` value), or `SOURCE_NONE` if outside all of them. Used by
## the drag-and-drop drop-target resolution (main.gd) -- unlike
## inventory_panel_row_at(), this matches empty space within a column's list
## too (dropping below the last row must still count as a drop into that
## column, not a miss).
static func column_at(refs: HudManager.InventoryPanelRefs, pos: Vector2) -> int:
	if not refs.panel.visible:
		return InventoryRow.SOURCE_NONE
	if refs.fitted_col.get_global_rect().has_point(pos):
		return InventoryRow.SOURCE_FITTED
	if refs.inv_col.get_global_rect().has_point(pos):
		return InventoryRow.SOURCE_SHIP_CARGO
	if refs.station_col.get_global_rect().has_point(pos):
		return InventoryRow.SOURCE_STATION
	if refs.ships_col.get_global_rect().has_point(pos):
		return InventoryRow.SOURCE_SHIPS
	return InventoryRow.SOURCE_NONE


## True when the inventory panel is open and `pos` falls anywhere inside it
## (a row or its empty margin/header). Lets main.gd swallow the click so a
## miss on the open panel doesn't fall through to the 3D world behind it
## (thrust / select). Distinct from inventory_panel_row_at(), which only
## reports actionable row hits.
static func inventory_panel_consumes(refs: HudManager.InventoryPanelRefs, pos: Vector2) -> bool:
	return refs.panel.visible and refs.panel.get_global_rect().has_point(pos)
