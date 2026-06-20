---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / 新スクリプト追加時
related  : docs/architecture-review-server.md（サーバー側）, docs/architecture.md, docs/playtest-guide.md
date     : 2026-06-20（GdUnit4 + pinned Godot CLI 導入後に更新）
---

# Architecture Review — Dawn Client (Godot)

サーバー側 [architecture-review-server.md](./architecture-review-server.md) のクライアント版。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: C+**

| 観点 | 評価 | 理由 |
|---|---|---|
| ファイル分割 | B− | connection / ship_controller / camera_controller / tactical_overlay の4本は単一責務で良好。`main.gd` 1本が突出して肥大化 |
| `main.gd` の責務集約 | D | 入力処理・HUD構築・サーバーメッセージ取り込み・マーカー生成・ワープ座標計算・カプ再現計算が単一クラスに集約（god object） |
| 重複 | C | マーカー生成（ゲート/天体）・ピッキング（船/ゲート/天体）・ワープ着地点計算（ゲート/天体）がそれぞれ同型ロジックの2重実装 |
| 結合度 | B− | signal 経由の `connection.gd` ↔ `main.gd` 結合は良好。`@onready` のシーンツリー直パス参照と modules dict のキー前提は脆い |
| デッドコード | A | 星系マップのサーバー化（直近リファクタ）後も残骸なし。コメントは ADR 参照付きで現状と一致 |
| サーバー側との対比 | — | サーバー側はクレート分割・ファイルサイズ管理が進んでいる（A−）一方、クライアントは1ファイル制（GDScript の歴史的経緯もあり）でこの観点が手薄 |

サーバー側が長期にわたる分割リファクタ（Phase 2〜9）を経て A− に達したのに対し、
クライアントは機能追加が `main.gd` に積み増される形で進んできており、責務分離が未着手の段階にある。

---

## ファイルサイズ一覧（2026-06-20 時点）

| ファイル | 行数 | 判定 |
|---|---|---|
| `client/scripts/main.gd` | 1482 | 🔴 god object。C-1着手で3件抽出済みだが残り9責務超が同居 |
| `client/scripts/ship_controller.gd` | 272 | 🟢 単一船の視覚表現に専念。結合なし |
| `client/scripts/connection.gd` | 245 | 🟢 WebSocket I/O とシグナル発行のみ。教科書的な境界 |
| `client/scripts/camera_controller.gd` | 124 | 🟢 自己完結したオービットカメラ |
| `client/scripts/navigation_marker_renderer.gd` | 146 | 🟢 C-1で新設。`main.gd` からゲート/天体マーカー生成 + スペクトル色を抽出した stateless static class |
| `client/scripts/input_decoder.gd` | 85 | 🟢 C-1で新設。キー入力→アクション決定の純粋関数のみを抽出した stateless static class |
| `client/scripts/ship_picking.gd` | 90 | 🟢 C-1で新設。`main.gd` からピッキング3関数を抽出した stateless static class |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画のみ |

合計 2,537 行のうち `main.gd` が58%を占める（抽出前69%から低下）。他7ファイルは合計1055行で粒度・責務とも妥当。
（`client/test/*.gd`（main_test.gd 68行 + ship_picking_test.gd 109行 +
navigation_marker_renderer_test.gd 112行 + input_decoder_test.gd 108行）は別カウント。
シーンツリー無依存な純粋関数群、`ShipPicking`、`NavigationMarkerRenderer`、
`InputDecoder` の全staticメソッドをGdUnit4でテスト済み〔計35ケース〕。
§「テストカバレッジ」参照）

---

## main.gd 内部構造（行範囲別）

