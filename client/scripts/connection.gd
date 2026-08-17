## connection.gd
##
## ClientConnection の GDScript 側ラッパー。
## サーバー（Rust simulation）との WebSocket 通信を担当する。
##
## 設計 (ADR-0005, ADR-0007):
##   接続フロー:
##     1. 接続確立 → Hello を送信
##     2. Welcome を受信 → player_id / ship_id を保持、welcomed シグナル発行
##     3. InitialState を受信 → initial_state シグナル発行
##     4. 通常の DomainEvent ストリームを受信
##
## Phase 5: Hello/Welcome ハンドシェイク（ORIGIN シグナルを廃止）

extends Node

# ── シグナル ──────────────────────────────────────────────────────────────────

## Owner-only authoritative normal-flight state for client prediction (ADR-0043).
signal motion_correction_received(correction: MotionCorrectionPresentation)
signal connection_changed(connected: bool)
## Welcome 受信時: player_id と ship_id を通知
signal welcomed(player_id: int, ship_id: int)
## On InitialState: notifies the full payload (ships + navigation map)
signal initial_state_received(state: InitialStatePresentation)
## On PlayerLoadout, after the typed Rust state has already been replaced.
signal player_fitting_received()
## ModuleActivated 受信時
signal module_activated(ship_id: int, module_id: int, slot: String)
## ModuleDeactivated 受信時。reason は "cap" | "range" | ""（""=プレイヤー起因、ADR-0035）。
signal module_deactivated(ship_id: int, module_id: int, slot: String, reason: String)
## Current Market balance and bounded open-order snapshot.
signal market_snapshot_received(snapshot: MarketSnapshot)

# ── 設定 ─────────────────────────────────────────────────────────────────────

const SERVER_URL         := "ws://127.0.0.1:7878"
const RECONNECT_INTERVAL := 2.0
## Keep retry latency short without writing one log record per attempt while
## the server is unavailable. Godot has no project-level log rotation setting,
## so the client must bound its own reconnect diagnostics.
const RECONNECT_LOG_INTERVAL := 30.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ws              : WebSocketPeer = WebSocketPeer.new()
var _connected       : bool          = false
var _welcomed        : bool          = false   ## Welcome 受信済みか
var _reconnect_timer : float         = 0.0
var _reconnect_log_elapsed : float  = RECONNECT_LOG_INTERVAL
var _reconnect_attempts : int       = 0
var _server_url      : String        = SERVER_URL
## Opaque server-issued capability used for reconnect/resume (ADR-0048).
var _resume_ticket   : PackedByteArray = PackedByteArray()
## ClientCommand/ServerMessageDecoder are GDExtension classes (dawn-protocol/
## dawn-client-gdext, ADR-0041/ADR-0042) -- globally registered, no preload
## needed. The decoder returns a typed ServerMessageOutcome that owns all
## Rust-side variant projection.
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
	_connect_to_server()

func _process(delta: float) -> void:
	_ws.poll()
	var state: int = _ws.get_ready_state()

	if state == WebSocketPeer.STATE_OPEN:
		if not _connected:
			_connected = true
			_welcomed  = false
			_reconnect_attempts = 0
			_reconnect_log_elapsed = 0.0
			print("[Connection] connected to ", _server_url)
			connection_changed.emit(true)
			## 接続直後に Hello を送信する
			_send_hello()
		_receive_messages()

	elif state == WebSocketPeer.STATE_CLOSED:
		if _connected:
			_connected = false
			_welcomed  = false
			print("[Connection] disconnected, reconnecting in %.1fs" % RECONNECT_INTERVAL)
			connection_changed.emit(false)
		_reconnect_timer += delta
		_reconnect_log_elapsed += delta
		if _reconnect_timer >= RECONNECT_INTERVAL:
			_reconnect_timer = 0.0
			_connect_to_server()

# ── 公開 API ──────────────────────────────────────────────────────────────────

## ADR-0037: flight/steering/module/Undock commands carry no ship_id — the
## server always resolves them against the caller's active ship, so there is
## no wire-representable way to name a ship the player isn't currently
## flying. Owned-docked-ship commands (Fit/Unfit/Reorder/BuildPackagedShip/
## DisassembleShip/TransferCargo) still carry an explicit ship_id.
##
## ADR-0041/ADR-0042/#273: every Sector send_* function constructs the one
## typed Rust `ClientRequest` authority through a dedicated GDExtension method.
## No Sector request is assembled through a Dictionary/JSON round-trip, and
## acting active-ship identity is supplied only by the admitted server session.
## Market remains a separate typed request envelope.
func send_move_command(target: Vector3) -> void:
	_send_request(_cmd.move_command(target.x, target.y, target.z))

## Typed network actions arrive already classified by dawn-client-core. This is
## the transport entry point for requests that do not need Godot-only data.
## Camera-dependent double-click movement still uses send_move_command() after
## the local action asks main.gd to project the screen ray.
func send_action(action: ClientAction) -> void:
	_send_request(action.request_result())

