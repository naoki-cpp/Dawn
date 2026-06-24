## hud_manager.gd
##
## Builds and updates the player HUD panels (status, ship status, target,
## module bar, duel result overlay), extracted from main.gd
## (architecture-review-client.md C-1, final slice). Stateless static
## methods: build_* functions construct a Control subtree under the given
## `hud` root and return a Dictionary of the node references the caller
## needs to keep (mirroring main.gd's old member vars); update_* functions
## take those refs back plus the live values to display. main.gd owns the
## refs (stored in its own member vars) and all game state; this class only
## knows how to build/refresh Control nodes from values handed to it.
class_name HudManager
extends RefCounted

## Layer colours for the three HP bands and the capacitor (EVE convention).
const COLOR_SHIELD := Color(0.29, 0.56, 0.85)  ## blue
const COLOR_ARMOR  := Color(0.88, 0.63, 0.19)  ## amber
const COLOR_HULL   := Color(0.82, 0.29, 0.29)  ## red
const COLOR_CAP    := Color(0.17, 0.66, 0.54)  ## teal

## Module slot state colours (border + state label).
const MODULE_ON  := Color(0.30, 0.75, 0.45)  ## active
const MODULE_OFF := Color(0.45, 0.50, 0.60)  ## inactive
const MODULE_CAP := Color(0.85, 0.35, 0.35)  ## cap-forced off


# -- Shared style/label helpers ------------------------------------------------

## Shared semi-transparent dark background for HUD panels, so text stays legible
## over bright stars / nebula. Panels are display-only -- mouse input passes
## through to the 3D viewport (clicks are handled in main.gd's _input, not via
## Controls).
static func hud_box_style(border_color: Color = Color(0.47, 0.59, 0.78, 0.28)) -> StyleBoxFlat:
	var box := StyleBoxFlat.new()
	box.bg_color = Color(0.03, 0.05, 0.09, 0.72)
	box.set_corner_radius_all(6)
	box.set_border_width_all(1)
	box.border_color = border_color
	return box


static func make_hud_label(font_size: int, color: Color) -> Label:
	var lbl := Label.new()
	lbl.add_theme_font_size_override("font_size", font_size)
	lbl.add_theme_color_override("font_color", color)
	lbl.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return lbl


## Apply the dark-track / coloured-fill styleboxes to a progress bar.
static func style_bar(bar: ProgressBar, fill_color: Color) -> void:
	var fill := StyleBoxFlat.new()
	fill.bg_color = fill_color
	fill.set_corner_radius_all(2)
	bar.add_theme_stylebox_override("fill", fill)

	var bg := StyleBoxFlat.new()
	bg.bg_color = Color(0.08, 0.09, 0.12)
	bg.set_corner_radius_all(2)
	bar.add_theme_stylebox_override("background", bg)


## Build a label/bar/value row. Returns {row, bar, value} so the caller can
## update the bar and the numeric readout each frame.
static func make_stat_bar(label_text: String, fill_color: Color) -> Dictionary:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)
	row.mouse_filter = Control.MOUSE_FILTER_IGNORE

	var name_lbl := make_hud_label(11, fill_color.lightened(0.2))
	name_lbl.text = label_text
	name_lbl.custom_minimum_size = Vector2(30.0, 0.0)
	row.add_child(name_lbl)

	var bar := ProgressBar.new()
	bar.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	bar.size_flags_vertical   = Control.SIZE_SHRINK_CENTER
	bar.custom_minimum_size = Vector2(0.0, 9.0)
	bar.show_percentage = false
	bar.min_value = 0.0
	bar.max_value = 100.0
	bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
	style_bar(bar, fill_color)
	row.add_child(bar)

	var val_lbl := make_hud_label(11, Color(0.82, 0.87, 0.94))
	val_lbl.custom_minimum_size = Vector2(92.0, 0.0)
	val_lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	row.add_child(val_lbl)

	return {"row": row, "bar": bar, "value": val_lbl}


