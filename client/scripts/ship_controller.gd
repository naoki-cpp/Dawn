## ship_controller.gd
##
## 個々の Ship エンティティの表示と移動補間を担当する。
##
## サーバーから受け取った ShipMoved イベントの座標へ
## スムーズに補間移動する（クライアントサイド予測の簡易版）。

extends Node3D

# ── 設定 ─────────────────────────────────────────────────────────────────────

## サーバー座標系からGodot座標系へのスケール係数
## サーバー: 10,000 単位のSector / Godot: メートル単位なので縮小
const WORLD_SCALE := 0.1

## 位置補間の速さ（1秒あたりどれだけターゲットに近づくか）
const LERP_SPEED := 8.0

# ── 内部状態 ─────────────────────────────────────────────────────────────────

var ship_id       : int      ## EntityId の u64 値
var _target_pos   : Vector3  ## サーバーから受け取ったターゲット座標（Godot座標系）
var _is_init      := false

# ── ライフサイクル ────────────────────────────────────────────────────────────

func _ready() -> void:
	_target_pos = position

func _process(delta: float) -> void:
	if not _is_init:
		return
	## ターゲット座標へ線形補間で追従する
	position = position.lerp(_target_pos, LERP_SPEED * delta)

# ── 公開 API ──────────────────────────────────────────────────────────────────

## SpawnedEvent を受けて Ship を初期化する。
## server_pos: サーバー座標系の Vector3
func initialize(id: int, server_pos: Vector3) -> void:
	ship_id     = id
	_target_pos = _to_godot(server_pos)
	position    = _target_pos
	_is_init    = true

## ShipMoved イベントを受けてターゲット座標を更新する。
## 補間は _process() で自動的に行われる。
func update_target(server_pos: Vector3) -> void:
	_target_pos = _to_godot(server_pos)

# ── 座標変換 ─────────────────────────────────────────────────────────────────

## サーバー座標系（右手, Z前方）→ Godot 座標系（右手, -Z前方）
static func _to_godot(v: Vector3) -> Vector3:
	return Vector3(v.x, v.y, -v.z) * WORLD_SCALE
