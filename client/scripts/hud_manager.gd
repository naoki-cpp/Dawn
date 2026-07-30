## hud_manager.gd
##
## Builds and updates the player HUD panels (status, ship status, target,
## module bar, duel result overlay), extracted from main.gd
## (architecture-review/client.md C-1, final slice). Stateless static
## methods: build_* functions construct a Control subtree under the given
## `hud` root and return a typed *Refs object (StatusPanelRefs,
## ShipStatusPanelRefs, TargetPanelRefs, InventoryPanelRefs, ModuleSlotRefs)
## holding the node references the caller needs to keep (mirroring main.gd's
## old member vars); update_* functions take those refs back plus the live
## values to display. Typed fields, not a string-keyed Dictionary, so a
## renamed/dropped field is a compile error instead of a silent null at
## runtime. hud_surface.gd owns the refs (stored in its own member vars) and
## all game state; this class only knows how to build/refresh Control nodes
## from values handed to it.
## Hit-testing (answering "what's under this screen position") is a
## separate responsibility, split into HudHitTest (hud_hit_test.gd,
## architecture-review/client.md C-9) -- see that file's doc comment.
class_name HudManager
extends RefCounted

## ModuleRow/ItemRow/OwnedShipRow are GDExtension classes
## (dawn-client-gdext, ADR-0039/ADR-0040) -- globally registered, no preload needed.
const InventoryRow = preload("res://scripts/inventory_row.gd")

## Layer colours for the three HP bands and the capacitor (EVE convention).
const COLOR_SHIELD := Color(0.29, 0.56, 0.85)  ## blue
const COLOR_ARMOR  := Color(0.88, 0.63, 0.19)  ## amber
const COLOR_HULL   := Color(0.82, 0.29, 0.29)  ## red
const COLOR_CAP    := Color(0.17, 0.66, 0.54)  ## teal

## Module slot state colours (border + state label).
const MODULE_ON    := Color(0.30, 0.75, 0.45)  ## active
const MODULE_OFF   := Color(0.45, 0.50, 0.60)  ## inactive
const MODULE_CAP   := Color(0.85, 0.35, 0.35)  ## cap-forced off
const MODULE_RANGE := Color(0.85, 0.65, 0.25)  ## range-forced off (ADR-0035)


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


## Typed refs for one make_stat_bar() row, so update_ship_status_panel()
## doesn't re-derive the {row, bar, value} shape from string keys.
class StatBarRefs extends RefCounted:
	var row: HBoxContainer
	var bar: ProgressBar
	var value: Label

	func _init(row_: HBoxContainer, bar_: ProgressBar, value_: Label) -> void:
		row = row_
		bar = bar_
		value = value_


## Build a label/bar/value row. Returns the refs so the caller can update the
## bar and the numeric readout each frame.
static func make_stat_bar(label_text: String, fill_color: Color) -> StatBarRefs:
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

	return StatBarRefs.new(row, bar, val_lbl)


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


## Set a StatBarRefs pair to cur/max: fill percentage + "cur / max" readout.
static func set_stat_bar(entry: StatBarRefs, cur: float, mx: float) -> void:
	var pct: float = (cur / mx * 100.0) if mx > 0.0 else 0.0
	entry.bar.value = clampf(pct, 0.0, 100.0)
	entry.value.text = "%d / %d" % [int(round(cur)), int(round(mx))]


## Set a number-less mini bar to a cur/max fill percentage.
static func set_mini_bar(bar: ProgressBar, cur: float, mx: float) -> void:
	bar.value = clampf((cur / mx * 100.0) if mx > 0.0 else 0.0, 0.0, 100.0)


# -- Top-left status panel ------------------------------------------------------

## Typed refs for the top-left status panel.
class StatusPanelRefs extends RefCounted:
	var conn_dot: ColorRect
	var conn_label: Label
	var name_label: Label
	var info_label: Label

	func _init(conn_dot_: ColorRect, conn_label_: Label, name_label_: Label, info_label_: Label) -> void:
		conn_dot = conn_dot_
		conn_label = conn_label_
		name_label = name_label_
		info_label = info_label_