## A compact, label-less, number-less HP bar for the target panel.
static func make_mini_bar(fill_color: Color) -> ProgressBar:
	var bar := ProgressBar.new()
	bar.custom_minimum_size = Vector2(0.0, 6.0)
	bar.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	bar.show_percentage = false
	bar.min_value = 0.0
	bar.max_value = 100.0
	bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
	style_bar(bar, fill_color)
	return bar


## Set a {bar, value} pair to cur/max: fill percentage + "cur / max" readout.
static func set_stat_bar(entry: Dictionary, cur: float, mx: float) -> void:
	var pct: float = (cur / mx * 100.0) if mx > 0.0 else 0.0
	(entry["bar"] as ProgressBar).value = clampf(pct, 0.0, 100.0)
	(entry["value"] as Label).text = "%d / %d" % [int(round(cur)), int(round(mx))]


## Set a number-less mini bar to a cur/max fill percentage.
static func set_mini_bar(bar: ProgressBar, cur: float, mx: float) -> void:
	bar.value = clampf((cur / mx * 100.0) if mx > 0.0 else 0.0, 0.0, 100.0)


# -- Top-left status panel ------------------------------------------------------

## Builds the connection dot + ship name + "System X · N m/s" panel.
## Returns {conn_dot, conn_label, name_label, info_label}.
static func build_status_panel(hud: CanvasLayer) -> Dictionary:
	var panel := Panel.new()
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_theme_stylebox_override("panel", hud_box_style())
	panel.offset_left = 10.0;  panel.offset_top    = 10.0
	panel.offset_right = 232.0; panel.offset_bottom = 78.0
	hud.add_child(panel)

	var vb := VBoxContainer.new()
	vb.set_anchors_preset(Control.PRESET_FULL_RECT)
	vb.offset_left = 9.0; vb.offset_top = 6.0; vb.offset_right = -9.0; vb.offset_bottom = -6.0
	vb.add_theme_constant_override("separation", 2)
	vb.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_child(vb)

	var conn_row := HBoxContainer.new()
	conn_row.add_theme_constant_override("separation", 6)
	conn_row.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var conn_dot := ColorRect.new()
	conn_dot.custom_minimum_size = Vector2(8.0, 8.0)
	conn_dot.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	conn_dot.mouse_filter = Control.MOUSE_FILTER_IGNORE
	conn_row.add_child(conn_dot)
	var conn_label := make_hud_label(11, Color(0.62, 0.69, 0.80))
	conn_row.add_child(conn_label)
	vb.add_child(conn_row)

	var name_label := make_hud_label(13, Color(1.0, 0.62, 0.25))
	vb.add_child(name_label)
	var info_label := make_hud_label(11, Color(0.62, 0.69, 0.80))
	vb.add_child(info_label)

	return {"conn_dot": conn_dot, "conn_label": conn_label, "name_label": name_label, "info_label": info_label}


## Refresh the status panel from current connection/ship state.
static func update_status_panel(refs: Dictionary, connected: bool, ship_type_name: String, system_name: String, speed_str: String) -> void:
	var conn_dot: ColorRect = refs["conn_dot"]
	conn_dot.color = Color(0.25, 0.75, 0.42) if connected else Color(0.92, 0.66, 0.26)
	(refs["conn_label"] as Label).text = "ONLINE" if connected else "CONNECTING..."
	(refs["name_label"] as Label).text = ship_type_name if ship_type_name != "" else "—"
	(refs["info_label"] as Label).text = "System %s · %s" % [system_name, speed_str]


# -- Bottom-left ship-status panel ----------------------------------------------

