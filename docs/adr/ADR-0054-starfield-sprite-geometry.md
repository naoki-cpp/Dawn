---
id      : ADR-0054
title   : Starfield as CPU-generated sprite geometry
status  : accepted
date    : 2026-08-23
deciders: [human, ai-agent]
related : ADR-0053 (amends §3 — the sky shader no longer owns the star layer), ADR-0025 (local star direction stays in the sky shader), ADR-0044 (stars are direction-only and take no absolute f64 position), ADR-0055 (the nebula background, decided separately)
---

# ADR-0054 — Starfield as CPU-generated sprite geometry

## 背景

ADR-0053 §3 は星を `client/shaders/space_sky.gdshader` の内部に置いた。統計的な
星野をフラグメントシェーダで手続き生成し、明るい実在星は 16 スロットの uniform
配列（`catalog_directions` / `catalog_colors` / `catalog_brightness`、供給元は
`client/scripts/sky_catalog.gd`）で重ねる構成である。

2026-08-23 にこの星野をより実際の星空へ寄せる作業を行い、**点光源をフラグメント
シェーダで生成するという方式そのものに起因する問題**が 4 つ計測で確認された。

1. **float32 の桁落ち。** 角度カーネルを `1.0 - dot(dir, star_dir)` で作ると、両辺が
   ≈1.0 のため 32bit float では絶対誤差が ~1e-6 あり、**負に振れうる**。これに
   spread（10^6 オーダー）を掛けると `exp(-x)` が `exp(+large)` になる。実測では
   設計上の上限 3.8 に対し **linear 2140 超のピクセルが 1.48%** 発生し、白い塊として
   描画された。弦ベクトル `dot(dir - star_dir, dir - star_dir)` で回避したが、
   「毎ピクセルで鋭い角度カーネルを評価する」限りこの危険性の分類は残る。
2. **ハッシュのエントロピー喪失。** `fract(p * 443.8975)` 型のハッシュは入力の絶対値が
   大きいほど float の刻み幅が広がって情報を失う。fbm の深いオクターブ（座標が 1600 超）
   では **289万格子点に対し相異値 2,892**（χ²/dof 1082）まで縮退していた。整数格子
   ハッシュで解消したが、その代償が次項である。
3. **コストがピクセル数に比例し、星数に比例しない。** 等立体角リンググリッド + 3×3
   近傍サンプリングは **1 ピクセルあたり 27 セル評価**である。数千個の星を描くために
   毎フレーム 90 万ピクセル × 27 回のハッシュを回している。整数ハッシュ導入後の実測で
   sky パスは **43ms → 57ms（720p / Intel UHD、1.3 倍の回帰）**になった。
4. **サブピクセルのエイリアシング。** 解析的に描く点光源には mip もフィルタも無いため、
   σ を概ね 1px 以上に保たないとカメラ回転でちらつく。結果として**見た目が出力解像度に
   縛られる**。σ は 720p では 0.8px、1440p では 1.6px と変わってしまう。

