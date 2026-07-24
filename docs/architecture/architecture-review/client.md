---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 構造評価
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / 新スクリプト追加時
related  : docs/architecture/architecture-review/server.md（サーバー側）, docs/architecture/architecture.md, docs/process/playtest-guide.md,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-07-17（定期再計測 その5。PR #143のtyped HUD refsを反映。`hud_manager.gd` は789→892、`main.gd` は1217→1219、client/scripts合計は4924行。C-9を再観測としてclient-pendingへ戻し、GdUnit4 202ケース・テストコード2838行を確認。client総合はA−へ更新）
---

# Architecture Review — Dawn Client（Godot・構造評価）

サーバー側 [server.md](./server.md) のクライアント版。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

このファイルは「今どういう状態か」（グレード・ファイルサイズ・issue ID の登録簿）を扱う。
未完項目・保留判断は
[client-pending.md](./client-pending.md)、
解消済みの作業ログは
[client-completed.md](./client-completed.md) を参照。

> `client/scripts/*.gd` のコード内コメントは issue ID（例 `C-1`）でこのファイルを参照している
> （`client.md C-1` の形）。ID の登録簿はこのファイルに残し続けること —
> 完了済みログへ移しても構わないが、その場合はコード側の参照コメントも更新すること。

---

## 現状評価

**総合: A−**（2026-07-17 再計測。責務分担・結合度は健全で、`main.gd` のgod object化も再発していない。
ただしPR #143のtyped HUD refs追加で `hud_manager.gd` が789→892行となり、C-9のwatch帯へ再到達した。
この増加はHUD panel refsの型安全化という一貫した責務によるものだが、ファイルサイズ評価は一段下げる。
`main.gd` は1219行、client/scripts合計は4924行。GdUnit4ケース数は202、テストコード総量は2838行で、
既存の分割モジュールとテスト可能性は維持されている）

| 観点 | 評価 | 理由 |
|---|---|---|
| ファイル分割 | A | `main.gd` から `HudManager`/`HudSurface`/`NavigationMarkerRenderer`/`ShipPicking`/`InputDecoder`/`WorldSession`/`WorldInteraction`/`WorldPresentation` を抽出。live world state は `WorldSession`、live HUD Control 参照は `HudSurface`、world interaction policy は `WorldInteraction`、world visual side effect は `WorldPresentation` が所有 |
| `main.gd` の責務集約 | A | god object は実質解消。selection state・ダブルクリック・world selection 優先順位・dock/undock を含む action gating は `WorldInteraction` へ、floating origin / nav marker placement / sky sun update / warp tunnel / player ship presentation は `WorldPresentation` へ移動済み。`main.gd` に残るのは scene lifecycle / scene node generation / event dispatch / network send / HUD frame assembly。2026-07-08、複数所有船ロスター（SHIPS列）・右クリックでのTransferToStation送信ハンドラ・ドラッグ&ドロップ状態機械・Disembark（Xキー）を追加（1056→1219）が、既存の orchestration 層に自然に収まった |
| 重複 | A− | マーカー生成・ピッキング・ワープ着地点計算の同型ロジックは解消済み（C-2） |
| 結合度 | A | signal 経由の `connection.gd` ↔ `main.gd` 結合は良好。`@onready` のシーンツリー直パス参照はフェイルファストガードで解消（C-3）。modules/inventory dict のキー前提の脆さ（C-4）は `ModuleRow`/`ItemRow` typed row 化で解消済み。2026-07-08、`hud_manager.gd` のインベントリ行 Dictionary も `InventoryRow` typed class 化で解消（C-8） |
| デッドコード | A | 残骸なし。コメントは ADR 参照付きで現状と一致 |
| テストカバレッジ | A− | 新設クラス + main.gd残存ロジックの一部を GdUnit4 で計202ケース実行確認済み（2026-07-11実測・0 errors/0 failures/0 orphans。ADR-0042で`ClientMessageDecoder`/`hello_command`テストを追加し186→202）。`WorldSession` / `HudSurface` / `WorldInteraction` / `WorldPresentation` は引き続き scene tree なしで単体テスト可能。scene-tree/ネットワーク依存の end-to-end 入力経路（ドラッグ&ドロップの実際のマウス操作等）だけが手動確認領域として残る |
| サーバー側との対比 | — | サーバー側はクレート分割（A−）、クライアントはファイル分割（A）。テストカバレッジは依然サーバー側（カバレッジ80%要件）が厚い |