## Builds the Shield / Armor / Hull bars + capacitor bar panel.
## Returns {bar_shield, bar_armor, bar_hull, bar_cap}, each itself the
## {row, bar, value} Dictionary returned by make_stat_bar().
static func build_ship_status_panel(hud: CanvasLayer) -> Dictionary:
	var panel := Panel.new()
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_theme_stylebox_override("panel", hud_box_style())
	## Pinned to the bottom-left corner with a 10px margin.
	panel.anchor_top = 1.0; panel.anchor_bottom = 1.0
	panel.offset_left = 10.0; panel.offset_right = 225.0
	panel.offset_top = -122.0; panel.offset_bottom = -10.0
	hud.add_child(panel)

	var vb := VBoxContainer.new()
	vb.set_anchors_preset(Control.PRESET_FULL_RECT)
	vb.offset_left = 9.0; vb.offset_top = 7.0; vb.offset_right = -9.0; vb.offset_bottom = -7.0
	vb.add_theme_constant_override("separation", 3)
	vb.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_child(vb)

	var header := make_hud_label(10, Color(0.54, 0.63, 0.76))
	header.text = "HULL INTEGRITY"
	vb.add_child(header)

	var bar_shield: Dictionary = make_stat_bar("SH", COLOR_SHIELD)
	var bar_armor : Dictionary = make_stat_bar("AR", COLOR_ARMOR)
	var bar_hull  : Dictionary = make_stat_bar("HU", COLOR_HULL)
	vb.add_child(bar_shield["row"])
	vb.add_child(bar_armor["row"])
	vb.add_child(bar_hull["row"])

	var spacer := Control.new()
	spacer.custom_minimum_size = Vector2(0.0, 2.0)
	spacer.mouse_filter = Control.MOUSE_FILTER_IGNORE
	vb.add_child(spacer)

	var bar_cap: Dictionary = make_stat_bar("CAP", COLOR_CAP)
	vb.add_child(bar_cap["row"])

	return {"bar_shield": bar_shield, "bar_armor": bar_armor, "bar_hull": bar_hull, "bar_cap": bar_cap}


## Drive the Shield / Armor / Hull bars and the capacitor bar from current state.
static func update_ship_status_panel(
	bars: Dictionary, player_ship_id: int,
	shield: float, max_shield: float, armor: float, max_armor: float, hull: float, max_hull: float,
	cap_current: float, cap_max: float,
) -> void:
	var bar_shield: Dictionary = bars["bar_shield"]
	var bar_armor : Dictionary = bars["bar_armor"]
	var bar_hull  : Dictionary = bars["bar_hull"]
	var bar_cap   : Dictionary = bars["bar_cap"]

	if player_ship_id < 0:
		## Destroyed: empty bars, flag the hull row.
		set_stat_bar(bar_shield, 0.0, max_shield)
		set_stat_bar(bar_armor,  0.0, max_armor)
		set_stat_bar(bar_hull,   0.0, max_hull)
		(bar_hull["value"] as Label).text = "DESTROYED"
	elif shield < 0.0:
		## State not yet received: assume full.
		set_stat_bar(bar_shield, max_shield, max_shield)
		set_stat_bar(bar_armor,  max_armor,  max_armor)
		set_stat_bar(bar_hull,   max_hull,   max_hull)
	else:
		set_stat_bar(bar_shield, shield, max_shield)
		set_stat_bar(bar_armor,  armor,  max_armor)
		set_stat_bar(bar_hull,   hull,   max_hull)

	if cap_current < 0.0:
		(bar_cap["bar"] as ProgressBar).value = 0.0
		(bar_cap["value"] as Label).text = "-"
	else:
		set_stat_bar(bar_cap, cap_current, cap_max)


# -- Top-center target panel -----------------------------------------------------