一方 CCP の Trinity（EVE のレンダラ、2026 年 MIT 公開）はこの方式を採っていない。
[`trinity/Eve/EveStarfield.cpp`](https://github.com/carbonengine/trinity/blob/main/trinity/Eve/EveStarfield.cpp)
は seed 付き乱数から **CPU 側で一度だけ星を生成し、板ポリのスプライトとして頂点
バッファに積む**（既定 `m_starCount( 500 )`）。位置は面積保存の球面一様サンプリングで、
格子も極の偏りも原理的に生じない。

```cpp
float t = TriRand() * 2.0f * PI;
float u = ( TriRand() - 0.5f ) * 2.0f;
float sq = sqrtf( 1.0f - u * u );
star->position = Vector3( radius * sq * cosf( t ), radius * sq * sinf( t ), radius * u );
```

上記 4 つの問題は、いずれも「星をピクセルシェーダで生成する」という選択から派生
している。生成を CPU に移せば 4 つとも消える。

## 決定

**星野を sky shader から切り離し、CPU 生成のスプライト geometry として描く。**
ADR-0053 §3 のうち星に関する部分を本 ADR が改訂する。ADR-0025 の「ローカル恒星の
方向は sky shader が持つ」部分は変更しない。

### 1. 位置生成

固定 seed から一度だけ、面積保存の球面一様サンプリングで方向を作る。格子は使わない。

```gdscript
var u: float = rng.randf() * 2.0 - 1.0
var theta: float = rng.randf() * TAU
var sq: float = sqrt(1.0 - u * u)
var direction := Vector3(sq * cos(theta), u, sq * sin(theta))
```

### 2. 無限遠に置く — Trinity からの意図的な逸脱

Trinity は星を有限距離 100〜300 に置き（`Lerp( minDist, maxDist, dist * dist )`）、
視差を出している。**Dawn はこれを採用しない。** EVE の 1 シーンでは自艦は数 km しか
動かないが、Dawn は AU スケールを移動する（ADR-0029 / ADR-0044）。有限距離の星野は
そのスケールでは流れて破綻する。

Dawn の星野はカメラの**平行移動には追従し、回転には追従しない**ノードに親子付けし、
実質無限遠として扱う。星は方向のみを持ち、絶対 f64 座標を持たない（ADR-0044 の
権威座標には一切関与しない）。

### 3. スプライト

`MultiMeshInstance3D` + `QuadMesh`、`use_custom_data = true`。インスタンスごとに
スペクトル色、統計的な星野では相対的な合成 flux、実在星では等級由来の flux、星間塵への露出度を持たせ、ビルボード化と放射状の減衰は頂点／
フラグメントシェーダで行う。**テクスチャ資産は追加しない** — 減衰は quad の UV から
手続きで作れるので、`space_sky.gdshader` 冒頭の "No external textures required" と
いう性質を星野側でも保つ。

quad の画面上のサイズをピクセル単位で持つため、**画面上の最小フットプリントを 1px 以上に固定
できる**。これが (4) の解像度依存を解消する。

### 4. 旧方式: 分布は一様ではなく銀河構造に従う（2026-11-11 廃止）

この節は以前の angular-only 実装の履歴であり、現行実装の説明ではない。現行方式は
後述の「三次元銀河サンプリング」を参照する。

面積保存の一様サンプリングは極の偏りを消すが、それだけでは**壁紙のように平坦**に
見える。実際の空の奥行きは、まばらな近傍の星の向こうに、天の川という遠方の星の壁が
あることから生まれる。したがって生成は 3 群に分ける。

- 一様（残り）: 近傍の星。明るく、サイズも大きい。
- 銀河円盤 `DISC_FRACTION`: 銀河面からの高さを指数分布で引く。密度は
  `exp(-|height| * 4.5)` となり、シェーダの `disk` 項と一致する。flux は 0.42 倍。
- 銀河バルジ `BULGE_SHARE`（円盤分のうち）: 銀河座標 +X 方向へ Rayleigh 分布で
  集中。シェーダの `exp(-bulge_dist^2 * 5.0)` と一致する。flux は 0.30 倍。

星野は視差を持たない方向シェルのままだが、custom data の別成分で減光への露出度を表す。
近傍群はほぼ前景、円盤群は中程度、バルジ群は強く塵の影響を受ける。これは奥行きの描画上の
近似であり、実距離やサーバー座標を追加するものではない。名前付きカタログ星もこの範囲では
近傍群として扱う。

遠い群ほど暗くすることで、帯とバルジが「個々の星」ではなく「靄」として読める。
全ての星は共通のpoint spreadを使い、明暗はfluxで表現する。星ごとにサイズまで
変えて明るさを二重に強調しない。2026-11-12の参照比較ではGaussian coreをsigma
0.72pxに保ち、共通haloを加えた。初回設定はピーク強度2%・sigma 3px・quad半幅10pxだったが、
実プレイでは星点とにじみが過剰に見えたため、ピーク強度0.8%・sigma 2.4px・半幅8pxへ抑えた。
Gaussianを途中で切らずに裾を描画する。
これは参照コードのcore-plus-halo光学プロファイルを取り入れるもので、大気の
シンチレーションや星ごとの物理的な円盤を追加するものではない。
2026-11-11の背景調整では `DISC_FRACTION` を 0.55 から 0.66 へ上げて銀河面の密度を
強め、総数を 8,000 から 32,000 へ増やした。実プレイ確認では点星が過密でハローが目立ったため、
2026-11-12に総数を16,000へ下げ、共通ハローも縮小・減光した。比較用の実天体マップはローカルの無視対象
ディレクトリだけで使用し、製品には含めない。星の配置分布と塵による減光は、この背景表現に
限る。等級スケール・太陽の角サイズは変更しない。

#### 2026-11-11: 三次元銀河サンプリング

以下の旧実装説明にある「銀河面に沿う方向サンプリング」と固定dust exposureは廃止した。
観測者を銀河中心から約8.2 kpc、銀河面の近くに置き、近傍の前景星と、三次元位置から
投影した円盤・渦状腕・棒状バルジの星を生成する。円盤は面積要素を含む指数半径分布と
薄い円盤・厚い円盤の指数高さ分布を使う。代表的なthin/thick diskのscale length/height
（2.12/3.05 kpc、0.205/0.640 kpc）は[Barros et al. 2016の一つのモデル](https://www.aanda.org/articles/aa/pdf/2016/09/aa27535-15.pdf)
から初期値を取った。これは表示用サンプルであり、天の川の確定パラメーターではない。

実在星カタログと背景の向きは、任意の傾きではなく標準ICRS/赤道座標から銀河座標への回転で
合わせる（[Astropy Galactic frame](https://github.com/astropy/astropy/blob/main/astropy/coordinates/builtin_frames/galactic.py)）。星ごとに観測者からの距離を保持し、共通の
`GalacticModel` modelでその星までの `A_V` を積分して波長別に減光する。名前付き明るい星カタログには
距離データがないため、短い前景距離を暫定的に割り当てる。生成星の明るさは現状、見栄えのための
統計サンプルであり、恒星光度関数・年齢分布・距離減光に基づく測光合成ではない。腕の本数・ピッチ・
バルジ形状も視覚的近似で、実測カタログの再現を目的としない。

#### 2026-11-11: 恒星連続光と塵減光の共有モデル

点星と背景shaderが別々のdust係数を持たないよう、`client/scripts/galactic_model.gd` を係数の
唯一の定義元とする。LAMOSTベース滑らかdust diskのscale length 3.192 kpc、scale height 0.103 kpc、
太陽の高さ23 pc、拡散ISMの初期正規化 `0.7 A_V mag/kpc` を使う。累積可視等級 `A_V` を、
CCM89 `R_V=3.1` のRGB波長サンプルから透過率へ変換する。

背景は自由なコア色・nebula色の加算をやめ、5000 K黒体のRGB標本を代表的な恒星連続光として積分する。
これは一温度で置き換えた表示近似であり、完全な恒星人口合成ではない。発光ガスをシェーダノイズで
捏造せず、観測可能な電離ガスモデルが用意できるまで広域ガス発光は描かない。`display_radiance_scale` は
表示露出だけを調整し、密度や減光法則は変えない。

### 5. またたきは採用しない

Trinity は星ごとに `flashIntensity` / `flashPhase` / `flashRate` を持ちアニメーション
させる。**Dawn は採用しない。** シンチレーション（またたき）は大気による現象であり、
真空中の観測者には起こらない。ADR-0053 が掲げた「物理的に筋の通った presentation」
という方針に反するため、Trinity の中でこの 1 点だけ意図的に落とす。

### 6. 実在星カタログの統合

`SkyCatalog` の実在星は同じ MultiMesh のインスタンスになる。旧sky shaderにあった
16スロットのuniform配列というGPU都合の制約はなくなる。統計的な星野と実在星が
**同一の等級スケール**に載るため、「Sirius が手続き生成の星より暗い」（2026-08-23 に
発見した逆転）のような不整合が構造的に起こらなくなる。

### 7. sky shader に残るもの

`space_sky.gdshader` はネビュラ、天の川、ローカル恒星のディスク／コロナ／フレア、
ambient のみを担当する。`star_threshold` / `star_brightness` / `catalog_directions` /
`catalog_colors` / `catalog_brightness` の各 uniform と、`star_temperature()`・
星ループ・カタログループは削除する。

### 検討して落とした案

**現行の手続きシェーダ方式のまま調整だけ行う。** 背景 §1〜§4 の計測がそのまま
不採用の理由である。桁落ちの危険性の分類が残り、コストはピクセル数に比例し続け、
見た目が出力解像度に縛られたままになる。整数ハッシュによる 1.3 倍の性能回帰も
解消されない。

**`GPUParticles3D`。** 星野は完全に静的で、シミュレーションもライフタイムも不要で
ある。パーティクルシステムは毎フレームの process コストを払う分だけ無駄が大きい。

**星野を cubemap テクスチャに焼く。** 角度解像度が固定されるため点光源が必ずぼける。
Dawn が欲しいのは「1〜2px の鋭い点」なので、この方式は目的そのものを損なう。低周波な
コンテンツであるネビュラとは要件が逆であり、そちらは ADR-0055 で別途判断する。

## 実装チェックリスト

- [x] `client/scripts/starfield.gd` が固定 seed から方向・スペクトル色・等級を
      生成する純関数を持ち、GdUnit4 テストが seed 固定時の決定性を検証する。
- [x] 球面一様性のテストがある（例: 生成した方向の緯度分布が `sin(lat)` 一様に一致し、
      極付近に偏らないこと）。
- [x] またたき関連のパラメータが存在しないことをテストまたはレビューで確認する。
- [x] 銀河円盤・バルジへの集中がテストで検証されている（銀河面付近の密度が
      一様分布より高く、かつ遠方群の総 flux が低いこと）。
- [x] 星野ノードがカメラの平行移動に追従し回転に追従しないことをテストで検証する。
- [x] `SkyCatalog` の実在星が統計的な星野と同一の等級→flux 変換を通ることを
      テストで検証する（Sirius が最輝であること）。
- [x] `space_sky.gdshader` から `star_*` / `catalog_*` uniform と星ループが消えている。
- [x] `client/test/world_presentation_test.gd` の uniform 契約テストが新しい
      uniform 集合に更新されている。
- [x] sky パスのフレーム時間を変更前後で計測し、720p での回帰が無いことを記録する
      （Phase 11 §7 のフレームレート回帰確認を兼ねる）。720p / Intel UHD で
      HEAD 45.1ms に対し 45.8ms。計測は両条件ともスプライト星野を含む。
- [x] ADR-0053 の `related` に本 ADR を追記する。
- [x] `docs/process/roadmap/pending.md` の Phase 11 に本 ADR を参照する項目を置く。

## Open questions

- MultiMesh のインスタンス数の予算。過去の比較では 500 / 2,000 / 4,000 / 8,000 を
  描画し、500 は視野内に約 70 個しか入らず明らかに疎だった。視覚比較で32,000へ増やした後、
  実プレイで過密だったため16,000へ下げた。16,000件での性能計測は未完了であり、
  `Starfield.DEFAULT_COUNT` 一箇所で調整できる。
- 等級を実際の星表（Yale Bright Star Catalogue の V<6.5 部分など）から取り込むか、
  統計分布のままにするか。取り込む場合はデータのライセンスと
  `THIRD-PARTY-LICENSES.md` の扱いを別途確認する。
- 星野を AoI やタクティカルオーバーレイのピッキングから確実に除外する方法
  （`MultiMeshInstance3D` はレイキャスト対象にならないが、明示的に確認する）。

## 実装で判明したこと

- `VIEWPORT_SIZE` は Godot の spatial シェーダでは **fragment 段の組み込み**であり、
  `vertex()` では使えない。しかもコンパイルエラーにならず黙って quad が潰れ、星が
  1 個も出ない状態になる。ビューポート寸法は uniform で渡す必要がある
  （`viewport_pixels`、`WorldPresentation._update_starfield()` が毎フレーム更新）。
- `MultiMesh` のインスタンスデータは RenderingServer 側に置かれるため、
  `--headless` のダミーレンダラでは `get_instance_transform()` 等が 0 を返す。
  実デバイスでは正しく往復する。パッキングの検証は自動テストの対象外とし、
  実描画で確認する（`client/test/starfield_test.gd` にその旨を明記）。