func send_lock_on_command(target_id: int) -> void:
	_send_request(_cmd.lock_on_command(target_id))

## Active モジュールをオンにする。p_target_ship_id は Weapon/Tackle など
## ターゲットを要求する種別のときだけ指定する（-1 = 指定なし、ADR-0035）。
func send_activate_module(p_module_id: int, p_slot: String, p_target_ship_id: int = -1) -> void:
	_send_request(_cmd.activate_module_command(p_module_id, p_slot, p_target_ship_id))

## Active モジュールをオフにする。
func send_deactivate_module(p_module_id: int, p_slot: String) -> void:
	_send_request(_cmd.deactivate_module_command(p_module_id, p_slot))

## [S キー] 減速停止コマンド。サーバーが thrust を逆方向に掛けて速度ゼロまで減速する。
func send_stop_command() -> void:
	_send_request(_cmd.stop_command())

## ジャンプゲート経由の Sector 移動を要求する（ADR-0009）。
func send_jump_command(p_gate_id: int) -> void:
	_send_request(_cmd.jump_command(p_gate_id))

## [A キー] アプローチ（半自動操船）。選択した船へ自動接近する（ADR-0015）。
func send_approach_command(p_target_id: int) -> void:
	_send_request(_cmd.approach_command(p_target_id))

## [A キー] ジャンプゲートへアプローチ（半自動操船）。射程内まで自動接近する（ADR-0015）。
func send_approach_gate_command(p_gate_id: int) -> void:
	_send_request(_cmd.approach_gate_command(p_gate_id))

## [W key] Warp (short-range Fold) to a Jump Gate (ADR-0022/ADR-0025).
func send_warp_command(p_gate_id: int) -> void:
	_send_request(_cmd.warp_command(p_gate_id))

## [W key] Warp (short-range Fold) to a celestial body (ADR-0025).
func send_warp_to_body_command(p_body_id: int) -> void:
	_send_request(_cmd.warp_to_body_command(p_body_id))

## [W key] Warp (short-range Fold) to an NPC station.
func send_warp_to_station_command(p_station_id: int) -> void:
	_send_request(_cmd.warp_to_station_command(p_station_id))

## [O key] Orbit a selected ship at its weapon range (server-side default, ADR-0031).
func send_orbit_command(p_target_id: int) -> void:
	_send_request(_cmd.orbit_command(p_target_id, -1.0))

## [O key] Orbit a selected Jump Gate at its weapon range (server-side default, ADR-0031).
func send_orbit_gate_command(p_gate_id: int) -> void:
	_send_request(_cmd.orbit_gate_command(p_gate_id, -1.0))

## [K key] Hold at least p_range_m metres from a selected ship; p_range_m <= 0
## falls back to the server-side default (weapon range, ADR-0031).
func send_keep_at_range_command(p_target_id: int, p_range_m: float = -1.0) -> void:
	_send_request(_cmd.keep_at_range_command(p_target_id, p_range_m))

## [K key] Hold at least p_range_m metres from a selected Jump Gate; p_range_m
## <= 0 falls back to the server-side default (weapon range, ADR-0031).
func send_keep_at_range_gate_command(p_gate_id: int, p_range_m: float = -1.0) -> void:
	_send_request(_cmd.keep_at_range_gate_command(p_gate_id, p_range_m))

## [Inventory panel] Move a module from inventory into a fitting slot (ADR-0032).
func send_fit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	_send_request(_cmd.fit_module_command(p_ship_id, p_module_id, p_slot))

## [Inventory panel] Move a fitted module back into inventory (ADR-0032).
func send_unfit_module_command(p_ship_id: int, p_module_id: int, p_slot: String) -> void:
	_send_request(_cmd.unfit_module_command(p_ship_id, p_module_id, p_slot))

## [Inventory panel] Reorder two fitted modules within the same slot kind
## (drag-and-drop reorder in the FITTED column). Persisted server-side since
## iteration order assigns weapon hotkey F-numbers -- see ADR-0032's
## 2026-07-08 amendment.
func send_reorder_fitted_module_command(
	p_ship_id: int, p_slot: String, p_from_index: int, p_to_index: int
) -> void:
	_send_request(_cmd.reorder_fitted_module_command(
		p_ship_id, p_slot, p_from_index, p_to_index))

func send_dock_command(p_station_id: int) -> void:
	_send_request(_cmd.dock_command(p_station_id))

func send_undock_command() -> void:
	_send_request(_cmd.undock_command())

func send_build_packaged_ship_command(p_ship_id: int, p_station_id: int, p_ship_type_id: int) -> void:
	_send_request(_cmd.build_packaged_ship_command(p_ship_id, p_station_id, p_ship_type_id))

func send_disassemble_ship_command(p_ship_id: int, p_station_id: int) -> void:
	_send_request(_cmd.disassemble_ship_command(p_ship_id, p_station_id))