## Builds the connection dot + ship name + "System X · N m/s" panel.
static func build_status_panel(hud: CanvasLayer) -> StatusPanelRefs:
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

	return StatusPanelRefs.new(conn_dot, conn_label, name_label, info_label)


## Refresh the status panel from current connection/ship state.
static func update_status_panel(refs: StatusPanelRefs, connected: bool, ship_type_name: String, system_name: String, speed_str: String) -> void:
	refs.conn_dot.color = Color(0.25, 0.75, 0.42) if connected else Color(0.92, 0.66, 0.26)
	refs.conn_label.text = "ONLINE" if connected else "CONNECTING..."
	refs.name_label.text = ship_type_name if ship_type_name != "" else "—"
	refs.info_label.text = "System %s · %s" % [system_name, speed_str]


# -- Bottom-left ship-status panel ----------------------------------------------

## Typed refs for the bottom-left Shield/Armor/Hull/Capacitor bars panel.
class ShipStatusPanelRefs extends RefCounted:
	var bar_shield: StatBarRefs
	var bar_armor: StatBarRefs
	var bar_hull: StatBarRefs
	var bar_cap: StatBarRefs

	func _init(bar_shield_: StatBarRefs, bar_armor_: StatBarRefs, bar_hull_: StatBarRefs, bar_cap_: StatBarRefs) -> void:
		bar_shield = bar_shield_
		bar_armor = bar_armor_
		bar_hull = bar_hull_
		bar_cap = bar_cap_


## Builds the Shield / Armor / Hull bars + capacitor bar panel.
static func build_ship_status_panel(hud: CanvasLayer) -> ShipStatusPanelRefs:
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

	var bar_shield: StatBarRefs = make_stat_bar("SH", COLOR_SHIELD)
	var bar_armor : StatBarRefs = make_stat_bar("AR", COLOR_ARMOR)
	var bar_hull  : StatBarRefs = make_stat_bar("HU", COLOR_HULL)
	vb.add_child(bar_shield.row)
	vb.add_child(bar_armor.row)
	vb.add_child(bar_hull.row)

	var spacer := Control.new()
	spacer.custom_minimum_size = Vector2(0.0, 2.0)
	spacer.mouse_filter = Control.MOUSE_FILTER_IGNORE
	vb.add_child(spacer)

	var bar_cap: StatBarRefs = make_stat_bar("CAP", COLOR_CAP)
	vb.add_child(bar_cap.row)

	return ShipStatusPanelRefs.new(bar_shield, bar_armor, bar_hull, bar_cap)


## Drive the Shield / Armor / Hull bars and the capacitor bar from current state.
static func update_ship_status_panel(
	bars: ShipStatusPanelRefs, player_ship_id: int,
	shield: float, max_shield: float, armor: float, max_armor: float, hull: float, max_hull: float,
	cap_current: float, cap_max: float,
) -> void:
	var bar_shield: StatBarRefs = bars.bar_shield
	var bar_armor : StatBarRefs = bars.bar_armor
	var bar_hull  : StatBarRefs = bars.bar_hull
	var bar_cap   : StatBarRefs = bars.bar_cap

	if player_ship_id < 0:
		## Destroyed: empty bars, flag the hull row.
		set_stat_bar(bar_shield, 0.0, max_shield)
		set_stat_bar(bar_armor,  0.0, max_armor)
		set_stat_bar(bar_hull,   0.0, max_hull)
		bar_hull.value.text = "DESTROYED"
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
		bar_cap.bar.value = 0.0
		bar_cap.value.text = "-"
	else:
		set_stat_bar(bar_cap, cap_current, cap_max)


# -- Top-center target panel -----------------------------------------------------