サーバー側が長期にわたる分割リファクタ（Phase 2〜9）を経て A− に達したのに対し、
クライアントは 2026-06-20 の C-1 で初めて本格的な責務分離に着手し、4クラスの抽出と
GdUnit4 テスト基盤の整備（`scripts/setup-godot.*` による pin 済み Godot CLI）を完了した。

---

## 最新ファイルサイズ一覧（2026-07-17 再計測）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `client/scripts/main.gd` | 1219 | 🟢 orchestration層。scene lifecycle / node generation / event dispatch / network send / HUD frame assembly |
| `client/scripts/hud_manager.gd` | 892 | 🟡 C-9再観測。HUD panel構築・更新とtyped refsに責務は揃うが、850行watch帯へ再到達 |
| `client/scripts/world_session.gd` | 358 | 🟢 live world state専任 |
| `client/scripts/connection.gd` | 346 | 🟢 WebSocket I/Oとsignal発行のみ |
| `client/scripts/ship_controller.gd` | 342 | 🟢 単一船の視覚表現に専念 |
| `client/scripts/world_presentation.gd` | 311 | 🟢 world visual side effect専任 |
| `client/scripts/hud_surface.gd` | 233 | 🟢 HUD Control refsとrender frame委譲 |
| `client/scripts/navigation_marker_renderer.gd` | 227 | 🟢 marker生成専任 |
| `client/scripts/input_decoder.gd` | 158 | 🟢 input factsへの変換を行う純粋関数 |
| `client/scripts/camera_controller.gd` | 142 | 🟢 orbit camera専任 |
| `client/scripts/world_interaction.gd` | 133 | 🟢 selection / click→intent / key actionのdeep module |
| `client/scripts/ship_picking.gd` | 104 | 🟢 画面空間ピッキング専任 |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画専任 |
| `client/scripts/inventory_row.gd` | 90 | 🟢 typed inventory row shape |
| `crates/dawn-client-core/src/world_space.rs` | — | 🟢 absolute f64 / floating-origin coordinate model（2026-07-24 Rust移管） |
| `crates/dawn-client-gdext/src/world_space_gd.rs` | — | 🟢 WorldSpaceのGodot型アダプター（最終Vector3変換のみ） |
| `client/scripts/hud_hit_test.gd` | 80 | 🟢 HUD画面座標hit-test専任 |
| `client/scripts/billboard_ring.gd` | 65 | 🟢 selection ring共通処理 |
| `client/scripts/unit_format.gd` | 38 | 🟢 単位整形専任 |
| `client/scripts/warp_tunnel_effect.gd` | 10 | 🟢 warp tunnel表示ラッパー |

**client/scripts合計: 4,924行。** C-9の再到達は、hit-testの責務逆流ではなくPR #143のtyped refs追加による。
`hud_manager.gd`を直ちに分割せず、型定義とpanel更新のcohesionを維持したまま、次のHUD機能追加で責務が
複数の変更理由へ分かれた時点を再評価トリガーとする。

## 前回ファイルサイズ一覧（2026-07-10 時点・履歴）