## Convert a station-inventory Packaged Ship item into a new live docked ship
## (ADR-0034 9B, ADR-0037). No ship_id -- the ship doesn't exist yet.
func send_assemble_command(p_station_id: int, p_ship_type_id: int) -> void:
	_send_request(_cmd.assemble_command(p_station_id, p_ship_type_id))

## Leave the active ship while docked, without disassembling it (ADR-0037).
func send_disembark_command() -> void:
	_send_request(_cmd.disembark_command())

## Make an owned, docked ship the caller's active ship (ADR-0037). This is
## how a player re-boards after Disembark, or switches between owned ships.
func send_select_active_ship_command(p_ship_id: int) -> void:
	_send_request(_cmd.select_active_ship_command(p_ship_id))

## Move the entire stack of one canonical Item identity out of a docked ship's
## cargo into the caller's station inventory (ADR-0034 9B).
func send_transfer_to_station_command(
	p_ship_id: int,
	p_station_id: int,
	p_item_id: ItemIdentity
) -> void:
	_send_request(_cmd.transfer_to_station_command(p_ship_id, p_station_id, p_item_id))

## The reverse of send_transfer_to_station_command: move the entire stack from
## station inventory back into the docked ship's cargo.
func send_transfer_from_station_command(
	p_ship_id: int,
	p_station_id: int,
	p_item_id: ItemIdentity
) -> void:
	_send_request(_cmd.transfer_from_station_command(p_ship_id, p_station_id, p_item_id))

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
	return _connected and _welcomed

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
	if require_welcome and not _welcomed:
		return
	_ws.send(result.bytes, WebSocketPeer.WRITE_MODE_BINARY)

func _send_hello() -> void:
	_send_result(_cmd.hello_command(_resume_ticket), false)
	print("[Connection] Hello sent")

func _connect_to_server() -> void:
	_reconnect_attempts += 1
	var should_log_attempt := should_log_reconnect(
		_reconnect_attempts,
		_reconnect_log_elapsed,
		RECONNECT_LOG_INTERVAL)
	if should_log_attempt:
		_reconnect_log_elapsed = 0.0
		print("[Connection] connecting to %s ... (attempt %d)" % [
			_server_url,
			_reconnect_attempts])
	_ws = WebSocketPeer.new()
	var err: int = _ws.connect_to_url(_server_url)
	if err != OK and should_log_attempt:
		push_warning("[Connection] connect_to_url failed: %s" % error_string(err))

## Reconnect attempts remain frequent for responsiveness, but their diagnostics
## are emitted only on the first attempt and at the bounded interval thereafter.
static func should_log_reconnect(attempt: int, elapsed: float, interval: float) -> bool:
	return attempt <= 1 or elapsed >= interval

## One WebSocket frame always carries exactly one postcard `ServerMessage`
## (ADR-0042). `ServerMessageDecoder` returns a typed Rust outcome; the
## outcome itself owns variant projection and calls one fixed `_accept_*`
## method below. No runtime path rebuilds the wire enum as a Dictionary or
## dispatches by a string `"type"` field.
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
	player_id = p_player_id
	ship_id = p_ship_id
	_resume_ticket = p_resume_ticket
	_welcomed = true
	print("[Connection] Welcome: player_id=%d ship_id=%d" % [player_id, ship_id])
	welcomed.emit(player_id, ship_id)


func _accept_initial_state(state: InitialStatePresentation) -> void:
	print("[Connection] InitialState: %d ships" % state.ships.size())
	initial_state_received.emit(state)


func _accept_player_loadout() -> void:
	print("[Connection] PlayerLoadout received")
	player_fitting_received.emit()


func _accept_redirect(ws_addr: String, p_resume_ticket: PackedByteArray) -> void:
	if ws_addr.is_empty():
		push_warning("[Connection] Redirect without ws_addr")
		return
	_resume_ticket = p_resume_ticket
	_server_url = _normalize_ws_url(ws_addr)
	_welcomed = false
	_connected = false
	_reconnect_timer = RECONNECT_INTERVAL
	print("[Connection] Redirect: reconnecting to %s with resume ticket" % _server_url)
	connection_changed.emit(false)
	_ws.close()


func _accept_module_activated(p_ship_id: int, p_module_id: int, slot: String) -> void:
	module_activated.emit(p_ship_id, p_module_id, slot)


func _accept_module_deactivated(
	p_ship_id: int,
	p_module_id: int,
	slot: String,
	reason: String
) -> void:
	module_deactivated.emit(p_ship_id, p_module_id, slot, reason)


func _accept_market_snapshot(snapshot: MarketSnapshot) -> void:
	market_snapshot_received.emit(snapshot)


func _accept_motion_correction(correction: MotionCorrectionPresentation) -> void:
	motion_correction_received.emit(correction)


func _normalize_ws_url(addr: String) -> String:
	if addr.begins_with("ws://") or addr.begins_with("wss://"):
		return addr
	return "ws://" + addr
