## connection.gd
##
## ClientConnection の GDScript 側ラッパー。
## サーバー（Rust simulation）との WebSocket 通信を担当する。
##
## 設計 (ADR-0005):
##   サーバー → クライアント : DomainEvent の JSON ストリーム（改行区切り）
##   クライアント → サーバー : Command の JSON 送信
##
## Phase 4: WebSocket over localhost:7878
## Phase 5: gRPC/QUIC に差し替え（このファイルのみ変更）

extends Node

# ── シグナル ──────────────────────────────────────────────────────────────────

signal event_received(payload: Dictionary)
signal connection_changed(connected: bool)

# ── 設定 ─────────────────────────────────────────────────────────────────────

const SERVER_URL        := "ws://127.0.0.1:7878"
const RECONNECT_INTERVAL := 2.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var _ws                : WebSocketPeer = WebSocketPeer.new()
var _connected         : bool          = false
var _reconnect_timer   : float         = 0.0
var _buffer            : String        = ""

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_connect_to_server()

func _process(delta: float) -> void:
	_ws.poll()

	var state: int = _ws.get_ready_state()

	if state == WebSocketPeer.STATE_OPEN:
		if not _connected:
			_connected = true
			print("[Connection] connected to ", SERVER_URL)
			connection_changed.emit(true)
		_receive_messages()

	elif state == WebSocketPeer.STATE_CLOSED:
		if _connected:
			_connected = false
			print("[Connection] disconnected, reconnecting in %.1fs" % RECONNECT_INTERVAL)
			connection_changed.emit(false)
		_reconnect_timer += delta
		if _reconnect_timer >= RECONNECT_INTERVAL:
			_reconnect_timer = 0.0
			_connect_to_server()

# ── 公開 API ──────────────────────────────────────────────────────────────────

func send_move_command(ship_id: int, target: Vector3) -> void:
	if not _connected:
		return
	var payload: Dictionary = {
		"type":    "MoveCommand",
		"ship_id": ship_id,
		"target":  { "x": target.x, "y": target.y, "z": target.z }
	}
	_ws.send_text(JSON.stringify(payload) + "\n")

func is_connected_to_server() -> bool:
	return _connected

# ── 内部処理 ──────────────────────────────────────────────────────────────────

func _connect_to_server() -> void:
	print("[Connection] connecting to ", SERVER_URL, " ...")
	_ws = WebSocketPeer.new()
	var err: int = _ws.connect_to_url(SERVER_URL)
	if err != OK:
		push_warning("[Connection] connect_to_url failed: %s" % error_string(err))

func _receive_messages() -> void:
	while _ws.get_available_packet_count() > 0:
		var raw: String = _ws.get_packet().get_string_from_utf8()
		_buffer += raw
		_flush_buffer()

func _flush_buffer() -> void:
	while "\n" in _buffer:
		var idx: int    = _buffer.find("\n")
		var line: String = _buffer.left(idx).strip_edges()
		_buffer = _buffer.substr(idx + 1)
		if line.is_empty():
			continue
		var result: Variant = JSON.parse_string(line)
		if result == null:
			push_warning("[Connection] failed to parse JSON: " + line)
			continue
		event_received.emit(result as Dictionary)
