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
| `client/scripts/main.gd` | 1614 | 🔴 god object。C-1着手で1件抽出済みだが残り12責務超が同居 |
| `client/scripts/ship_controller.gd` | 272 | 🟢 単一船の視覚表現に専念。結合なし |
| `client/scripts/connection.gd` | 245 | 🟢 WebSocket I/O とシグナル発行のみ。教科書的な境界 |
| `client/scripts/camera_controller.gd` | 124 | 🟢 自己完結したオービットカメラ |
| `client/scripts/ship_picking.gd` | 90 | 🟢 C-1で新設。`main.gd` からピッキング3関数を抽出した stateless static class |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画のみ |

合計 2,438 行のうち `main.gd` が66%を占める（抽出前69%から低下）。他5ファイルは合計824行で粒度・責務とも妥当。
（`client/test/*.gd`（main_test.gd 86行 + ship_picking_test.gd 109行）は別カウント。
シーンツリー無依存な純粋関数群と `ShipPicking` の全staticメソッドをGdUnit4でテスト済み
〔計15ケース〕。§「テストカバレッジ」参照）

---

## main.gd 内部構造（行範囲別）

| 行範囲 | 内容 | 評価 |
|---|---|---|
| 1–44 | ノード参照（`@onready`）・HUD パネル変数宣言 | 🟢 |
| 46–144 | 定数・内部状態（船/HP/モジュール/ゲート・天体配列/選択状態） | 🟡 ドメインごとに分類はされているが量が多い |
| 146–172 | `_ready()` / `_process()` | 🟡 `_process()` がカプ再現・近接判定・太陽方向・HUD更新の集約呼び出し点になっている |
| 173–300 | ゲート/天体マーカー生成 + スペクトル色テーブル | 🟢 共通部分（座標変換・子ノードクリア）は `_server_to_godot_pos`/`_clear_children` に抽出済み（C-2解消）。メッシュ生成自体は構造が異なるため未統合 |
| 302–360 | 太陽方向シェーダー更新・ゲート近接判定 | 🟢 |
| 362–420 | `_input()`（F1–F8 / S / J / A / W / Tab / クリック） | 🟡 60行の一枚岩 match。UI状態更新とコマンド送信が混在 |
| 471–530 | ダブルクリック判定・船/ゲート/天体ピッキング・選択 | 🟢 **C-1着手**: ピッキング3関数の本体を `ship_picking.gd`（`ShipPicking` class）へ移動済み。main.gd 側は候補データを渡すだけの薄いラッパー |
| 532–595 | ロックオン・移動・停止コマンド送信 | 🟢 |
| 597–918 | サーバーイベント dispatch（jump/system change/AoI/fitting/destroyed等） | 🟢 個々のハンドラは narrow |
| 920–971 | ワープ着地点計算（ゲート/天体） | 🟢 共通コア `_compute_warp_snap_pos_core` に抽出済み（C-2解消）。GdUnit4 テスト済み。係数は `GATE_WARP_ARRIVAL_FACTOR`/`BODY_WARP_ARRIVAL_FACTOR` で命名済み |
| 1050–1123 | HUD更新ディスパッチ + クライアント側カプ再現シミュレーション | 🟢 サーバーの `CapacitorSystem` を正しく模倣 |
| 1126–1205 | 状態クリア・空間環境構築・プレイヤーマテリアル | 🟢 |
| 1207–1583 | HUDパネル構築・更新（約377行・全体の23%） | 🔴 単独で `HudManager` 相当の規模。フレーム毎更新ロジックも同居 |
| 1586–1614 | デュエル結果オーバーレイ | 🟢 |

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
| `HudManager`（HUD構築・更新を専有） | 1207–1583 + `_update_hud()` | ~380行 | 未着手 |
| `NavigationMarkerRenderer`（ゲート/天体マーカー生成） | 173–300 | ~125行 | 未着手 |
| ~~`ShipPickingSystem`（船/ゲート/天体ピッキング統合）~~ | ~~477–558~~ | ~~~80行~~ | **完了** → `client/scripts/ship_picking.gd`（`ShipPicking` class, static methods）に抽出。`client/test/ship_picking_test.gd` で8ケースを Godot CLI 実行確認済み |
| `InputHandler`（キー/マウス入力ルーティング） | 362–420 + ダブルクリック判定 | ~80行 | 未着手 |

`ShipPickingSystem` 抽出で `main.gd` は 1661→1614 行（-47行）。残り3件を抽出すると
`main.gd` は ~700行（イベント dispatch + spawning + 状態保持のオーケストレーション層）
まで縮小できる見込み。

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

`client/test/main_test.gd` が `main.gd` のシーンツリー無依存な純粋関数 3個
（`_server_to_godot_pos` / `_spectral_color` / `_compute_warp_snap_pos_core`）を
計7ケースでカバー。`client/test/ship_picking_test.gd` が C-1 で抽出した
`ShipPicking`（`ray_point_distance` / `pick_ship_at` / `pick_gate_at` /
`pick_body_at`）を計8ケースでカバー。両方とも pin 済み Godot で実行確認済み
（計15ケース全件PASS）。

テスト実行中に実際の不具合を2回発見・修正している:
- `main_test.gd` 初回実行時: テストの `Node3D` をシーンツリーに追加し忘れ、
  `global_position` が `(0,0,0)` 固定で読めてしまい1件が偶然PASSしていた。
- `ship_picking_test.gd` 初回実行時: `class_name ShipPicking` を追加した直後は
  Godot のグローバルクラスキャッシュが未更新で `Identifier "ShipPicking" not
  declared` と全件失敗——上記「`class_name` 抽出の手順上の注意」を参照。

いずれも「Godot エディタなしでは検証できない」という旧来の想定が剥がれたことで
初めて見つかった類の不具合。詳細: `AI_DEVELOPMENT_GUIDE.md` §8。

マーカー生成のループ構造自体（C-1 で抽出予定の `NavigationMarkerRenderer`）は
今のところテスト対象外——シーンツリー・Node3D に依存し `ShipPicking` 同様の
パターンでテスト可能なはずだが、未着手。`HudManager`/`InputHandler` はテスト
よりも視覚的な確認（HUD レイアウト崩れなど）が主な検証手段になる領域。

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
| C-1 `main.gd` 分割（HudManager 等の抽出） | 構造 | **進行中**。`ShipPickingSystem`（→`ship_picking.gd`）抽出が完了（2026-06-20、テスト8件付き）。残り `NavigationMarkerRenderer` / `InputHandler` / `HudManager` は未着手。GDScript の構文・実行エラーは pin 済み Godot CLI + GdUnit4 で検証可能になったが、HUD の見た目・レイアウト崩れなど視覚的な確認は依然エディタが必要 |
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
- `tactical_overlay.gd` — 射程リング描画のみ。受け取った値を描くだけで状態を持たない
- `main.gd` のイベント dispatch 層（597–918行） — 個々のハンドラは narrow で ADR 参照付き。
  ここは god object 化した `main.gd` の中で唯一、責務分離が既にできている部分