| 行範囲 | 内容 | 評価 |
|---|---|---|
| 1–44 | ノード参照（`@onready`）・HUD パネル変数宣言 | 🟢 |
| 46–144 | 定数・内部状態（船/HP/モジュール/ゲート・天体配列/選択状態） | 🟡 ドメインごとに分類はされているが量が多い |
| 146–172 | `_ready()` / `_process()` | 🟡 `_process()` がカプ再現・近接判定・太陽方向・HUD更新の集約呼び出し点になっている |
| 173–197 | ゲート/天体マーカー生成（呼び出しのみ） | 🟢 **C-1着手**: 本体を `navigation_marker_renderer.gd`（`NavigationMarkerRenderer` class）へ移動済み。main.gd 側は候補データを渡すだけの薄いラッパー |
| 198–252 | 太陽方向シェーダー更新・ゲート近接判定 | 🟢 `NavigationMarkerRenderer.spectral_color()` を利用 |
| 253–322 | `_input()`（キー判定の決定は `InputDecoder` へ委譲・マウス処理は残置） | 🟡 **C-1着手**: キーボード分岐は `input_decoder.gd`（`InputDecoder.decode_key()`）へ移動。マウスのダブルクリック判定・HUD連動・ピッキング選択は状態を持つため main.gd に残置（判断の経緯は C-1 セクション参照） |
| 323–404 | ダブルクリック判定・船/ゲート/天体ピッキング・選択 | 🟢 ピッキング3関数の本体は `ship_picking.gd`（`ShipPicking` class）へ移動済み。main.gd 側は候補データを渡すだけの薄いラッパー |
| 405–463 | ロックオン・移動・停止コマンド送信 | 🟢 |
| 464–785 | サーバーイベント dispatch（jump/system change/AoI/fitting/destroyed等） | 🟢 個々のハンドラは narrow |
| 787–838 | ワープ着地点計算（ゲート/天体） | 🟢 共通コア `_compute_warp_snap_pos_core` に抽出済み（C-2解消）。GdUnit4 テスト済み。係数は `GATE_WARP_ARRIVAL_FACTOR`/`BODY_WARP_ARRIVAL_FACTOR` で命名済み |
| 918–991 | HUD更新ディスパッチ + クライアント側カプ再現シミュレーション | 🟢 サーバーの `CapacitorSystem` を正しく模倣 |
| 994–1073 | 状態クリア・空間環境構築・プレイヤーマテリアル | 🟢 |
| 1075–1451 | HUDパネル構築・更新（約377行・全体の25%） | 🔴 単独で `HudManager` 相当の規模。フレーム毎更新ロジックも同居 |
| 1454–1482 | デュエル結果オーバーレイ | 🟢 |

---

## 問題一覧

### High

#### C-1: `main.gd` の責務過多（god object） — 着手済み（2026-06-20）

13以上の異種責務（入力・HUD構築・メッセージ取り込み・マーカー生成・ピッキング・
ワープ座標計算・カプ再現・環境構築・デュエルUI）が単一クラスに集約している。
サーバー側で Phase 2〜7 を通じて解消した「肥大化した `node/mod.rs`」と同種の問題が、
クライアント側ではまだ大部分手つかずで残っている。

候補となる抽出先（優先度順）:

| 抽出候補 | 対象行 | 規模 | 状態 |
|---|---|---|---|
| `HudManager`（HUD構築・更新を専有） | 1075–1451 + `_update_hud()` | ~380行 | 未着手 |
| ~~`NavigationMarkerRenderer`（ゲート/天体マーカー生成）~~ | ~~173–300~~ | ~~~125行~~ | **完了** → `client/scripts/navigation_marker_renderer.gd`（`NavigationMarkerRenderer` class, static methods）に抽出。`_spectral_color` も移動（太陽方向更新からも参照）。`client/test/navigation_marker_renderer_test.gd` で8ケースを Godot CLI 実行確認済み |
| ~~`ShipPickingSystem`（船/ゲート/天体ピッキング統合）~~ | ~~477–558~~ | ~~~80行~~ | **完了** → `client/scripts/ship_picking.gd`（`ShipPicking` class, static methods）に抽出。`client/test/ship_picking_test.gd` で8ケースを Godot CLI 実行確認済み |
| ~~`InputHandler`（キー/マウス入力ルーティング）~~ | ~~253–311~~ | ~~~80行~~ | **部分完了（スコープを縮小）** → キーボード判定（F1–F8/S/J/A/W/Tab）のみ `client/scripts/input_decoder.gd`（`InputDecoder.decode_key()`、純粋関数）に抽出。マウスのダブルクリック判定・HUDモジュールスロット連動・ピッキング選択は main.gd に残置——理由は下記参照。`client/test/input_decoder_test.gd` で15ケースを Godot CLI 実行確認済み |