## Builds the lock-target panel (shown only while a lock target is held).
## Uses the same blue/amber/red colour coding as the self panel, but in
## compact bars with no numeric readout. Returns {panel, name_label,
## dist_label, bar_shield, bar_armor, bar_hull}.
## Typed refs for the top-center lock-target panel.
class TargetPanelRefs extends RefCounted:
	var panel: Panel
	var name_label: Label
	var dist_label: Label
	var bar_shield: ProgressBar
	var bar_armor: ProgressBar
	var bar_hull: ProgressBar

	func _init(
		panel_: Panel, name_label_: Label, dist_label_: Label,
		bar_shield_: ProgressBar, bar_armor_: ProgressBar, bar_hull_: ProgressBar
	) -> void:
		panel = panel_
		name_label = name_label_
		dist_label = dist_label_
		bar_shield = bar_shield_
		bar_armor = bar_armor_
		bar_hull = bar_hull_


static func build_target_panel(hud: CanvasLayer) -> TargetPanelRefs:
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

	return TargetPanelRefs.new(panel, name_label, dist_label, bar_shield, bar_armor, bar_hull)


## Show / hide and populate the top-center target panel from the lock target.
## `hp` is {shield, max_shield, armor, max_armor, hull, max_hull} when
## `target_known` is true; ignored otherwise.
static func update_target_panel(refs: TargetPanelRefs, lock_target_id: int, target_known: bool, dist_text: String, hp: ShipHealth) -> void:
	if lock_target_id < 0:
		refs.panel.visible = false
		return
	refs.panel.visible = true
	refs.name_label.text = "◎ TARGET #%d" % lock_target_id

	var bar_shield: ProgressBar = refs.bar_shield
	var bar_armor : ProgressBar = refs.bar_armor
	var bar_hull  : ProgressBar = refs.bar_hull

	if not target_known:
		## Target left the area but the lock has not been cleared yet.
		refs.dist_label.text = "SIGNAL LOST"
		set_mini_bar(bar_shield, 0.0, 1.0)
		set_mini_bar(bar_armor,  0.0, 1.0)
		set_mini_bar(bar_hull,   0.0, 1.0)
		return

	refs.dist_label.text = dist_text

	## HP bars, relative to the target's own maxima (recorded at spawn). If we
	## have no HP record yet, leave the bars at their last known fill rather
	## than snapping to empty.
	if not (hp is ShipHealth):
		return
	var health: ShipHealth = hp as ShipHealth
	set_mini_bar(bar_shield, health.shield, health.max_shield)
	set_mini_bar(bar_armor,  health.armor,  health.max_armor)
	set_mini_bar(bar_hull,   health.hull,   health.max_hull)


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


## Typed refs for one module-bar slot (F-number/name/state box).
class ModuleSlotRefs extends RefCounted:
	var panel: Panel
	var style: StyleBoxFlat
	var name: Label
	var state: Label
	var module_index: int

	func _init(panel_: Panel, style_: StyleBoxFlat, name_: Label, state_: Label, module_index_: int = -1) -> void:
		panel = panel_
		style = style_
		name = name_
		state = state_
		module_index = module_index_


## Rebuild the slot boxes from the current fitting. One slot per *active*
## module (passive modules have no F-key), in declaration order. Returns
## the new module_slots array.
static func rebuild_module_bar(module_bar: HBoxContainer, player_modules: Array) -> Array[ModuleSlotRefs]:
	for child: Node in module_bar.get_children():
		child.queue_free()
	var module_slots: Array[ModuleSlotRefs] = []

	var f_number: int = 1
	for i: int in range(player_modules.size()):
		var row: ModuleRow = player_modules[i]
		if not row.is_active_module:
			continue  ## Skip Passive modules
		var slot: ModuleSlotRefs = make_module_slot(f_number, row.name)
		slot.module_index = i
		module_bar.add_child(slot.panel)
		module_slots.append(slot)
		f_number += 1
	return module_slots


## Build one slot box (F-number / name / state) so update_module_bar() can
## refresh it each frame.
static func make_module_slot(f_number: int, mod_name: String) -> ModuleSlotRefs:
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

	return ModuleSlotRefs.new(panel, style, name_lbl, state_lbl)


