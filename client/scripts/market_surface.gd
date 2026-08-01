## Market UI surface (roadmap 9D-5, ADR-0034).
##
## Owns only the presentation and input controls for the Market. The server
## remains authoritative for Currency, order matching, ownership, and item
## settlement; callbacks let main.gd keep network access at the application
## boundary.
extends RefCounted

var _panel: Panel = null
var _balance_label: Label = null
var _notice_label: Label = null
var _side_select: OptionButton = null
var _item_select: OptionButton = null
var _price_edit: LineEdit = null
var _quantity_edit: LineEdit = null
var _orders_list: VBoxContainer = null
var _cargo: Array = []
var _orders: Array = []
var _on_refresh: Callable
var _on_place: Callable
var _on_cancel: Callable


func build(
	hud: CanvasLayer,
	on_refresh: Callable,
	on_place: Callable,
	on_cancel: Callable
) -> void:
	_on_refresh = on_refresh
	_on_place = on_place
	_on_cancel = on_cancel

	_panel = Panel.new()
	_panel.name = "MarketPanel"
	_panel.mouse_filter = Control.MOUSE_FILTER_STOP
	_panel.add_theme_stylebox_override("panel", HudManager.hud_box_style(Color(0.35, 0.72, 0.64, 0.65)))
	_panel.anchor_left = 0.5
	_panel.anchor_right = 0.5
	_panel.anchor_top = 0.5
	_panel.anchor_bottom = 0.5
	_panel.offset_left = -280.0
	_panel.offset_right = 280.0
	_panel.offset_top = -250.0
	_panel.offset_bottom = 250.0
	_panel.visible = false
	_panel.z_index = 20
	hud.add_child(_panel)

	var root := VBoxContainer.new()
	root.set_anchors_preset(Control.PRESET_FULL_RECT)
	root.offset_left = 14.0
	root.offset_top = 12.0
	root.offset_right = -14.0
	root.offset_bottom = -12.0
	root.add_theme_constant_override("separation", 8)
	_panel.add_child(root)

	var header := HBoxContainer.new()
	root.add_child(header)
	var title := HudManager.make_hud_label(15, Color(0.72, 0.95, 0.88))
	title.text = "MARKET"
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	header.add_child(title)
	var close := Button.new()
	close.text = "Close"
	close.pressed.connect(func() -> void: set_open(false))
	header.add_child(close)

	var balance_row := HBoxContainer.new()
	root.add_child(balance_row)
	_balance_label = HudManager.make_hud_label(12, Color(0.82, 0.94, 0.90))
	_balance_label.text = "Currency: 0"
	_balance_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	balance_row.add_child(_balance_label)
	var refresh := Button.new()
	refresh.text = "Refresh"
	refresh.pressed.connect(func() -> void: _on_refresh.call())
	balance_row.add_child(refresh)

	_notice_label = HudManager.make_hud_label(11, Color(0.95, 0.78, 0.42))
	_notice_label.text = ""
	_notice_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	root.add_child(_notice_label)

	var form := GridContainer.new()
	form.columns = 2
	form.add_theme_constant_override("h_separation", 10)
	form.add_theme_constant_override("v_separation", 5)
	root.add_child(form)

	form.add_child(_field_label("Side"))
	_side_select = OptionButton.new()
	_side_select.add_item("Ask")
	_side_select.add_item("Bid")
	form.add_child(_side_select)

	form.add_child(_field_label("Item"))
	_item_select = OptionButton.new()
	_item_select.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	form.add_child(_item_select)

	form.add_child(_field_label("Price"))
	_price_edit = LineEdit.new()
	_price_edit.placeholder_text = "Currency / unit"
	_price_edit.text = "1"
	_price_edit.custom_minimum_size = Vector2(150.0, 0.0)
	form.add_child(_price_edit)

	form.add_child(_field_label("Quantity"))
	_quantity_edit = LineEdit.new()
	_quantity_edit.placeholder_text = "Units"
	_quantity_edit.text = "1"
	form.add_child(_quantity_edit)

	var place := Button.new()
	place.text = "Place order"
	place.pressed.connect(_place_order)
	root.add_child(place)

	var orders_header := HudManager.make_hud_label(12, Color(0.82, 0.87, 0.94))
	orders_header.text = "OPEN ORDERS"
	root.add_child(orders_header)

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	scroll.custom_minimum_size = Vector2(0.0, 150.0)
	root.add_child(scroll)
	_orders_list = VBoxContainer.new()
	_orders_list.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_orders_list.add_theme_constant_override("separation", 3)
	scroll.add_child(_orders_list)

	_refresh_item_options()


