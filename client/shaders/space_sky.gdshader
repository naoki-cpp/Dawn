// space_sky.gdshader
//
// 手続き生成による宇宙背景。外部テクスチャ不要。
//
// 生成要素:
//   - 星 : 4 レイヤー、密度・色温度・輝度にランダム変動
//   - 天の川帯 : 赤道付近に淡い青白い帯
//   - 深宇宙の霞 : ごく薄い青紫の散光

shader_type sky;
render_mode use_half_res_pass;

// ── パラメータ ──────────────────────────────────────────────────────────────

/// 星の密度閾値（高いほど星が少ない。0.96〜0.99 が適当）
uniform float star_threshold   : hint_range(0.90, 0.999) = 0.970;
/// 星の最大輝度
uniform float star_brightness  : hint_range(0.5,  5.0)   = 2.0;
/// 天の川の強度
uniform float milkyway_strength: hint_range(0.0,  0.2)   = 0.04;
/// 天の川の色（青白い）
uniform vec3  milkyway_color   : source_color             = vec3(0.55, 0.65, 0.9);
/// 背景の散光色（非常に暗い紫）
uniform vec3  ambient_color    : source_color             = vec3(0.01, 0.005, 0.02);

// ── ハッシュ関数 ──────────────────────────────────────────────────────────────

float hash1(vec3 p) {
	p = fract(p * vec3(443.8975, 397.2973, 491.1871));
	p += dot(p.zxy, p.yxz + 19.19);
	return fract(p.x * p.y * p.z);
}

// ── メイン ────────────────────────────────────────────────────────────────────

void sky() {
	vec3 dir = normalize(EYEDIR);
	vec3 col = ambient_color;

	// ── 星レイヤー（近〜遠の4層）──────────────────────────────────────────
	float scales[4];
	scales[0] = 80.0;
	scales[1] = 150.0;
	scales[2] = 260.0;
	scales[3] = 420.0;

	for (int i = 0; i < 4; i++) {
		vec3 p      = dir * scales[i];
		vec3 pi     = floor(p);
		vec3 pf     = fract(p);
		float h     = hash1(pi);

		if (h > star_threshold) {
			// 輝度: 閾値超過分を正規化してべき乗
			float t  = (h - star_threshold) / (1.0 - star_threshold);
			float bri = pow(t, 1.5) * star_brightness;

			// セル中心からの距離でぼかす（点光源らしく）
			vec2  offset = pf.xz - 0.5;
			float d      = length(offset);
			float glow   = exp(-d * 30.0) * bri;

			// 色温度: 青白い星〜赤みがかった星
			float temp    = hash1(pi + vec3(0.5));
			vec3  star_col = mix(
				vec3(0.7, 0.85, 1.0),   // 青白い（高温星）
				vec3(1.0, 0.85, 0.65),  // 赤みがかった（低温星）
				temp
			);

			col += glow * star_col;
		}
	}

	// ── 天の川帯 ─────────────────────────────────────────────────────────
	// dir.y ≈ 0 の平面に沿って淡く光る帯を作る
	float band_y   = abs(dir.y);
	float band     = exp(-band_y * 5.0) * milkyway_strength;
	// 経度方向にゆらぎを持たせる
	float angle    = atan(dir.z, dir.x);
	float wiggle   = sin(angle * 3.0) * 0.15 + sin(angle * 7.3 + 1.2) * 0.06;
	float band_w   = exp(-(band_y + wiggle) * 6.0) * milkyway_strength;
	col           += milkyway_color * max(band, band_w);

	// ── 微細なノイズで宇宙の「霞」を表現 ────────────────────────────────
	float haze = hash1(dir * 30.0) * 0.008;
	col += milkyway_color * haze * max(0.0, 1.0 - band_y * 3.0);

	COLOR = col;
}