> **2026-07-10、全ファイル再計測（`/architecture-review`）。** 現在の実測は
> `main.gd` 1217・`hud_manager.gd` 789（C-9解消、850から-61）・`hud_hit_test.gd` 88（新設）・
> `connection.gd` 374・`world_session.gd` 358・
> `ship_controller.gd` 342・`navigation_marker_renderer.gd` 227・`inventory_row.gd` 90・
> `hud_surface.gd` 234・`input_decoder.gd` 158・`camera_controller.gd` 142・
> `world_interaction.gd` 133・`world_presentation.gd` 311。`input_decoder.gd` は
> `I`キーのshipless soft-lock修正（147→158）。`main.gd`/`connection.gd`（+7/+5）は
> `player_fitting_received`シグナルを生JSON文字列渡しに変更した修正（PR #127末尾コミット、
> Dictionary再エンコードによる整数→浮動小数点化バグの修正）の反映漏れ。
> `hud_manager.gd`/`world_session.gd`/`hud_surface.gd`（-2/+1/+1）は誤差レベルの再計測drift。
>
> **同日、`player_loadout.gd`/`module_row.gd`/`item_row.gd` を削除**（ADR-0039/ADR-0040）。
> Godot 非依存のドメインロジック（capacitor シミュレーション・武器射程計算・PlayerLoadout
> wire row 型）は新設 `dawn-client-core`（純粋 Rust、`cargo test` 対象）へ移植し、
> `dawn-client-gdext`（GDExtension バインディング）が `PlayerLoadout`/`ModuleRow`/`ItemRow`
> という同名のグローバルクラスとして GDScript へ公開する。フィールド名・`equals()`/`clone()`を
> 完全一致させたため、`hud_manager.gd`/`hud_surface.gd`/`world_session.gd` 側の変更は
> `preload()` 行の削除のみで済んだ。
>
> **同日、C-9（`hud_manager.gd` watch 帯）を解消**（`/improve-codebase-architecture` 候補2）。
> ヒットテスト4関数（`module_slot_at`/`inventory_panel_row_at`/`column_at`/
> `inventory_panel_consumes`）を新設 `hud_hit_test.gd`（`HudHitTest`）へ抽出し、
> `hud_manager.gd` は HUD 構築・更新（`build_*`/`update_*`）専任に戻った。
> `fitted_header.clip_text` インシデント（表示専用の変更がヒットテストを黙って壊した実例）が
> 「今は変えない」判断を覆すトリガーになった。`hud_surface.gd` の4呼び出しを `HudHitTest.*`
> へ更新、対応する GdUnit4 テストは `hud_hit_test_test.gd` へ移動（新規テストなし、186/186維持）。

| ファイル | 行数 | 判定 |
|---|---|---|
| `client/scripts/main.gd` | 1217 | 🟢 オーケストレーション層。scene lifecycle / node generation / event dispatch / network send / HUD frame assembly を保持 |
| `client/scripts/hud_manager.gd` | 789 | 🟢 HUD 全パネルの構築・更新の stateless static class。C-9解消（2026-07-10）でヒットテスト4関数を `hud_hit_test.gd` へ分離し、850→789 |
| `client/scripts/hud_hit_test.gd` | 88 | 🟢 **新設**（2026-07-10、C-9解消）。`HudManager` が構築した Control 群への画面座標ヒットテスト専任（`module_slot_at`/`inventory_panel_row_at`/`column_at`/`inventory_panel_consumes`） |
| `client/scripts/connection.gd` | 344 | 🟢 WebSocket I/O とシグナル発行のみ。2026-07-10、`player_fitting_received` を生JSON文字列渡しに変更（Dictionary再エンコードによる整数→浮動小数点化バグの修正）。2026-07-11、ADR-0042でバイナリ送受信に対応、`_flush_buffer`の改行分割ロジックを撤去（374→344） |
| `client/scripts/world_session.gd` | 358 | 🟢 InitialState / AoI / HP / lock / tick-cap / dock state の client-side live world state |
| `client/scripts/ship_controller.gd` | 342 | 🟢 単一船の視覚表現に専念。ロックオン枠は `BillboardRing` 共通化 |
| `client/scripts/navigation_marker_renderer.gd` | 227 | 🟢 ゲート/惑星/ステーションマーカー生成 + スペクトル色 |
| `client/scripts/inventory_row.gd` | 90 | 🟢 HUD インベントリパネル行の typed shape |
| `client/scripts/hud_surface.gd` | 234 | 🟢 HUD Control 参照を所有し、`main.gd` からの render frame / hit-test 要求を `HudManager` へ委譲 |
| `client/scripts/input_decoder.gd` | 158 | 🟢 キー入力→アクション決定の純粋関数。GdUnit4 テスト済み。2026-07-10、`I`キーのshipless soft-lock修正（147→158） |
| `client/scripts/camera_controller.gd` | 142 | 🟢 自己完結したオービットカメラ |
| `client/scripts/world_interaction.gd` | 133 | 🟢 selection state、double-click timing、click→intent、lock intent、`InputDecoder` 連携を所有する deep module |
| `client/scripts/world_presentation.gd` | 311 | 🟢 floating origin / nav marker placement / sky sun update / warp tunnel / player ship presentation を所有する deep module |
| `client/scripts/ship_picking.gd` | 104 | 🟢 船/ゲート/天体ピッキング3関数（画面空間ピッキング） |
| `crates/dawn-client-core/src/world_space.rs` | — | 🟢 浮動原点・軸変換・距離計算をf64で所有（ADR-0029/0044） |
| `crates/dawn-client-gdext/src/world_space_gd.rs` | — | 🟢 WorldSpaceをGodotグローバルクラスとして公開し、最終値だけVector3へ狭める |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画のみ |
| `client/scripts/billboard_ring.gd` | 65 | 🟢 固定画面サイズの選択リング billboard 共通 static class |
| `client/scripts/unit_format.gd` | 38 | 🟢 速度/距離の適応的単位整形（m/s・km/s・AU/s） |
| `client/scripts/warp_tunnel_effect.gd` | 10 | 🟢 ワープトンネル ColorRect の intensity ラッパー |