`ShipPickingSystem` + `NavigationMarkerRenderer` + `InputDecoder` 抽出で `main.gd` は
1661→1482 行（-179行）。残り1件（`HudManager`）を抽出すると `main.gd` は ~750行
（イベント dispatch + マウス入力 + spawning + 状態保持のオーケストレーション層）
まで縮小できる見込み。

**マウス入力ハンドラを抽出しなかった理由**: `InputHandler` を当初「キー/マウス両方」の
ルーティングとして見積もっていたが、マウスクリック側（`_check_double_click` の
タイマー状態・カメラドラッグ判定・`_module_slot_at` というまだ抽出していない HUD
コードへの依存・ピッキング結果からの選択状態書き込み）はキーボード側と違って
「入力 → 決定」のみに切り出せる純粋な形をしていない。無理に抽出すると
インターフェースが main.gd の状態をそのまま受け渡すだけの薄皮一枚になり、
実質的な結合は変わらず複雑さだけが増す（Altitude 原則: 場当たり的な分割ではなく
本質的な責務境界で分ける）。`HudManager` 抽出後に `_module_slot_at` の依存が外れれば
再評価する。

**`class_name` 抽出の手順上の注意**: 新しい `class_name` を追加した直後は、Godot が
プロジェクトを一度スキャンするまでグローバル識別子として認識されない
（`Identifier "X" not declared in the current scope` で CLI テストが失敗する）。
`scripts/setup-godot.*` で取得した Godot バイナリで以下を一度実行してキャッシュを
再構築すること:
```bash
"$GODOT_BIN" --headless --editor --quit-after 3 --path client
```

### Medium

#### C-2: 同型ロジックの2重実装（3箇所） — 解消済み（2026-06-20）

- ~~**マーカー生成**: `_spawn_gate_markers()` と `_spawn_body_markers()` は
  「子ノードクリア → 配列ループ → Node3D生成 → メッシュ/Label3D付与」の同一パターン。~~
  → 「子ノードクリア」を `_clear_children(root)` に、「サーバー座標 → Godot 座標変換」を
  `_server_to_godot_pos(p)` に抽出。メッシュ/Label3D の生成自体はゲート（トーラス）と
  天体（恒星/惑星で分岐するスフィア）で構造が異なるため統合せず、文字通り同一だった
  2箇所のみを共通化した。
- ~~**ピッキング**: `_pick_ship_at()` / `_pick_gate_at()` / `_pick_body_at()` は
  「カメラレイ取得 → 対象配列ループ → レイ距離判定 → 最近接を返す」の同一パターン。~~
  → レイと点の距離計算（`(p - from).dot(dir)` → 最近接点距離）を
  `_ray_point_distance(from, dir, p) -> Vector2(dist, t)` に抽出し、3関数すべてで使用。
  候補データの取得方法・pick radius の計算（固定値 / 天体半径ベース）は関数ごとに異なるため、
  ループ構造自体は統合せず数式部分のみ共通化した。
- ~~**ワープ着地点計算**: `_compute_warp_snap_pos()`（ゲート向け）と
  `_compute_body_warp_snap_pos()`（天体向け）は同一パターンの重複。~~
  → 方向ベクトル計算＋オフセット適用の共通部分を
  `_compute_warp_snap_pos_core(target_pos, radius, arrival_factor)` に抽出し、
  係数も `GATE_WARP_ARRIVAL_FACTOR = 0.75` / `BODY_WARP_ARRIVAL_FACTOR = 1.5` として
  named constant 化した。

