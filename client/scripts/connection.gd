## connection.gd
##
## ClientConnection の GDScript 側ラッパー。
## サーバー（Rust simulation）との WebSocket 通信を担当する。
##
## 設計 (ADR-0005, ADR-0007):
##   接続フロー:
##     1. 接続確立 → Hello を送信
##     2. Welcome を受信 → player_id / ship_id と resume ticket を保持
##     3. 通常の ServerMessageOutcome を world target へ渡す
##
## Phase 5: Hello/Welcome ハンドシェイク（ORIGIN シグナルを廃止）

extends Node

# ── シグナル ──────────────────────────────────────────────────────────────────

signal connection_changed(connected: bool)
signal lifecycle_changed

# ── 設定 ─────────────────────────────────────────────────────────────────────

const SERVER_URL         := "ws://127.0.0.1:7878"

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ws: Variant = null
var _lifecycle := ClientConnectionLifecycle.new()
var _server_url      : String        = SERVER_URL
var _invite_server_url : String = ""
var _invite_token : String = ""
## Opaque server-issued capability used for reconnect/resume (ADR-0048).
var _resume_ticket   : PackedByteArray = PackedByteArray()
## ClientCommand/ServerMessageDecoder are GDExtension classes (dawn-protocol/
## dawn-client-gdext, ADR-0041/ADR-0042) -- globally registered, no preload
## needed. The decoder returns a typed ServerMessageOutcome that delegates to
## the canonical Rust inbound delivery module.
var _cmd             : ClientCommand         = ClientCommand.new()
var _decoder         : ServerMessageDecoder  = ServerMessageDecoder.new()
var _world_session   : WorldSession          = null
var _player_loadout  : PlayerLoadout         = null
var _world_target    : Object                = null

var player_id : int = -1
var ship_id   : int = -1

# ── ライフサイクル ────────────────────────────────────────────────────────────

func bind_client_state(
	session: WorldSession, loadout: PlayerLoadout, world_target: Object
) -> void:
	_world_session = session
	_player_loadout = loadout
	_world_target = world_target


func _ready() -> void:
	set_process(_lifecycle.is_running())


func lifecycle() -> ClientConnectionLifecycle:
	return _lifecycle


## Overridden by the test socket adapter; policy is identical in both paths.
func _new_socket() -> Variant:
	return WebSocketPeer.new()


func start_connection(url: String, token: String) -> bool:
	if not _lifecycle.can_edit() or not configure_endpoint(url, token):
		return false
	_resume_ticket.clear()
	set_process(true)
	_apply_transition(_lifecycle.start())
	return true


func cancel_connection() -> void:
	_apply_transition(_lifecycle.cancel())


## Native-client playtest configuration. Never log or serialize the bearer token.
func configure_endpoint(url: String, token: String) -> bool:
	var endpoint := SERVER_URL if url.is_empty() else url
	if not (endpoint.begins_with("ws://") or endpoint.begins_with("wss://")):
		return false
	for forbidden: String in ["@", "?", "#", "\r", "\n", "\t", "\\", " "]:
		if endpoint.contains(forbidden):
			return false
	if not token.is_empty():
		if token.length() != 64 or not token.is_valid_hex_number(false):
			return false
		var loopback := endpoint.begins_with("ws://127.0.0.1:") or endpoint.begins_with("ws://localhost:") or endpoint.begins_with("ws://[::1]:")
		if not endpoint.begins_with("wss://") and not loopback:
			return false
	_server_url = endpoint
	_invite_server_url = endpoint if not token.is_empty() else ""
	_invite_token = token
	return true


func _invite_headers() -> PackedStringArray:
	if _invite_token.is_empty() or _server_url != _invite_server_url:
		return PackedStringArray()
	return PackedStringArray(["Authorization: Bearer " + _invite_token])

func _process(delta: float) -> void:
	if not _lifecycle.is_running():
		return
	_apply_transition(_lifecycle.advance(delta))
	if not _lifecycle.is_running():
		return
	_ws.poll()
	var state: int = _ws.get_ready_state()

	if state == WebSocketPeer.STATE_OPEN:
		_apply_transition(_lifecycle.socket_opened())
		_receive_messages()

	elif state == WebSocketPeer.STATE_CLOSED:
		_apply_transition(_lifecycle.socket_closed())


func _apply_transition(step: ConnectionStep) -> void:
	if step != null:
		step.dispatch(self)


func _stop_transport() -> void:
	set_process(false)
	if _ws != null:
		_ws.close()
	_ws = null
	_invite_token = ""
	_invite_server_url = ""
	_resume_ticket.clear()
	player_id = -1
	ship_id = -1

# ── 公開 API ──────────────────────────────────────────────────────────────────

## ADR-0037: flight/steering/module/Undock commands carry no ship_id — the
## server always resolves them against the caller's active ship, so there is
## no wire-representable way to name a ship the player isn't currently
## flying. Owned-docked-ship commands (Fit/Unfit/Reorder/BuildPackagedShip/
## DisassembleShip/TransferCargo) still carry an explicit ship_id.
##
## ADR-0041/ADR-0042/#273: the remaining direct builders construct the one typed
## Rust `ClientRequest` authority through a dedicated GDExtension method. Input
## policy routes all other Sector requests through `send_action()` and station
## inventory policy routes its requests through `send_station_inventory_action()`.
## Market remains a separate typed request envelope.
func send_move_command(target: Vector3) -> void:
	_send_request(_cmd.move_command(target.x, target.y, target.z))