## Builds the lock-target panel (shown only while a lock target is held).
## Uses the same blue/amber/red colour coding as the self panel, but in
## compact bars with no numeric readout. Returns {panel, name_label,
## dist_label, bar_shield, bar_armor, bar_hull}.
static func build_target_panel(hud: CanvasLayer) -> Dictionary:
	var panel := Panel.new()
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_theme_stylebox_override("panel", hud_box_style(Color(0.82, 0.35, 0.35, 0.55)))
	panel.anchor_left = 0.5; panel.anchor_right = 0.5
	panel.offset_left = -110.0; panel.offset_right = 110.0
	panel.offset_top = 10.0; panel.offset_bottom = 70.0
	panel.visible = false
	hud.add_child(panel)

	var vb := VBoxContainer.new()
	vb.set_anchors_preset(Control.PRESET_FULL_RECT)
	vb.offset_left = 9.0; vb.offset_top = 6.0; vb.offset_right = -9.0; vb.offset_bottom = -6.0
	vb.add_theme_constant_override("separation", 3)
	vb.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_child(vb)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 6)
	header.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var name_label := make_hud_label(11, Color(0.90, 0.47, 0.47))
	name_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(name_label)
	var dist_label := make_hud_label(11, Color(0.82, 0.87, 0.94))
	header.add_child(dist_label)
	vb.add_child(header)

	var bar_shield := make_mini_bar(COLOR_SHIELD)
	var bar_armor  := make_mini_bar(COLOR_ARMOR)
	var bar_hull   := make_mini_bar(COLOR_HULL)
	vb.add_child(bar_shield)
	vb.add_child(bar_armor)
	vb.add_child(bar_hull)

	return {
		"panel": panel, "name_label": name_label, "dist_label": dist_label,
		"bar_shield": bar_shield, "bar_armor": bar_armor, "bar_hull": bar_hull,
	}


## Show / hide and populate the top-center target panel from the lock target.
## `hp` is {shield, max_shield, armor, max_armor, hull, max_hull} when
## `target_known` is true; ignored otherwise.
static func update_target_panel(refs: Dictionary, lock_target_id: int, target_known: bool, dist_text: String, hp: Dictionary) -> void:
	var panel: Panel = refs["panel"]
	if lock_target_id < 0:
		panel.visible = false
		return
	panel.visible = true
	(refs["name_label"] as Label).text = "◎ TARGET #%d" % lock_target_id

	var bar_shield: ProgressBar = refs["bar_shield"]
	var bar_armor : ProgressBar = refs["bar_armor"]
	var bar_hull  : ProgressBar = refs["bar_hull"]

	if not target_known:
		## Target left the area but the lock has not been cleared yet.
		(refs["dist_label"] as Label).text = "SIGNAL LOST"
		set_mini_bar(bar_shield, 0.0, 1.0)
		set_mini_bar(bar_armor,  0.0, 1.0)
		set_mini_bar(bar_hull,   0.0, 1.0)
		return

	(refs["dist_label"] as Label).text = dist_text

	## HP bars, relative to the target's own maxima (recorded at spawn). If we
	## have no HP record yet, leave the bars at their last known fill rather
	## than snapping to empty.
	if hp.is_empty():
		return
	set_mini_bar(bar_shield, hp.get("shield", 0.0) as float, hp.get("max_shield", 1.0) as float)
	set_mini_bar(bar_armor,  hp.get("armor",  0.0) as float, hp.get("max_armor",  1.0) as float)
	set_mini_bar(bar_hull,   hp.get("hull",   0.0) as float, hp.get("max_hull",   1.0) as float)


# -- Bottom-center module bar ----------------------------------------------------

## Builds the module bar container. The slots themselves are (re)populated
## by rebuild_module_bar(). A CenterContainer keeps the row centered
## regardless of how many modules are fitted.
static func build_module_bar(hud: CanvasLayer) -> HBoxContainer:
	var center := CenterContainer.new()
	center.set_anchors_preset(Control.PRESET_BOTTOM_WIDE)
	center.offset_top = -60.0
	center.offset_bottom = -8.0
	center.mouse_filter = Control.MOUSE_FILTER_IGNORE
	hud.add_child(center)

	var module_bar := HBoxContainer.new()
	module_bar.add_theme_constant_override("separation", 5)
	module_bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
	center.add_child(module_bar)
	return module_bar


