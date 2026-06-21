## spike_floating_origin_test.gd
##
## ADR-0028 スパイク S4（ゲート C2-1）の定量チェック。**捨てコード**。
## 実 AU の天体近傍で、グローバル固定原点 Vector3 は近傍オフセットを f32 量子化で失い、
## 浮動原点 Vector3 はそれを保つことを数値で確認する（目視ジッタの代わり）。
extends GdUnitTestSuite

## class_name はエディタ import 前だと headless で未登録になるため preload で参照する。
const SpikeFloatingOrigin = preload("res://scripts/spike_floating_origin.gd")

const AU_M: float = 1.495978707e11
const PLANET_AU: float = 5.0


## 惑星(5 AU)のすぐ近く(+10 m)にある物体を、両方式で描画→復元したときの誤差。
func test_floating_origin_preserves_nearby_offset_real_global_does_not() -> void:
	var planet_abs: Array = [PLANET_AU * AU_M, 0.0, 0.0]
	# 近傍物体：惑星 +10 m, +3 m。プレイヤー(=浮動原点)は惑星位置にいるとする。
	var nearby_abs: Array = [planet_abs[0] + 10.0, planet_abs[1] + 3.0, planet_abs[2]]
	var origin_abs: Array = planet_abs

	# --- グローバル固定原点（現行方式）---
	var naive_v: Vector3 = SpikeFloatingOrigin.render_naive(nearby_abs)
	var naive_back: Array = SpikeFloatingOrigin.decode_naive(naive_v)
	var err_naive: float = abs(naive_back[0] - nearby_abs[0])

	# --- 浮動原点 ---
	var float_v: Vector3 = SpikeFloatingOrigin.render_floating(nearby_abs, origin_abs)
	var float_back: Array = SpikeFloatingOrigin.decode_floating(float_v, origin_abs)
	var err_float: float = abs(float_back[0] - nearby_abs[0])

	prints("C2-1 naive global err (m):", err_naive, " floating-origin err (m):", err_float)

	# グローバル固定原点は近傍 10 m を量子化で失う（誤差が数 m オーダー）。
	assert_float(err_naive).is_greater(1.0)
	# 浮動原点は近傍を mm 未満で保つ。
	assert_float(err_float).is_less(0.001)


## 浮動原点をプレイヤー移動に追従させても、近傍物体の描画座標が小さく保たれる
## （= f32 安全域）ことを確認。原点を物体のすぐ近くに置く限り Vector3 成分は小さい。
func test_floating_origin_keeps_render_coords_small_under_motion() -> void:
	var planet_abs: Array = [PLANET_AU * AU_M, 0.0, 0.0]
	var max_component: float = 0.0
	# プレイヤーが惑星近傍を 1 km 刻みで動く間、近傍物体(+50 m)の描画成分を見る。
	for step in range(0, 100):
		var player_abs: Array = [planet_abs[0] + step * 1000.0, 0.0, 0.0]
		var obj_abs: Array = [player_abs[0] + 50.0, 0.0, 0.0]
		var v: Vector3 = SpikeFloatingOrigin.render_floating(obj_abs, player_abs)
		max_component = max(max_component, abs(v.x))
	prints("C2-1 max render |x| under motion (Godot units):", max_component)
	# 原点追従していれば描画座標は常に小さい（50 m * 0.1 = 5 単位程度）。
	assert_float(max_component).is_less(100.0)


## S5（ゲート C2-2）：ワープ中に浮動原点を離散リベース（1e8 m 刻みでスナップ）しても、
## *プレイヤー近傍*に随伴する物体（並走僚機・+200 m）の描画が原点切替フレームで飛ばないこと。
##
## 鍵：毎フレーム truth−origin を f64 で引き直す（前描画を平行移動で積み増さない）。近傍物体は
## 原点が粗くスナップしても (obj−origin) が小さいまま＝f32 で正確なので、余剰ジャンプは出ない。
##
## 補足の知見：*遠方*の固定天体（5 AU 先）は描画座標が巨大で f32 量子化されジッタるが、それは
## 浮動原点の限界ではなく「遠方は高精度メッシュで描かない」設計で解く（恒星と同じくビルボード/
## マーカー＝navigation_marker_renderer.gd の方針）。C2-2 が問うのは近傍の連続性のみ。
func test_warp_with_discrete_origin_rebasing_does_not_jump_nearby() -> void:
	var planet_abs: Array = [PLANET_AU * AU_M, 0.0, 0.0]
	var start: Array = [8000.0, 0.0, 0.0]  # 恒星近傍
	var arrival: Array = [planet_abs[0] - 1200.0, 0.0, 0.0]  # 惑星近傍
	var total: int = 250
	var rebase_quantum: float = 1.0e8  # 原点は 1e8 m 刻みでしかスナップしない（離散）

	# 見た目に効くのは「僚機 − プレイヤー」の相対描画位置。両者を同じ原点で引くので、
	# 原点がどれだけ粗くスナップしても相対位置は一定 (+200 m → 20 単位) のはず。
	# 真の相対 (+200 m) からのズレ＝可視ジャンプを測る。
	var prev_rel_x: float = INF
	var max_rel_drift: float = 0.0  # 真の相対(20 単位)からの最大ズレ
	var max_rel_jump: float = 0.0   # フレーム間の相対位置の飛び

	for elapsed in range(1, total + 1):
		var f: float = float(elapsed) / float(total)
		var t: float = f * f * (3.0 - 2.0 * f)  # smoothstep
		var player_x: float = start[0] + (arrival[0] - start[0]) * t
		# 並走僚機：常にプレイヤーの +200 m に随伴（近傍物体）。
		var wingman_abs: Array = [player_x + 200.0, 0.0, 0.0]
		# 離散リベース：原点はプレイヤーに最も近い量子点へスナップ。
		var origin_x: float = round(player_x / rebase_quantum) * rebase_quantum
		var origin_abs: Array = [origin_x, 0.0, 0.0]
		# プレイヤーと僚機を *同じ原点* で描画。
		var player_render: Vector3 = SpikeFloatingOrigin.render_floating([player_x, 0.0, 0.0], origin_abs)
		var wingman_render: Vector3 = SpikeFloatingOrigin.render_floating(wingman_abs, origin_abs)
		var rel_x: float = float(wingman_render.x) - float(player_render.x)

		# 真の相対 = 200 m * WORLD_SCALE = 20 単位。
		max_rel_drift = max(max_rel_drift, abs(rel_x - 200.0 * SpikeFloatingOrigin.WORLD_SCALE))
		if prev_rel_x != INF:
			max_rel_jump = max(max_rel_jump, abs(rel_x - prev_rel_x))
		prev_rel_x = rel_x

	prints("C2-2 relative pos: max drift from truth =", max_rel_drift, " max frame-to-frame jump =", max_rel_jump, "(Godot units)")
	# 同じ原点で引く限り、原点が粗くスナップしても相対位置は一定・飛ばない（f32 量子化分のみ）。
	assert_float(max_rel_drift).is_less(0.01)
	assert_float(max_rel_jump).is_less(0.01)
