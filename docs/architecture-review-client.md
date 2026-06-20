---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / 新スクリプト追加時
related  : docs/architecture-review.md（サーバー側）, docs/architecture.md, docs/playtest-guide.md
date     : 2026-06-20
---

# Architecture Review — Dawn Client (Godot)

サーバー側 [architecture-review.md](./architecture-review.md) のクライアント版。
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
| `client/scripts/main.gd` | 1646 | 🔴 god object。13以上の異種責務が同居 |
| `client/scripts/ship_controller.gd` | 272 | 🟢 単一船の視覚表現に専念。結合なし |
| `client/scripts/connection.gd` | 245 | 🟢 WebSocket I/O とシグナル発行のみ。教科書的な境界 |
| `client/scripts/camera_controller.gd` | 124 | 🟢 自己完結したオービットカメラ |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画のみ |

合計 2,380 行のうち `main.gd` が69%を占める。他4ファイルは合計734行で粒度・責務とも妥当。

---

## main.gd 内部構造（行範囲別）

| 行範囲 | 内容 | 評価 |
|---|---|---|
| 1–44 | ノード参照（`@onready`）・HUD パネル変数宣言 | 🟢 |
| 46–138 | 定数・内部状態（船/HP/モジュール/ゲート・天体配列/選択状態） | 🟡 ドメインごとに分類はされているが量が多い |
| 141–166 | `_ready()` / `_process()` | 🟡 `_process()` がカプ再現・近接判定・太陽方向・HUD更新の集約呼び出し点になっている |
| 168–289 | ゲート/天体マーカー生成 + スペクトル色テーブル | 🟡 2関数が同型パターンの重複（後述） |
| 291–346 | 太陽方向シェーダー更新・ゲート近接判定 | 🟢 |
| 348–404 | `_input()`（F1–F8 / S / J / A / W / Tab / クリック） | 🟡 60行の一枚岩 match。UI状態更新とコマンド送信が混在 |
| 436–565 | ダブルクリック判定・船/ゲート/天体ピッキング・選択 | 🟡 ピッキング3関数が同型パターンの重複 |
| 567–624 | ロックオン・移動・停止コマンド送信 | 🟢 |
| 626–1078 | サーバーイベント dispatch（jump/system change/AoI/fitting/destroyed等） | 🟢 個々のハンドラは narrow で十分テスト可能 |
| 958–1003 | ワープ着地点計算（ゲート/天体） | 🟡 2関数が同型パターンの重複。係数（0.75 / 1.5）が無名マジックナンバー |
| 1080–1154 | HUD更新ディスパッチ + クライアント側カプ再現シミュレーション | 🟢 サーバーの `CapacitorSystem` を正しく模倣 |
| 1157–1237 | 状態クリア・空間環境構築・プレイヤーマテリアル | 🟢 |
| 1239–1615 | HUDパネル構築・更新（377行・全体の23%） | 🔴 単独で `HudManager` 相当の規模。フレーム毎更新ロジックも同居 |
| 1617–1646 | デュエル結果オーバーレイ | 🟢 |

---

## 問題一覧

### High

#### C-1: `main.gd` の責務過多（god object）

13以上の異種責務（入力・HUD構築・メッセージ取り込み・マーカー生成・ピッキング・
ワープ座標計算・カプ再現・環境構築・デュエルUI）が単一クラスに集約している。
サーバー側で Phase 2〜7 を通じて解消した「肥大化した `node/mod.rs`」と同種の問題が、
クライアント側ではまだ手つかずで残っている。

候補となる抽出先（優先度順）:

| 抽出候補 | 対象行 | 規模 |
|---|---|---|
| `HudManager`（HUD構築・更新を専有） | 1239–1615 + `_update_hud()` | ~390行 |
| `NavigationMarkerRenderer`（ゲート/天体マーカー生成） | 168–289 | ~120行 |
| `ShipPickingSystem`（船/ゲート/天体ピッキング統合） | 457–565 | ~110行 |
| `InputHandler`（キー/マウス入力ルーティング） | 348–404 + ダブルクリック判定 | ~80行 |

これらを抽出すると `main.gd` は ~700行（イベント dispatch + spawning + 状態保持の
オーケストレーション層）まで縮小できる見込み。

### Medium

#### C-2: 同型ロジックの2重実装（3箇所）

- **マーカー生成**: `_spawn_gate_markers()`（168–205）と `_spawn_body_markers()`（209–269）は
  「子ノードクリア → 配列ループ → Node3D生成 → メッシュ/Label3D付与」の同一パターン。
- **ピッキング**: `_pick_ship_at()` / `_pick_gate_at()` / `_pick_body_at()`（457–565）は
  「カメラレイ取得 → 対象配列ループ → レイ距離判定 → 最近接を返す」の同一パターン。
- ~~**ワープ着地点計算**: `_compute_warp_snap_pos()`（ゲート向け）と
  `_compute_body_warp_snap_pos()`（天体向け）は同一パターンの重複。~~
  → **解消済み（2026-06-20）**: 方向ベクトル計算＋オフセット適用の共通部分を
  `_compute_warp_snap_pos_core(target_pos, radius, arrival_factor)` に抽出し、
  係数も `GATE_WARP_ARRIVAL_FACTOR = 0.75` / `BODY_WARP_ARRIVAL_FACTOR = 1.5` として
  named constant 化した。挙動・公開シグネチャは変更なし。

マーカー生成・ピッキングの2件は「対象データ + 取得関数（lambda/callback）」を受け取る
共通ヘルパーへ統合可能だが、シーンツリーの挙動に直結するため Godot エディタでの
動作確認なしに進めるのはリスクが高く、今回は見送る（ロードマップ参照）。

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

### 未着手（このレビューで新規に提起）

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| C-1 `main.gd` 分割（HudManager 等の抽出） | 構造 | 未着手。優先度高だが GDScript の `class_name` 別ファイル化は Godot エディタでの動作確認が必要なため、AI セッション単独では検証しにくい |
| C-2 重複ロジックの共通化（マーカー/ピッキング/ワープ座標） | 品質 | ワープ座標計算のみ解消済み（2026-06-20）。マーカー生成・ピッキングはエディタ確認なしのリスクを優先し見送り |
| C-3 シーンツリー直パス参照 | 品質・保留 | 実害小。ノード構成変更が発生したときに合わせて対応すれば十分 |
| C-4 PlayerFitting スキーマ検証 | 品質・保留 | 現状ドリフトなし。サーバー側 JSON 形式を変更する ADR が出たときに合わせて対応 |

### 採らない方針

- main.gd を複数の `.tscn` 化されたコンポーネント（個別シーン+スクリプト）に分割することは、
  Godot エディタでの動作確認なしに AI セッション単独で進めるとシーン参照切れのリスクが高い。
  **分割は GDScript ファイル内の `class_name` 抽出（同一シーンに留める）からまず試す。**

---

## 触らない箇所（安定・枯れている）

- `connection.gd` — WebSocket I/O とシグナル発行のみ。ドメインロジックなし。教科書的な境界
- `ship_controller.gd` — 単一船の視覚表現に専念。他システムへの結合なし
- `camera_controller.gd` — 自己完結したオービットカメラ。依存はターゲットノード参照のみ
- `tactical_overlay.gd` — 射程リング描画のみ。受け取った値を描くだけで状態を持たない
- `main.gd` のイベント dispatch 層（626–1078行） — 個々のハンドラは narrow で ADR 参照付き。
  ここは god object 化した `main.gd` の中で唯一、責務分離が既にできている部分
