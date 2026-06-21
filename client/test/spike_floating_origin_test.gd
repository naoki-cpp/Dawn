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