## Rebuild the slot boxes from the current fitting. One slot per *active*
## module (passive modules have no F-key), in declaration order. Returns
## the new module_slots array (each entry: {panel, style, name, state,
## module_index}).
static func rebuild_module_bar(module_bar: HBoxContainer, player_modules: Array) -> Array:
	for child: Node in module_bar.get_children():
		child.queue_free()
	var module_slots: Array = []

	var f_number: int = 1
	for i: int in range(player_modules.size()):
		var mod_dict: Dictionary = player_modules[i] as Dictionary
		if not (mod_dict.get("is_active_module", false) as bool):
			continue  ## Skip Passive modules
		var slot: Dictionary = make_module_slot(f_number, mod_dict.get("name", "?") as String)
		slot["module_index"] = i
		module_bar.add_child(slot["panel"])
		module_slots.append(slot)
		f_number += 1
	return module_slots


## Build one slot box (F-number / name / state). Returns {panel, style, name,
## state, module_index} so update_module_bar() can refresh it each frame.
static func make_module_slot(f_number: int, mod_name: String) -> Dictionary:
	var style := StyleBoxFlat.new()
	style.bg_color = Color(0.03, 0.05, 0.09, 0.78)
	style.set_corner_radius_all(5)
	style.set_border_width_all(1)
	style.border_color = MODULE_OFF

	var panel := Panel.new()
	panel.custom_minimum_size = Vector2(66.0, 46.0)
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_theme_stylebox_override("panel", style)

	var vb := VBoxContainer.new()
	vb.set_anchors_preset(Control.PRESET_FULL_RECT)
	vb.offset_left = 3.0; vb.offset_top = 2.0; vb.offset_right = -3.0; vb.offset_bottom = -2.0
	vb.add_theme_constant_override("separation", 0)
	vb.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_child(vb)

	var f_lbl := make_hud_label(9, Color(0.45, 0.52, 0.63))
	f_lbl.text = "F%d" % f_number
	f_lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	vb.add_child(f_lbl)

	var name_lbl := make_hud_label(9, Color(0.85, 0.89, 0.95))
	name_lbl.text = mod_name
	name_lbl.clip_text = true
	name_lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	name_lbl.vertical_alignment   = VERTICAL_ALIGNMENT_CENTER
	name_lbl.size_flags_vertical  = Control.SIZE_EXPAND_FILL
	vb.add_child(name_lbl)

	var state_lbl := make_hud_label(9, MODULE_OFF)
	state_lbl.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	vb.add_child(state_lbl)

	return {"panel": panel, "style": style, "name": name_lbl, "state": state_lbl, "module_index": -1}


## Refresh each module slot's state text + border colour (ON / OFF / CAP!).
static func update_module_bar(module_slots: Array, player_modules: Array) -> void:
	for slot: Dictionary in module_slots:
		var idx: int = slot["module_index"]
		if idx < 0 or idx >= player_modules.size():
			continue
		var mod_dict: Dictionary = player_modules[idx] as Dictionary
		var col: Color
		var txt: String
		if mod_dict.get("cap_forced_off", false) as bool:
			col = MODULE_CAP;  txt = "CAP!"
		elif mod_dict.get("is_active", false) as bool:
			col = MODULE_ON;   txt = "ON"
		else:
			col = MODULE_OFF;  txt = "OFF"
		var state_lbl: Label = slot["state"]
		state_lbl.text = txt
		state_lbl.add_theme_color_override("font_color", col)
		(slot["style"] as StyleBoxFlat).border_color = col


## Returns the F-key index (0-based, i.e. the position in module_slots) of
## the module slot under a screen position, or -1. Used so a click on the
## bar toggles the module instead of the world.
static func module_slot_at(module_slots: Array, pos: Vector2) -> int:
	for i: int in range(module_slots.size()):
		var panel: Panel = module_slots[i]["panel"]
		if panel.get_global_rect().has_point(pos):
			return i
	return -1


# -- Duel result overlay -----------------------------------------------------------