## Typed network actions arrive already classified by dawn-client-core. This is
## the transport entry point for requests that do not need Godot-only data.
## Camera-dependent double-click movement still uses send_move_command() after
## the local action asks main.gd to project the screen ray.
func send_action(action: ClientAction) -> void:
	_send_request(action.request_result())

## Station Inventory policy returns one request for ordinary rows or an
## ordered list for Unfit All. Each request is sent independently to preserve
## the existing non-atomic per-module semantics.
func send_station_inventory_action(action: StationInventoryAction) -> void:
	for index: int in action.request_count():
		_send_request(action.request_result_at(index))

## Active モジュールをオンにする。p_target_ship_id は Weapon/Tackle など
## ターゲットを要求する種別のときだけ指定する（-1 = 指定なし、ADR-0035）。
func send_activate_module(p_module_id: int, p_slot: String, p_target_ship_id: int = -1) -> void:
	_send_request(_cmd.activate_module_command(p_module_id, p_slot, p_target_ship_id))

## Active モジュールをオフにする。
func send_deactivate_module(p_module_id: int, p_slot: String) -> void:
	_send_request(_cmd.deactivate_module_command(p_module_id, p_slot))

## Market requests use a separate wire envelope from Sector commands
## (ADR-0034). The server answers each request with MarketSnapshot.
func send_market_refresh_command() -> void:
	_send_market_request(_cmd.market_refresh_command())

func send_market_place_order_command(
	p_ship_id: int,
	p_item_id: ItemIdentity,
	p_side: String,
	p_price: int,
	p_quantity: int
) -> void:
	_send_market_request(_cmd.market_place_order_command(
		p_ship_id, p_item_id, p_side, p_price, p_quantity))

func send_market_cancel_order_command(p_order_id: int) -> void:
	_send_market_request(_cmd.market_cancel_order_command(p_order_id))


func _accept_client_request_rejected(code: String, message: String) -> void:
	push_warning("[Connection] client request rejected [%s]: %s" % [code, message])

func is_connected_to_server() -> bool:
	return _lifecycle.is_connected()

# ── 内部処理 ──────────────────────────────────────────────────────────────────

## All builders return a typed ClientCommandResult. A failed construction or
## encoding is reported directly; no empty byte array is interpreted as an
## error sentinel.
func _send_request(result: ClientCommandResult) -> void:
	_send_result(result, true)


func _send_market_request(result: ClientCommandResult) -> void:
	_send_result(result, true)


func _send_result(result: ClientCommandResult, require_welcome: bool) -> void:
	if not result.ok:
		_accept_client_request_rejected(result.error_code, result.error_message)
		return
	if require_welcome and not _lifecycle.is_connected():
		return
	_ws.send(result.bytes, WebSocketPeer.WRITE_MODE_BINARY)

func _send_hello() -> void:
	_send_result(_cmd.hello_command(_resume_ticket), false)
	print("[Connection] Hello sent")

func _connect_to_server(attempt: int, log_attempt: bool) -> void:
	if log_attempt:
		print("[Connection] connecting to %s ... (attempt %d)" % [
			_server_url,
			attempt])
	_ws = _new_socket()
	_ws.handshake_headers = _invite_headers()
	_ws.heartbeat_interval = 20.0
	var err: int = _ws.connect_to_url(_server_url)
	if err != OK and log_attempt:
		push_warning("[Connection] connect_to_url failed: %s" % error_string(err))

## One WebSocket frame always carries exactly one postcard `ServerMessage`
## (ADR-0042). `ServerMessageDecoder` returns a typed Rust outcome. It delegates
## once to Rust inbound delivery, which selects the connection lifecycle target
## or final world presentation handler. No runtime path rebuilds the wire enum
## as a Dictionary or dispatches by a string `"type"` field.
func _receive_messages() -> void:
	while _ws.get_available_packet_count() > 0:
		var packet: PackedByteArray = _ws.get_packet()
		var outcome: ServerMessageOutcome = _decoder.decode(packet)
		if outcome == null:
			push_warning("[Connection] failed to decode binary ServerMessage")
			continue
		if _world_session == null or _player_loadout == null or _world_target == null:
			push_warning("[Connection] typed client state or world target is not bound")
			continue
		if not outcome.dispatch(
			self, _world_target, _world_session, _player_loadout, ship_id
		):
			push_warning("[Connection] failed to dispatch typed ServerMessage outcome")


func _accept_welcome(
	p_player_id: int,
	p_ship_id: int,
	p_resume_ticket: PackedByteArray
) -> void:
	var step := _lifecycle.welcome_received()
	if step == null:
		return
	player_id = p_player_id
	ship_id = p_ship_id
	_resume_ticket = p_resume_ticket
	_apply_transition(step)
	print("[Connection] Welcome: player_id=%d ship_id=%d" % [player_id, ship_id])


func _accept_redirect(ws_addr: String, p_resume_ticket: PackedByteArray) -> void:
	if ws_addr.is_empty():
		push_warning("[Connection] Redirect without ws_addr")
		return
	var step := _lifecycle.redirect()
	if step == null:
		return
	_resume_ticket = p_resume_ticket
	_server_url = _normalize_ws_url(ws_addr)
	print("[Connection] Redirect: reconnecting to %s with resume ticket" % _server_url)
	_ws.close()
	_apply_transition(step)


func _normalize_ws_url(addr: String) -> String:
	if addr.begins_with("ws://") or addr.begins_with("wss://"):
		return addr
	return "ws://" + addr
