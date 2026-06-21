## spike_floating_origin.gd
##
## ADR-0028 真スケール座標スパイク — S4: クライアント浮動原点（ゲート C2-1）。
## **これは捨てコード**（throwaway spike）。本番には統合しない。
##
## 問題：標準 Godot ビルドの Vector3 は f32 成分。実 AU（5 AU ≈ 7.48e11 m）の天体を
## WORLD_SCALE(0.1) 倍してもなお ~7.48e10 で、その付近の f32 ulp は数千 m。近傍物体の
## 10 m オフセットが量子化で消え、カメラ移動でジッタ／Z ファイトになる（= グローバル固定原点の破綻）。
##
## 解：プレイヤー近傍を原点に取り、描画 Vector3 を「(真の絶対位置 − 浮動原点)」で作る。
## GDScript の float は f64 なので、引き算を f64 で行ってから Vector3 を作れば、近傍物体は
## 小さい座標 = f32 で正確に表現される。
##
## ここでは「真の絶対位置」を f64（GDScript float）の [x,y,z] 配列で持ち、2 方式の
## 描画 Vector3 量子化誤差を比較する（目視ジッタの代わりの定量チェック）。
class_name SpikeFloatingOrigin
extends RefCounted

const WORLD_SCALE: float = 0.1  ## main.gd と同じサーバ→Godot スケール


## グローバル固定原点（現行 main.gd 方式）：真の絶対位置をそのまま Vector3 化。
## 大座標で f32 量子化が起き、近傍オフセットが失われる。
static func render_naive(truth_abs: Array) -> Vector3:
	return Vector3(
		truth_abs[0] * WORLD_SCALE,
		truth_abs[1] * WORLD_SCALE,
		-truth_abs[2] * WORLD_SCALE)


## 浮動原点方式：f64 で (真 − 原点) を引いてから Vector3 化。近傍は小座標で正確。
static func render_floating(truth_abs: Array, origin_abs: Array) -> Vector3:
	return Vector3(
		(truth_abs[0] - origin_abs[0]) * WORLD_SCALE,
		(truth_abs[1] - origin_abs[1]) * WORLD_SCALE,
		-(truth_abs[2] - origin_abs[2]) * WORLD_SCALE)


## 描画 Vector3 が表す「実際のサーバ m」を f64 で復元（量子化後の値）。
## naive 用：truth に対する量子化誤差の測定に使う。
static func decode_naive(rendered: Vector3) -> Array:
	return [
		float(rendered.x) / WORLD_SCALE,
		float(rendered.y) / WORLD_SCALE,
		-float(rendered.z) / WORLD_SCALE]


## 浮動原点用：原点を足し戻して実 m を復元。
static func decode_floating(rendered: Vector3, origin_abs: Array) -> Array:
	return [
		float(rendered.x) / WORLD_SCALE + origin_abs[0],
		float(rendered.y) / WORLD_SCALE + origin_abs[1],
		-float(rendered.z) / WORLD_SCALE + origin_abs[2]]
