## inventory_row.gd
##
## Typed shape for one row of the HUD inventory panel (FITTED/SHIP CARGO/
## STATION/SHIPS columns), replacing the bare Dictionary `hud_manager.gd`
## used to hand back to `main.gd` (architecture-review/client.md C-8).
## Unlike `ModuleRow`/`ItemRow` (GDExtension classes for typed PlayerLoadout
## wire rows, dawn-client-gdext/ADR-0039/ADR-0040), this is not wire-sourced.
## It pairs a rendered `Panel` with the canonical typed policy row and does not
## mirror the row's IDs, item identity, column, or fitted index in GDScript.
extends RefCounted

var panel: Control = null
## Typed station policy input. It is created by HudManager while rendering and
## consumed by main.gd only through StationInventoryInteraction.
var policy_row: StationInventoryRow = null


static func create(p_panel: Control, p_policy_row: StationInventoryRow) -> Variant:
	var row = new()
	row.panel = p_panel
	row.policy_row = p_policy_row
	return row
