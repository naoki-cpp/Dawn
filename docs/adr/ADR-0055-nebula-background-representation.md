---
id      : ADR-0055
title   : Nebula background — bake the procedural sky, then light distance with it
status  : accepted
date    : 2026-08-23
deciders: [human, ai-agent]
related : ADR-0053 (amends the "procedural Milky Way and nebula approximation" part of §3), ADR-0054 (the star layer, decided separately), ADR-0016 (game vision — per-system identity as a design goal)
---

# ADR-0055 — Nebula background representation

## 背景

`client/shaders/space_sky.gdshader` は現在、ネビュラと天の川をフラグメントシェーダ内の
fbm ノイズで生成している。1 ピクセルあたり **fbm を 7 回**呼ぶ（天の川の clump、広域
ダスト、輝線 3 領域 `n1`/`n2`/`n3`、ダスト吸収）。fbm は 5 オクターブ × snoise 8 コーナー
なので、**1 ピクセルあたり約 280 回のハッシュ評価**になる。720p / Intel UHD での実測で
sky パスは 43ms（約 23fps）を占めており、これは星野を除いた背景だけの数字である。

視覚面でも次の制約がある。

- **全 Sector で同じ絵になる。** ノイズは方向のみの関数なので、どの星系に行っても
  同じネビュラが見える。ADR-0016 が掲げるプレイヤー主導の territory において、
  「どの星系にいるか一目で分かる」ことは失われている。
- **色数と構造が uniform 数に縛られる。** 2026-08-23 の作業で
  `nebula_primary_color` / `nebula_secondary_color` / `nebula_highlight_color` の 3 色を
  uniform 化したが、絵作りの自由度は依然として「3 色 × ノイズしきい値」に留まる。
- **ADR-0053 の留保がそのまま残っている。** 同 ADR は「procedural nebula noise が
  measured survey ではない」と明記しており、手続き生成である限りこの留保は消えない。

CCP の Trinity（EVE のレンダラ、2026 年 MIT 公開）は手続きノイズを使っていない。

- `trinity/Eve/EveSpaceScene.cpp` はネビュラを **環境キューブマップ**として持つ
  （コード内コメント: "the environment cubemap aka the nebula"）。星系ごとの
  アート資産である。`EveSpaceScene.h` に `m_lowQualityNebulaResPath` があり、
  低品質版を別途持つ LOD 構成になっている。
- `m_nebulaIntensity` がシーン単位でスケールを持ち、`backgroundReflectionIntensity`
  として**船体の反射にも同じキューブマップが入る**。
- `trinity/PostProcess/Effects/Tr2PPFogEffect.cpp` がネビュラの**ぼかした mip
  （既定 `m_nebulaBlur( 7.0 )`）を画面全体に乗せる**（`m_nebulaInfluence( 0.5 )`）。
  EVE の背景があの絵画的な色をしていて、かつ手前の船とも色が馴染んでいるのは、
  この post-process が理由である。

## Trinity のソースを読んで分かった構造（2026-08-23 追記）

初版は「EVE はキューブマップを使っている」以上のことを書けていなかった。
`trinity/Eve/EveSpaceScene.cpp` と `EveSpaceScene_Blue.cpp` を読み、選択肢 B の
コスト見積もりを変える 3 点が判明した。

### 1. ネビュラは装飾ではなく、シーンの環境マップそのもの

`envMapResPath`（"Resource path for the scene's environment map aka the nebula"）
が読み込むキューブマップ 1 枚が、同時に次の 3 役を担う。

- 背景の描画（`DrawCameraSpaceScreenQuad` によるカメラ空間フルスクリーンquad）
- 船体の反射（`m_reflectionMapVar = m_envMap1`）
- 環境光

背景と手前のオブジェクトの色が馴染むのは、両者が同じ 1 枚を見ているからである。
**Dawn は既に同じ構造を持っている**: `world_presentation.gd` は
`ambient_light_source = AMBIENT_SOURCE_SKY` と `radiance_size = RADIANCE_SIZE_256`
を設定しており、Godot は `Sky` から放射キューブマップを生成して環境光と反射に使う。
つまり選択肢 B は**既存の差し込み口をそのまま使える**——`Sky` の material を
手続きシェーダからキューブマップ参照に替えるだけで、反射と環境光は自動的に追従する。
統合コストは初版の見積もりより小さい。

### 2. 星系ごとの識別性は「星系ごとに 1 枚描く」ことでは得ていない

`envMapRotation`（"Texture transform rotation applied to all envMaps"）が
シーン単位で公開されている。同じキューブマップを回転・反転させ、`nebulaIntensity` と
組み合わせることで多数の星系に使い回す。

これが初版の Open question「星系ごとのネビュラを持つ場合のアセット量」に対する答えに
なる。必要なのは**少数のキューブマップ × 回転 × 強度**であり、星系数ぶんの絵ではない。
アセットパイプラインの重さの見積もりが一桁変わる。