3件とも挙動・公開シグネチャは変更なし。Callable/lambda ベースの汎用ヘルパー化
（候補データ + 取得関数を渡す形）は当時 Godot エディタでの動作確認ができず
見送ったが、**2026-06-20 に `scripts/setup-godot.(sh|ps1)` で pin 済み Godot
バイナリをローカル取得し、GdUnit4 を実際に CLI 実行できるようになった**ので
この制約は解消済み。実際、C-1 の `ShipPickingSystem` 抽出（`ship_picking.gd`）では
`pick_gate_at()` が座標変換用の `Callable`（`main.gd` の named method
`_server_to_godot_pos` を値として渡す）を受け取る形にした——インラインの
マルチラインラムダではなく既存メソッド参照を渡す形なら構文リスクが小さいことを
確認できた。

### テストカバレッジ（2026-06-20 追加・更新）

- `client/test/main_test.gd` — `main.gd` に残るシーンツリー無依存な純粋関数2個
  （`_server_to_godot_pos` / `_compute_warp_snap_pos_core`）を計4ケースでカバー。
- `client/test/ship_picking_test.gd` — `ShipPicking`（`ray_point_distance` /
  `pick_ship_at` / `pick_gate_at` / `pick_body_at`）を計8ケースでカバー。
  実際の `Camera3D` をシーンツリーに置いてレイキャストを検証。
- `client/test/navigation_marker_renderer_test.gd` — `NavigationMarkerRenderer`
  （`spectral_color` / `clear_children` / `spawn_gate_markers` /
  `spawn_body_markers`）を計8ケースでカバー。マーカー生成は子ノード数・ローカル
  座標・meta タグ・ラベル文字列で検証——`global_position` 不要なので
  シーンツリーに追加しなくてもテストできる。
- `client/test/input_decoder_test.gd` — `InputDecoder.decode_key()` を
  F1–F8/S/J/A/W/Tab の優先順位・ガード条件（`player_ship_id` 未設定時の挙動等）
  を含め計15ケースでカバー。ネットワーク・タイマー・シーンツリー一切不要——
  「キー + 選択状態 → アクション」の純粋な決定ロジックだけを切り出した効果。

4ファイル合計35ケース、pin 済み Godot で実行確認済み（全件PASS）。

テスト実行中に実際の不具合・ハマりどころを発見している:
- `main_test.gd` 初回実行時: テストの `Node3D` をシーンツリーに追加し忘れ、
  `global_position` が `(0,0,0)` 固定で読めてしまい1件が偶然PASSしていた。
- `class_name` を新規追加した直後（`ship_picking_test.gd` / `navigation_marker_
  renderer_test.gd` / `input_decoder_test.gd` でそれぞれ発生): Godot のグローバル
  クラスキャッシュが未更新で `Identifier "X" not declared` と全件失敗——上記
  「`class_name` 抽出の手順上の注意」を参照。3回発生して再現性が確認できたので、
  以後 `class_name` 抽出をするたびに必ず再発するものとして手順化済み。

いずれも「Godot エディタなしでは検証できない」という旧来の想定が剥がれたことで
初めて見つかった類の不具合。詳細: `AI_DEVELOPMENT_GUIDE.md` §8。

残る `HudManager` 抽出と、main.gd に残したマウス入力処理はテストよりも視覚的な確認
（HUD レイアウト崩れ・クリック判定のズレなど）が主な検証手段になる領域。

#### C-3: シーンツリー直パス参照の脆さ

`@onready` で `$Connection` / `$World/Ships` / `$World/Gates` / `$World/Bodies` /
`$HUD/StatsLabel` 等を直接パス参照しており、`main.tscn` のノード構成を変更すると
silent にではなく実行時エラーで気づく形になる（null チェックなし）。
現状ノード構成は安定しているため実害は小さいが、シーン構造変更時は要注意。