合計 4,924 行（2026-07-17 実測。前回4,826から、PR #143のtyped HUD refs追加により増加。ADR-0042で`connection.gd`の
改行バッファリング撤去により-30）のうち `main.gd` が25%を占める
（C-1着手前69%から大幅低下、水準維持）。
新設 static class 群（C-1 の5クラス + ADR-0029 の `world_space`/`unit_format`/`warp_tunnel_effect`
+ `WorldSession` + `HudSurface` + `WorldInteraction` + `WorldPresentation` + C-8 の
`InventoryRow` + C-9 の `HudHitTest`）は、`WorldSession` が ship registry と live world state、`HudSurface` が
HUD Control 参照、`WorldInteraction` が selection と world interaction policy、
`WorldPresentation` が world visual side effect、`InventoryRow` が HUD インベントリパネル行の
shape を保持する。scene 生成と network send は `main.gd` 側。PlayerLoadout の wire row schema
（旧 `ModuleRow`/`ItemRow`/`player_loadout.gd`、C-4）は 2026-07-10、`dawn-client-core`
（純粋 Rust）+ `dawn-client-gdext`（GDExtension バインディング）へ移植した（ADR-0039/ADR-0040）。

（`client/test/*.gd` は 16 ファイル・合計 2,856 行。2026-07-11、ADR-0042で
`client_command_gd_test.gd`に`ClientMessageDecoder`/`hello_command`関連のテストを
追加（ケース数186→202、GdUnit4実行で確認済み・0 errors/0 failures/0 orphans）。
詳細な内訳は completed.md のテストカバレッジ表を参照）

---

## main.gd 内部構造（行範囲別、C-1 完了後）

> 注: 以下の行範囲は C-1 完了時点（1094行）のもの。現在の `main.gd` は 1217 行で、範囲には
> ずれがありうるが、区分
> （責務のまとまり）の傾向は有効だが、正確な行番号は次回の構造リファクタ（R-2 着手時）に再計測する。

| 行範囲 | 内容 | 評価 |
|---|---|---|
| 1–40 | ノード参照（`@onready`）・スクリプト preload | 🟢 |
| 42–135 | 定数・内部状態（船/HP/モジュール/ゲート・天体配列/選択状態） | 🟡 ドメインごとに分類はされているが量が多い |
| 137–163 | `_ready()` / `_process()` | 🟡 `_process()` がカプ再現・近接判定・太陽方向・HUD更新の集約呼び出し点になっている |
| 164–188 | ゲート/天体マーカー生成（呼び出しのみ） | 🟢 本体は `navigation_marker_renderer.gd` |
| 189–243 | 太陽方向シェーダー更新・ゲート近接判定 | 🟢 `NavigationMarkerRenderer.spectral_color()` を利用 |
| 244–313 | `_input()`（キー判定は `InputDecoder`、HUD hit-test は `HudSurface` へ委譲・world mouse処理は残置） | 🟡 マウスのダブルクリック判定・ピッキング選択は状態を持つため main.gd に残置（理由は C-1 参照） |
| 314–395 | ダブルクリック判定・船/ゲート/天体ピッキング・選択 | 🟢 ピッキング本体は `ship_picking.gd` |
| 396–454 | ロックオン・移動・停止コマンド送信 | 🟢 |
| 455–788 | サーバーイベント dispatch（jump/system change/AoI/fitting/destroyed等） | 🟢 個々のハンドラは narrow |
| 789–908 | ワープ着地点計算（ゲート/天体） | 🟢 共通コア `_compute_warp_snap_pos_core` に抽出済み（C-2）。GdUnit4 テスト済み |
| 909–970 | HUD更新ディスパッチ（`HudSurface.render()` 呼び出し） | 🟢 値の整形（速度文字列・距離文字列等）のみ main.gd 側、Control参照所有は `HudSurface`、構築・描画は `HudManager` |
| 971–1003 | クライアント側カプ再現シミュレーション | 🟢 サーバーの `CapacitorSystem` を正しく模倣 |
| 1004–1094 | 状態クリア・空間環境構築・プレイヤーマテリアル | 🟢 |