## Builds the full-screen VICTORY/DEFEAT overlay (hidden by default) as a
## child of `parent`. Returns the Label to pass to show/hide_duel_result().
static func build_duel_result_overlay(parent: Node) -> Label:
	var canvas := CanvasLayer.new()
	canvas.layer = 10
	parent.add_child(canvas)

	var label := Label.new()
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	label.vertical_alignment   = VERTICAL_ALIGNMENT_CENTER
	label.anchors_preset       = Control.PRESET_FULL_RECT
	label.visible              = false
	label.add_theme_font_size_override("font_size", 96)
	canvas.add_child(label)
	return label


static func show_duel_result(label: Label, victory: bool) -> void:
	if label == null:
		return
	if victory:
		label.text = "VICTORY"
		label.add_theme_color_override("font_color", Color(0.2, 1.0, 0.3))
	else:
		label.text = "DEFEAT"
		label.add_theme_color_override("font_color", Color(1.0, 0.2, 0.2))
	label.visible = true


static func hide_duel_result(label: Label) -> void:
	if label != null:
		label.visible = false


# -- Inventory / Fitting panel (ADR-0032) -------------------------------------
#
# Each module has a fixed slot kind (Weapon -> High, Afterburner -> Mid, ...),
# so unlike a general-purpose inventory there is no slot-targeting step: an
# inventory row's only action is "fit this module's own kind", and a fitted
# row's only action is "unfit this one". One click each way, no drag-drop.
# Rows are plain Panels (not real Buttons) hit-tested manually in main.gd's
# _input, matching every other clickable HUD element here (module bar, etc.)
# -- mouse_filter stays MOUSE_FILTER_IGNORE throughout so world clicks behind
# a hidden panel are never blocked.

const INVENTORY_ROW_HEIGHT := 22.0

## Hidden by default; toggled by the I key. Returns
## {panel, fitted_list, inventory_list, fitted_rows, inventory_rows}.
## *_rows are populated by update_inventory_panel(); each row is
## {panel, module_id, slot, action} ("fit" or "unfit").
static func build_inventory_panel(hud: CanvasLayer) -> Dictionary:
	var panel := Panel.new()
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_theme_stylebox_override("panel", hud_box_style())
	panel.anchor_left = 0.5; panel.anchor_right = 0.5
	panel.anchor_top = 0.5; panel.anchor_bottom = 0.5
	panel.offset_left = -260.0; panel.offset_right = 260.0
	panel.offset_top = -180.0; panel.offset_bottom = 180.0
	panel.visible = false
	hud.add_child(panel)

	var columns := HBoxContainer.new()
	columns.set_anchors_preset(Control.PRESET_FULL_RECT)
	columns.offset_left = 10.0; columns.offset_top = 10.0
	columns.offset_right = -10.0; columns.offset_bottom = -10.0
	columns.add_theme_constant_override("separation", 12)
	columns.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.add_child(columns)

	var fitted_col := VBoxContainer.new()
	fitted_col.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	fitted_col.mouse_filter = Control.MOUSE_FILTER_IGNORE
	columns.add_child(fitted_col)
	var fitted_header := make_hud_label(12, Color(0.85, 0.89, 0.95))
	fitted_header.text = "FITTED (click to unfit)"
	fitted_col.add_child(fitted_header)
	var fitted_list := VBoxContainer.new()
	fitted_list.add_theme_constant_override("separation", 2)
	fitted_list.mouse_filter = Control.MOUSE_FILTER_IGNORE
	fitted_col.add_child(fitted_list)

	var inv_col := VBoxContainer.new()
	inv_col.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	inv_col.mouse_filter = Control.MOUSE_FILTER_IGNORE
	columns.add_child(inv_col)
	var inv_header := make_hud_label(12, Color(0.85, 0.89, 0.95))
	inv_header.text = "INVENTORY (click to fit)"
	inv_col.add_child(inv_header)
	var inventory_list := VBoxContainer.new()
	inventory_list.add_theme_constant_override("separation", 2)
	inventory_list.mouse_filter = Control.MOUSE_FILTER_IGNORE
	inv_col.add_child(inventory_list)

	return {
		"panel": panel, "fitted_list": fitted_list, "inventory_list": inventory_list,
		"fitted_rows": [], "inventory_rows": [],
	}