#### C-4: PlayerFitting dict のスキーマ非検証

`_player_modules` 配列の各要素は `"is_active"` / `"module_id"` / `"slot"` /
`"cap_cost_per_cycle"` / `"stat_delta"` 等の特定キーを前提に読まれるが、
`connection.gd` 側でのスキーマ検証はない。サーバー側 JSON 形式
（`serialization.rs` の `build_player_fitting_json()`）とキー名が食い違うと
silent に値が欠落する（GDScript の `Dictionary.get()` はデフォルト値で握り潰す）。

---

## 改善ロードマップ

### 進行中・未着手

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| C-1 `main.gd` 分割（HudManager 等の抽出） | 構造 | **進行中（3/4完了、うち1件はスコープ縮小）**。`ShipPickingSystem`（→`ship_picking.gd`）・`NavigationMarkerRenderer`（→`navigation_marker_renderer.gd`）・`InputDecoder`（→`input_decoder.gd`、キーボード判定のみ）抽出完了（2026-06-20、テスト31件付き）。残り `HudManager` は未着手。マウス入力処理は意図的に main.gd に残置（理由は C-1 セクション参照）。GDScript の構文・実行エラーは pin 済み Godot CLI + GdUnit4 で検証可能になったが、HUD の見た目・レイアウト崩れなど視覚的な確認は依然エディタが必要 |
| C-2 重複ロジックの共通化（マーカー/ピッキング/ワープ座標） | 品質 | **解消済み（2026-06-20）**。3件とも「文字通り同一」な計算式（座標変換・子ノードクリア・レイ距離・方向ベクトル）のみを named helper に抽出。ループ構造やメッシュ生成は個別性を残した |
| C-3 シーンツリー直パス参照 | 品質・保留 | 実害小。ノード構成変更が発生したときに合わせて対応すれば十分 |
| C-4 PlayerFitting スキーマ検証 | 品質・保留 | 現状ドリフトなし。サーバー側 JSON 形式を変更する ADR が出たときに合わせて対応 |

### 採らない方針

- main.gd を複数の `.tscn` 化されたコンポーネント（個別シーン+スクリプト）に分割することは、
  シーン参照切れのリスクが高い。pin 済み Godot CLI で構文・実行エラーは検出できるようになったが、
  シーンツリー構成の妥当性（ノードパスの解決・レイアウト）はヘッドレス実行だけでは確認しきれない。
  **分割は GDScript ファイル内の `class_name` 抽出（同一シーンに留める）からまず試す。**
  `class_name` 抽出後は GdUnit4 で実行確認できるので、この制約は以前より緩和されている。

---

## 触らない箇所（安定・枯れている）

- `connection.gd` — WebSocket I/O とシグナル発行のみ。ドメインロジックなし。教科書的な境界
- `ship_controller.gd` — 単一船の視覚表現に専念。他システムへの結合なし
- `camera_controller.gd` — 自己完結したオービットカメラ。依存はターゲットノード参照のみ
- `ship_picking.gd` — C-1 で新設した stateless static class。GdUnit4 テスト付きで
  挙動が固定されている。他クラスへの依存はメソッド引数経由のみ（Camera3D・候補データ・
  Callable）で、Godot のシーン構成変更の影響を受けない
- `navigation_marker_renderer.gd` — C-1 で新設した stateless static class。同上。
  メッシュ/Label3D 生成は main.tscn の `Gates`/`Bodies` ノード構成に依存しないため
  シーン変更の影響を受けない
- `input_decoder.gd` — C-1 で新設した stateless static class。同上。
  入出力ともプリミティブ値/Dictionaryのみで、main.gd の状態・シーンへの依存がない
- `tactical_overlay.gd` — 射程リング描画のみ。受け取った値を描くだけで状態を持たない
- `main.gd` のイベント dispatch 層（464–785行） — 個々のハンドラは narrow で ADR 参照付き。
  ここは god object 化した `main.gd` の中で唯一、責務分離が既にできている部分