---

## Issue ID 登録簿

コード内コメントが参照する issue ID の一覧。詳細な解消経緯は
[client-completed.md](./client-completed.md) を参照。
現在 open な issue は C-9 の再観測のみ。詳細な判断と再評価トリガーは
[client-pending.md](./client-pending.md) を参照。

| ID | 内容 | 状態 |
|---|---|---|
| C-1 | `main.gd` god object（13以上の異種責務） | 解消済み |
| C-2 | マーカー生成/ピッキング/ワープ着地点計算の同型ロジック2重実装 | 解消済み |
| C-3 | シーンツリー直パス参照の脆さ（`@onready` の `$Connection` 等8箇所、null チェックなし） | 解消済み |
| C-4 | PlayerLoadout dict のスキーマ非検証 | 解消済み |
| C-5 | client World Session module | 解消済み |
| C-6 | client HUD Surface module | 解消済み |
| C-7 | client World Interaction module | 解消済み |
| C-8 | インベントリ行 Dictionary が stringly-typed のまま main.gd と合意している | 解消済み |
| C-9 | `hud_manager.gd` が watch 帯（850行）に到達 | 再観測（2026-07-17、892行）。hit-testの責務逆流ではなくtyped HUD refs追加による増加。 |

**運用上の注意**: `class_name` を新規追加した直後は Godot がプロジェクトを
スキャンするまでグローバル識別子として認識されない（CLI テストが
`Identifier "X" not declared` で失敗する）。`class_name` 抽出のたびに発生する。
```bash
"$GODOT_BIN" --headless --editor --quit-after 3 --path client
```

---

## 触らない箇所（安定・枯れている）

- `connection.gd` — WebSocket I/O とシグナル発行のみ。ドメインロジックなし。教科書的な境界
- `ship_controller.gd` — 単一船の視覚表現に専念。ロックオン枠の生成だけ `BillboardRing`
  に依存するようになったが（2026-06-21）、それ以外は他システムへの結合なし
- `camera_controller.gd` — 自己完結したオービットカメラ。依存はターゲットノード参照のみ
- `billboard_ring.gd` — 2026-06-21新設。固定画面サイズの選択リングを生成する
  stateless static class。GdUnit4 テスト付き。`navigation_marker_renderer.gd`（惑星）と
  `ship_controller.gd`（ロックオン枠）が共有——同種の「選択/状態インジケーター」表現を
  1ヶ所にまとめることで、距離耐性や見た目の一貫性を保証する
- `tactical_overlay.gd` — 射程リング描画のみ。受け取った値を描くだけで状態を持たない
- `ship_picking.gd` / `navigation_marker_renderer.gd` / `input_decoder.gd` /
  `hud_manager.gd` / `hud_hit_test.gd` / `hud_surface.gd` / `world_interaction.gd` — C-1 以降に新設した HUD・入力・描画支援モジュール群。いずれも GdUnit4
  テスト付きで挙動が固定されている。他クラスへの依存はメソッド引数経由のみ
  （Camera3D・候補データ・Callable・refs Dictionary・`BillboardRing`・HUD root Control・正規化 input facts）で、main.tscn のノード構成
  変更の影響を受けにくい
- `main.gd` のイベント dispatch 層（455–788行） — 個々のハンドラは narrow で ADR 参照付き

---

未完項目・保留判断は
[client-pending.md](./client-pending.md)、
解消済みの作業ログ・テストカバレッジの内訳は
[client-completed.md](./client-completed.md) を参照。