func _field_label(text: String) -> Label:
	var label := HudManager.make_hud_label(11, Color(0.62, 0.69, 0.80))
	label.text = text
	return label


func set_cargo(cargo: Array) -> void:
	_cargo = cargo.duplicate()
	_refresh_item_options()


func apply_snapshot(snapshot: MarketSnapshot) -> void:
	_balance_label.text = "Currency: %d" % snapshot.balance
	_notice_label.text = snapshot.notice
	_orders = snapshot.orders
	_render_orders()


func toggle() -> bool:
	set_open(not is_open())
	return is_open()


func set_open(open: bool) -> void:
	if _panel != null:
		_panel.visible = open


func is_open() -> bool:
	return _panel != null and _panel.visible


func panel_consumes(pos: Vector2) -> bool:
	return is_open() and _panel.get_global_rect().has_point(pos)


func keyboard_consumes() -> bool:
	if not is_open() or _panel.get_viewport() == null:
		return false
	return _panel.get_viewport().gui_get_focus_owner() != null


func _refresh_item_options() -> void:
	if _item_select == null:
		return
	_item_select.clear()
	for item: Variant in _cargo:
		var label := "%s x%d" % [item.name as String, item.count as int]
		_item_select.add_item(label)
	if _item_select.item_count == 0:
		_item_select.add_item("Scrap Metal")


## Returns one canonical Item identity plus the selected cargo count. The empty
## cargo fallback preserves the existing ability to place a Scrap Metal bid.
func _selected_item() -> Dictionary:
	if _cargo.is_empty():
		return {
			"item_id": ItemIdentity.scrap_metal(),
			"count": 0,
		}
	var index := _item_select.get_selected()
	if index < 0 or index >= _cargo.size():
		return {}
	var item: ItemRow = _cargo[index] as ItemRow
	return {
		"item_id": item.item_id,
		"count": item.count,
	}


func _place_order() -> void:
	var item := _selected_item()
	if item.is_empty():
		_notice_label.text = "No ship cargo selected"
		return
	var item_id: ItemIdentity = item.get("item_id") as ItemIdentity
	if item_id == null:
		_notice_label.text = "Invalid item selection"
		return
	var price := _price_edit.text.to_int()
	var quantity := _quantity_edit.text.to_int()
	if price <= 0 or quantity <= 0:
		_notice_label.text = "Price and quantity must be positive"
		return
	var side := "Ask" if _side_select.get_selected_id() == 0 else "Bid"
	if side == "Ask" and quantity > (item.count as int):
		_notice_label.text = "Quantity exceeds ship cargo"
		return
	_on_place.call(item_id, side, price, quantity)


func _render_orders() -> void:
	if _orders_list == null:
		return
	for child: Node in _orders_list.get_children():
		child.queue_free()
	for order_entry: Variant in _orders:
		var order: MarketOrder = order_entry as MarketOrder
		var row := HBoxContainer.new()
		row.custom_minimum_size = Vector2(0.0, 24.0)
		var text := "%s %s x%d @ %d" % [
			order.side,
			_order_item_name(order),
			order.quantity,
			order.price,
		]
		var label := HudManager.make_hud_label(11, Color(0.82, 0.87, 0.94))
		label.text = text
		label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		row.add_child(label)
		if order.is_own:
			var cancel := Button.new()
			cancel.text = "Cancel"
			cancel.pressed.connect(_cancel_order.bind(order.order_id))
			row.add_child(cancel)
		_orders_list.add_child(row)


func _order_item_name(order: MarketOrder) -> String:
	var item_id: ItemIdentity = order.item_id
	if item_id == null:
		return "Unknown item"
	if item_id.is_scrap_metal():
		return "Scrap Metal"
	if item_id.is_module():
		return "Module #%d" % (item_id.module_id() as int)
	if item_id.is_packaged_ship():
		return "Ship #%d" % (item_id.ship_type_id() as int)
	return "Unknown item"

func _cancel_order(order_id: int) -> void:
	if order_id >= 0:
		_on_cancel.call(order_id)