### 3. 品質段階が最初から組み込まれている

`lowQualityNebulaResPath` と `lowQualityNebulaMixResPath` が別リソースとして
存在し、`externalParameter: LQ_Nebula` に紐づく。低スペック環境向けの差し替えを
後付けではなく最初から資産側に持たせている。

## 決定

**選択肢 C を採る。手続き生成はそのまま残し、実行時に一度だけ等距円筒パノラマへ
焼き、以降はそれをサンプルする。**

ネビュラと天の川は方向のみの関数なので、毎フレーム毎ピクセルで評価する理由が無い。
`RenderingServer.sky_bake_panorama()` で 2048x1024 のパノラマを生成し、
`space_sky.gdshader` はテクスチャフェッチ 1 回に置き換える。実測で **1 ピクセル
あたり約 280 回のハッシュ評価が消え、sky パスは 42.4ms → 19.0ms（720p /
Intel UHD、2.23 倍）**になった。焼く前後の描画差は平均 0.0005 で、見た目は変わらない。

選択肢 B（外部アート資産のキューブマップ）ではなく C を選ぶ理由は、B が
アセットパイプラインという未決の前提に依存するのに対し、C は**その前提抜きで
EVE と同じ実行時構造**（背景・反射・環境光が 1 枚を共有する）を先に手に入れられる
からである。後から資産が用意できるなら、差し替え先は同じ `Sky` の material であり、
本 ADR の実装は B への移行を妨げない。

### 焼かないもの

ローカル恒星（ディスク・コロナ・フレア）は自艦の移動で方向が変わるため、方向のみの
背景には決して含められない。シェーダ内で分岐し、恒星は常にライブで描く。焼き付け中に
恒星が既に点灯していた場合は焼くのをやめ、手続き経路のまま留まる。

### フォールバック

手続き経路はシェーダから削除せず `use_baked_nebula` の分岐に残す。
`sky_bake_panorama()` はヘッドレス／ダミーレンダラでは何も返さないため、
焼けなかった場合に空が真っ黒になることを防ぐ。

### 選択肢 A — 現行の手続きノイズを維持し、パラメータのみ調整する

追加資産ゼロ。`nebula_strength` を下げる程度の調整で「宇宙らしい黒」は戻る。
星系ごとの識別性とコストの問題は残る。

### 選択肢 B — 事前生成キューブマップ（EVE 方式）

星系ごとにネビュラのキューブマップを持ち、`Sky` の背景として直接サンプルする。
毎ピクセルの fbm 280 回がテクスチャフェッチ 1 回に置き換わるため、**背景のコストは
劇的に下がる**。星系ごとの絵作りが可能になり、船体反射にも同じ環境マップを流用できる。

代償は**アセットパイプラインが必要になること**である。Dawn には現在テクスチャ資産の
生成・格納・配布の仕組みが無い（ADR-0053 は "A future texture/asset pipeline can
replace the procedural planet material" と、その不在を前提に書かれている）。
Phase 11 §1「船種ごとの glTF 3Dモデル」も同じ「調達方針が必要」という状態で止まって
おり、**この 2 つは同じ意思決定の下流にある**。

### 選択肢 C — 手続き生成を一度だけ焼く（採用）

現在の fbm を実行時ではなくビルド時（または初回起動時）に走らせ、キューブマップへ
焼く。星系ごとに seed を変えれば識別性も得られる。外部アート資産は不要なので
パイプラインの重さは B より小さい。ただし「手続き生成の見た目」の限界はそのまま
残り、EVE のような絵画的な質にはならない。

## 判断材料の非対称性についての注記

`docs/reference/carbon-engine-comparison.md` が定めた読み方をここでも適用する。
EVE は 20 年以上の本番運用の末にキューブマップ + post-process 合成という構成に
たどり着いており、これは当て推量ではない。一方 Dawn の手続きノイズは**一度も
プレイテストを経ていない**。「Dawn は手続き生成を選んだ」ことと「その選択が
正しい」ことは別である。現時点で手続き生成を支持する根拠は「資産パイプラインが
まだ無い」という実装都合のみであり、絵作りの観点では EVE 方式が優位である。

## Open questions

採用した選択肢 C の範囲外として残るもの。選択肢 B へ進むときに再度問われる。

- Dawn にテクスチャ／モデルのアセットパイプラインを持つのか。持つなら Phase 11 §1
  （glTF 船モデル）と本 ADR は同じ基盤の上に乗るので、**先にパイプラインの ADR が要る**。
- ~~星系ごとのネビュラのアセット量~~ → 上記 §2 で解決。少数のキューブマップを
  回転・強度で使い回す。残る問いは次の 1 点。