## One clickable row: "<Slot>: <Name>". Returns {panel, module_id, slot, action}.
static func _make_inventory_row(text: String, module_id: int, slot: String, action: String) -> Dictionary:
	var row := Panel.new()
	row.custom_minimum_size = Vector2(0.0, INVENTORY_ROW_HEIGHT)
	row.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var style := StyleBoxFlat.new()
	style.bg_color = Color(0.05, 0.07, 0.11, 0.6)
	style.set_corner_radius_all(3)
	row.add_theme_stylebox_override("panel", style)

	var lbl := make_hud_label(11, Color(0.82, 0.87, 0.94))
	lbl.text = text
	lbl.set_anchors_preset(Control.PRESET_FULL_RECT)
	lbl.offset_left = 6.0
	lbl.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	lbl.mouse_filter = Control.MOUSE_FILTER_IGNORE
	row.add_child(lbl)

	return {"panel": row, "module_id": module_id, "slot": slot, "action": action}


## Rebuild both columns from the latest PlayerFitting payload. `modules` is
## the flat fitted-module array (slot/module_id/name fields, same shape the
## module bar already consumes); `inventory` is [{module_id,name,slot,kind}].
static func update_inventory_panel(refs: Dictionary, modules: Array, inventory: Array) -> void:
	var fitted_list: VBoxContainer = refs["fitted_list"]
	var inventory_list: VBoxContainer = refs["inventory_list"]
	for child: Node in fitted_list.get_children():
		child.queue_free()
	for child: Node in inventory_list.get_children():
		child.queue_free()

	var fitted_rows: Array = []
	for entry: Variant in modules:
		var m: Dictionary = entry as Dictionary
		var slot: String = m.get("slot", "") as String
		var module_id: int = m.get("module_id", 0) as int
		var text := "%s: %s" % [slot, m.get("name", "?") as String]
		var row := _make_inventory_row(text, module_id, slot, "unfit")
		fitted_list.add_child(row["panel"])
		fitted_rows.append(row)
	refs["fitted_rows"] = fitted_rows

	var inventory_rows: Array = []
	for entry: Variant in inventory:
		var item: Dictionary = entry as Dictionary
		var slot: String = item.get("slot", "") as String
		var module_id: int = item.get("module_id", 0) as int
		var text := "%s: %s" % [slot, item.get("name", "?") as String]
		var row := _make_inventory_row(text, module_id, slot, "fit")
		inventory_list.add_child(row["panel"])
		inventory_rows.append(row)
	refs["inventory_rows"] = inventory_rows


static func toggle_inventory_panel(refs: Dictionary) -> void:
	var panel: Panel = refs["panel"]
	panel.visible = not panel.visible


## Returns the row under `pos` from either column, or {} if none.
## {module_id, slot, action} ("fit" sends FitModuleCommand, "unfit" sends
## UnfitModuleCommand) -- {} has no "action" key, so callers can check
## `result.has("action")` to tell a miss from a hit.
static func inventory_panel_row_at(refs: Dictionary, pos: Vector2) -> Dictionary:
	var panel: Panel = refs["panel"]
	if not panel.visible:
		return {}
	for row: Dictionary in (refs["fitted_rows"] as Array):
		if (row["panel"] as Panel).get_global_rect().has_point(pos):
			return row
	for row: Dictionary in (refs["inventory_rows"] as Array):
		if (row["panel"] as Panel).get_global_rect().has_point(pos):
			return row
	return {}


## True when the inventory panel is open and `pos` falls anywhere inside it
## (a row or its empty margin/header). Lets main.gd swallow the click so a
## miss on the open panel doesn't fall through to the 3D world behind it
## (thrust / select). Distinct from inventory_panel_row_at(), which only
## reports actionable row hits.
static func inventory_panel_consumes(refs: Dictionary, pos: Vector2) -> bool:
	var panel: Panel = refs["panel"]
	return panel.visible and panel.get_global_rect().has_point(pos)