## Refresh each module slot's state text + border colour (ON / OFF / CAP! / RANGE!).
static func update_module_bar(module_slots: Array[ModuleSlotRefs], player_modules: Array) -> void:
	for slot: ModuleSlotRefs in module_slots:
		var idx: int = slot.module_index
		if idx < 0 or idx >= player_modules.size():
			continue
		var row: ModuleRow = player_modules[idx]
		var col: Color
		var txt: String
		## "cap" | "range" | "" (server-authoritative, ADR-0035) — replaces the
		## old client-side manual/forced heuristic, which always mislabelled
		## a range-forced deactivation as a capacitor exhaustion.
		if row.forced_reason == "cap":
			col = MODULE_CAP;   txt = "CAP!"
		elif row.forced_reason == "range":
			col = MODULE_RANGE; txt = "RANGE!"
		elif row.is_active:
			col = MODULE_ON;    txt = "ON"
		else:
			col = MODULE_OFF;   txt = "OFF"
		slot.state.text = txt
		slot.state.add_theme_color_override("font_color", col)
		slot.style.border_color = col


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
## *_rows are populated by update_inventory_panel() as Array[InventoryRow].
## Typed refs for the four-column inventory panel (FITTED / SHIP CARGO /
## STATION / SHIPS). `build_picker_open` is mutated after construction (see
## toggle_build_picker in hud_surface.gd) -- everything else is set once at
## build time, except the four `*_rows` arrays which update_inventory_panel()
## replaces wholesale on every rebuild.
class InventoryPanelRefs extends RefCounted:
	var panel: Panel
	var fitted_list: VBoxContainer
	var inventory_list: VBoxContainer
	var station_list: VBoxContainer
	var ships_list: VBoxContainer
	## The wrapping column (header + list) for each of the four columns --
	## column_at() hit-tests these instead of the bare *_list containers,
	## since a *_list with no rows yet (e.g. FITTED before any module is
	## fitted) collapses to zero height, but the wrapping column always has
	## nonzero height (the header Label).
	var fitted_col: VBoxContainer
	var inv_col: VBoxContainer
	var station_col: VBoxContainer
	var ships_col: VBoxContainer
	var fitted_rows: Array[InventoryRow] = []
	var inventory_rows: Array[InventoryRow] = []
	var station_rows: Array[InventoryRow] = []
	var ship_rows: Array[InventoryRow] = []
	for entry: Variant in owned_ships:
		var ship: OwnedShipRow = entry as OwnedShipRow
		var ship_id: int = ship.ship_id
		var is_active: bool = ship.is_active
		var name := ship.ship_type_name if not ship.ship_type_name.is_empty() else "Ship #%d" % ship_id
		var status := "active" if is_active else ("docked" if ship.docked_station_id >= 0 else "away")
		var text := "%s: %s" % [m.slot, m.name]
		var row := _make_inventory_row(
			text, m.module_id, m.slot, InventoryRow.ACTION_UNFIT, 0, "", 0,
			InventoryRow.SOURCE_FITTED, m.index)
		fitted_list.add_child(row.panel)
		fitted_rows.append(row)

	## "Unfit All" (e.g. to clear the way for Disassemble, which requires a
	## fully unfitted ship) -- only meaningful, and only shown, when at least
	## one module is actually fitted.
	if not fitted_rows.is_empty():
		var unfit_all_row := _make_inventory_row(
			"Unfit all", 0, "", InventoryRow.ACTION_UNFIT_ALL, 0, "", 0,
			InventoryRow.SOURCE_FITTED)
		fitted_list.add_child(unfit_all_row.panel)
		fitted_rows.append(unfit_all_row)

	refs.fitted_rows = fitted_rows

	var inventory_rows: Array[InventoryRow] = []
	for entry: Variant in inventory:
		var item: ItemRow = entry
		var text: String
		var action := InventoryRow.ACTION_NONE
		if item.item_type == "Module":
			text = "%s: %s x%d" % [item.slot, item.name, item.count]
			action = InventoryRow.ACTION_FIT
		else:
			text = "%s x%d" % [item.name, item.count]
		var row := _make_inventory_row(
			text, item.module_id, item.slot, action, item.ship_type_id,
			item.item_type, item.count, InventoryRow.SOURCE_SHIP_CARGO)
		inventory_list.add_child(row.panel)
		inventory_rows.append(row)
	refs.inventory_rows = inventory_rows

	var station_rows: Array[InventoryRow] = []
	for entry: Variant in station_inventory:
		var item: ItemRow = entry
		var text: String
		var action := InventoryRow.ACTION_NONE
		if item.item_type == "PackagedShip":
			text = "%s x%d (click to assemble)" % [item.name, item.count]
			action = InventoryRow.ACTION_ASSEMBLE
		else:
			text = "%s x%d" % [item.name, item.count]
		var row := _make_inventory_row(
			text, 0, "", action, item.ship_type_id, item.item_type, item.count,
			InventoryRow.SOURCE_STATION)
		station_list.add_child(row.panel)
		station_rows.append(row)

	## Disassemble/Build action rows (Phase 9B task 10) -- dedicated buttons
	## alongside the existing [Y]/[B] keyboard shortcuts, which keep working
	## unchanged. Always shown; the server validates docked/ownership context
	## and rejects if not applicable (same pattern as SHIPS-column
	## select_active_ship rows, which rely on server-side validation too).
	var disassemble_row := _make_inventory_row(
		"Disassemble active ship", 0, "", InventoryRow.ACTION_DISASSEMBLE, 0, "", 0,
		InventoryRow.SOURCE_STATION)
	station_list.add_child(disassemble_row.panel)
	station_rows.append(disassemble_row)

	var picker_open: bool = refs.build_picker_open
	var toggle_text := "Build Ship ▾" if picker_open else "Build Ship ▸"
	var build_toggle_row := _make_inventory_row(
		toggle_text, 0, "", InventoryRow.ACTION_BUILD_TOGGLE, 0, "", 0,
		InventoryRow.SOURCE_STATION)
	station_list.add_child(build_toggle_row.panel)
	station_rows.append(build_toggle_row)

	if picker_open:
		for entry: Variant in buildable_ship_types:
			var t: BuildableShipType = entry as BuildableShipType
			var ship_type_id: int = t.ship_type_id
			var name: String = t.name
			var picker_row := _make_inventory_row(
				"  %s" % name, 0, "", InventoryRow.ACTION_BUILD_SHIP_TYPE, ship_type_id,
				"", 0, InventoryRow.SOURCE_STATION)
			station_list.add_child(picker_row.panel)
			station_rows.append(picker_row)

	refs.station_rows = station_rows

	var ship_rows: Array[InventoryRow] = []
	for entry: Variant in owned_ships:
		var ship: Dictionary = entry as Dictionary
		var ship_id: int = ship.get("ship_id", 0) as int
		var is_active: bool = ship.get("is_active", false) as bool
		var raw_ship_type_name: Variant = ship.get("ship_type_name", null)
		var ship_type_name: String = "" if raw_ship_type_name == null else raw_ship_type_name as String
		var name := ship_type_name if not ship_type_name.is_empty() else "Ship #%d" % ship_id
		var raw_docked_station_id: Variant = ship.get("docked_station_id", null)
		var docked_station_id: int = -1 if raw_docked_station_id == null else raw_docked_station_id as int
		var status := "active" if is_active else ("docked" if docked_station_id >= 0 else "away")
		var text := "%s (%s)" % [name, status]
		var row := _make_ship_row(text, ship_id, is_active)
		ships_list.add_child(row.panel)
		ship_rows.append(row)
	refs.ship_rows = ship_rows


static func toggle_inventory_panel(refs: InventoryPanelRefs) -> void:
	refs.panel.visible = not refs.panel.visible


## Hit-testing (module_slot_at, inventory_panel_row_at, column_at,
## inventory_panel_consumes) lives in hud_hit_test.gd (HudHitTest) -- see
## that file's doc comment for why it was split out.