- 星系ごとのネビュラを持つ場合、どのデータが「どの星系か」をクライアントに伝えるのか。
  現在 `ServerFact` に Sector の視覚的アイデンティティを表す項目は無い。追加するなら
  wire schema の変更であり、ADR-0042 系の手続き（`gen_wire_schema` 再生成）が必要になる。
- ~~ネビュラを船体反射に流用するか~~ → 上記 §1 で解決。Godot の `Sky` 放射
  キューブマップ経由で自動的に流用される。EVE の `backgroundReflectionIntensity`
  に相当する反射だけを別倍率にする機能を持つかどうかは残る問い。
- ~~post-process のネビュラ合成（`Tr2PPFogEffect` 相当）まで追うか~~ → 追った。
  下記 §空気遠近法を参照。

## 空気遠近法（`Tr2PPFogEffect` 相当）

EVE の `Tr2PPFogEffect` は「ぼかしたネビュラを画面に乗せる」だけの効果ではなく、
**距離に応じた空気遠近法**である。`EnvironmentFogColor.fx` がネビュラの mip 7
（`nebulaBlur 7.0`）と定数色から霧色を作り、`EnvironmentFogComposit.fx` が
2,000 / 25,000 / 120,000 の 3 距離帯で本編に混ぜる。遠いものほど背景色へ寄る。

**Godot は同じ考え方を `Environment` の標準機能として持っている**ため、
CompositorEffect を書く必要はなかった。

| Godot | EVE の対応物 |
|---|---|
| `fog_aerial_perspective` | `nebulaInfluence` — 霧色を空の色へ寄せる |
| `fog_depth_begin` / `end` / `curve` | `blendDistance0/1/2` |
| `fog_light_color` / `fog_density` | `Color` / `totalAmount` |
| `fog_sky_affect` | `backgroundOcclusion` |

距離は EVE と同じ 2,000 m 〜 120,000 m を採り、`WorldSpace::render_scale()` で
換算する。近距離戦闘（数 km）は `fog_depth_begin` より手前なので影響を受けない。

**星野は `fog_disabled` で除外する。** スプライトの殻は 4,000,000 レンダー単位に
あり、`fog_depth_end` の数百倍遠いため、除外しないと全天が霧色に潰れる。

### 物理的な位置づけ

真空に空気遠近法は生じないので、これは ADR-0053 が掲げた「物理的に筋の通った
presentation」の例外にあたる——ただし完全な作り物ではない。星間ダストとネビュラの
ガスは実際に遠方の光を散乱・赤化させるので、**Sector がネビュラの中にあるという
前提の下では物理的な裏付けがある**。その前提を外れる絵作りに使うなら再検討する。

## 実装チェックリスト

- [x] `space_sky.gdshader` が `nebula_panorama` / `use_baked_nebula` /
      `bake_pass` を宣言し、手続き経路をフォールバックとして保持している。
- [x] `WorldPresentation._maybe_bake_nebula()` が焼き付けを 2 段階で行う。
- [x] 恒星が点灯済みなら焼かずに手続き経路へ留まることをテストで検証する。
- [x] 焼き付けが同フレームで走らないこと、レンダラが画像を返さない場合に
      フォールバックすることをテストで検証する。
- [x] uniform 契約テストが新しい 3 uniform を含む。
- [x] 焼き付け前後の描画差と sky パスのフレーム時間を実測で記録する
      （差分平均 0.0005、42.4ms → 19.0ms）。
- [x] `Environment` が `FOG_MODE_DEPTH` + `fog_aerial_perspective` で構成され、
      距離が `render_scale()` 経由で換算されている。
- [x] 近距離戦闘の間合いが `fog_depth_begin` より手前であることをテストで固定する。
- [x] `star_sprite.gdshader` が `fog_disabled` を宣言していることをテストで固定する。
- [x] 空気遠近法の効きを実測で記録する（角サイズ一定の球で 115 km 時
      0.85 → 0.65、暖色シフトあり。1 km では変化なし）。

## 実装で判明したこと

`RenderingServer.sky_bake_panorama()` について、文書からは読み取れなかった 3 点。

- **生きた `WorldEnvironment` に接続された `Sky` でなければ空の画像が返る。**
  単独で生成した `Sky` の RID を渡しても、サイズは正しいが全ピクセル 0 になる。
- **同フレームに書いた uniform は反映されない。** `bake_pass` を立てた直後に焼くと
  立てる前の状態が焼かれる。今回はビネットが焼き込まれ、実行時に二重に掛かった。
- **等距円筒の U 軸は `atan(-dir.x, -dir.z) / TAU`。** u=0 が -Z、0.25 が -X、
  0.5 が +Z、0.75 が +X。符号を取り違えると空の別の場所を読むが、ノイズ背景では
  破綻として見えず、単に「少し暗い」ように見えるだけで発見しにくい。方向を色に
  焼いたパノラマを読んで規約を実測するのが確実。
