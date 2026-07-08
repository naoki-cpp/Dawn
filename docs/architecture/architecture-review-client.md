---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / 新スクリプト追加時
related  : docs/architecture/architecture-review-server.md（サーバー側）, docs/architecture/architecture.md, docs/process/playtest-guide.md
date     : 2026-07-09（定期再計測。client/scripts の行数 drift を再計測して更新。構造評価と issue 状態に変化はなく、client 総合 A を維持）
---

# Architecture Review — Dawn Client (Godot)

サーバー側 [architecture-review-server.md](./architecture-review-server.md) のクライアント版。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: A**（2026-07-09 再計測で維持。前回レビュー以降、ファイルサイズ表の
記録値は再び drift していたが、責務分担・結合度・保留課題の状態に新しい悪化はない。
`main.gd` は 1089 行、補助モジュール群も引き続き 100〜300 行帯が中心で収まっている。
GdUnit4 ケース数は実測 175）

| 観点 | 評価 | 理由 |
|---|---|---|
| ファイル分割 | A | `main.gd` から `HudManager`/`HudSurface`/`NavigationMarkerRenderer`/`ShipPicking`/`InputDecoder`/`WorldSession`/`WorldInteraction`/`WorldPresentation` を抽出。live world state は `WorldSession`、live HUD Control 参照は `HudSurface`、world interaction policy は `WorldInteraction`、world visual side effect は `WorldPresentation` が所有 |
| `main.gd` の責務集約 | A | god object は実質解消。selection state・ダブルクリック・world selection 優先順位・dock/undock を含む action gating は `WorldInteraction` へ、floating origin / nav marker placement / sky sun update / warp tunnel / player ship presentation は `WorldPresentation` へ移動済み。`main.gd` に残るのは scene lifecycle / scene node generation / event dispatch / network send / HUD frame assembly。2026-07-08、複数所有船ロスター（SHIPS列）・右クリックでのTransferToStation送信ハンドラを追加（974→1056）が、既存の orchestration 層に自然に収まった |
| 重複 | A− | マーカー生成・ピッキング・ワープ着地点計算の同型ロジックは解消済み（C-2） |
| 結合度 | A | signal 経由の `connection.gd` ↔ `main.gd` 結合は良好。`@onready` のシーンツリー直パス参照はフェイルファストガードで解消（C-3）。modules/inventory dict のキー前提の脆さ（C-4）は `ModuleRow`/`ItemRow` typed row 化で解消済み。2026-07-08、`hud_manager.gd` のインベントリ行 Dictionary も `InventoryRow` typed class 化で解消（C-8） |
| デッドコード | A | 残骸なし。コメントは ADR 参照付きで現状と一致 |
| テストカバレッジ | A− | 新設クラス + main.gd残存ロジックの一部を GdUnit4 で計175ケース実行確認済み（2026-07-08 実測）。`WorldSession` / `HudSurface` / `WorldInteraction` / `WorldPresentation` は引き続き scene tree なしで単体テスト可能。scene-tree/ネットワーク依存の end-to-end 入力経路だけが手動確認領域として残る |
| サーバー側との対比 | — | サーバー側はクレート分割（A−）、クライアントはファイル分割（A）。テストカバレッジは依然サーバー側（カバレッジ80%要件）が厚い |

サーバー側が長期にわたる分割リファクタ（Phase 2〜9）を経て A− に達したのに対し、
クライアントは 2026-06-20 の C-1 で初めて本格的な責務分離に着手し、4クラスの抽出と
GdUnit4 テスト基盤の整備（`scripts/setup-godot.*` による pin 済み Godot CLI）を完了した。

---

## ファイルサイズ一覧（2026-07-09 時点）

> **2026-07-09、全ファイル再計測（`/architecture-review`）。** 現在の実測は
> `main.gd` 1089・`hud_manager.gd` 729・`connection.gd` 318・`world_session.gd` 298・
> `ship_controller.gd` 292・`navigation_marker_renderer.gd` 200・`player_loadout.gd` 191・
> `module_row.gd` 106・`item_row.gd` 44・`inventory_row.gd` 83・`hud_surface.gd` 194・
> `input_decoder.gd` 128・`camera_controller.gd` 113・`world_interaction.gd` 101・
> `world_presentation.gd` 262。前回記録より main/HUD 周辺の行数は増えているが、補助モジュールへ
> 責務が薄く保たれたまま整理が進んでいる。

| ファイル | 行数 | 判定 |
|---|---|---|
| `client/scripts/main.gd` | 1089 | 🟢 オーケストレーション層。scene lifecycle / node generation / event dispatch / network send / HUD frame assembly を保持 |
| `client/scripts/hud_manager.gd` | 729 | 🟢 HUD 全パネルの構築・更新の stateless static class。責務は単一（HUD 構築） |
| `client/scripts/connection.gd` | 318 | 🟢 WebSocket I/O とシグナル発行のみ |
| `client/scripts/world_session.gd` | 298 | 🟢 InitialState / AoI / HP / lock / tick-cap / dock state の client-side live world state |
| `client/scripts/ship_controller.gd` | 292 | 🟢 単一船の視覚表現に専念。ロックオン枠は `BillboardRing` 共通化 |
| `client/scripts/navigation_marker_renderer.gd` | 200 | 🟢 ゲート/惑星/ステーションマーカー生成 + スペクトル色 |
| `client/scripts/player_loadout.gd` | 191 | 🟢 loadout/インベントリ正規化と capacitor 再現の純粋関数。main.gd は呼び出すのみで内部構造に触れない |
| `client/scripts/module_row.gd` | 106 | 🟢 typed module row |
| `client/scripts/item_row.gd` | 44 | 🟢 typed inventory row schema |
| `client/scripts/inventory_row.gd` | 83 | 🟢 HUD インベントリパネル行の typed shape |
| `client/scripts/hud_surface.gd` | 194 | 🟢 HUD Control 参照を所有し、`main.gd` からの render frame / hit-test 要求を `HudManager` へ委譲 |
| `client/scripts/input_decoder.gd` | 128 | 🟢 キー入力→アクション決定の純粋関数。GdUnit4 テスト済み |
| `client/scripts/camera_controller.gd` | 113 | 🟢 自己完結したオービットカメラ |
| `client/scripts/world_interaction.gd` | 101 | 🟢 selection state、double-click timing、click→intent、lock intent、`InputDecoder` 連携を所有する deep module |
| `client/scripts/world_presentation.gd` | 262 | 🟢 floating origin / nav marker placement / sky sun update / warp tunnel / player ship presentation を所有する deep module |
| `client/scripts/ship_picking.gd` | 93 | 🟢 船/ゲート/天体ピッキング3関数（画面空間ピッキング） |
| `client/scripts/world_space.gd` | 74 | 🟢 浮動原点（真 AU 距離レンダリング用の WorldSpace リベース） |
| `client/scripts/tactical_overlay.gd` | 67 | 🟢 射程リング描画のみ |
| `client/scripts/billboard_ring.gd` | 59 | 🟢 固定画面サイズの選択リング billboard 共通 static class |
| `client/scripts/unit_format.gd` | 34 | 🟢 速度/距離の適応的単位整形（m/s・km/s・AU/s） |
| `client/scripts/warp_tunnel_effect.gd` | 8 | 🟢 ワープトンネル ColorRect の intensity ラッパー |

合計 4,483 行（2026-07-09 実測）のうち `main.gd` が24%を占める
（C-1着手前69%から大幅低下、水準維持）。
新設 static class 群（C-1 の5クラス + ADR-0029 の `world_space`/`unit_format`/`warp_tunnel_effect`
+ PR #33 の `player_loadout` + `WorldSession` + `HudSurface` + `WorldInteraction` +
`WorldPresentation` + C-4 の `ModuleRow`/`ItemRow` + C-8 の `InventoryRow`）は、
`WorldSession` が ship registry と live world state、`HudSurface` が HUD Control 参照、
`WorldInteraction` が selection と world interaction policy、`WorldPresentation` が
world visual side effect、`ModuleRow`/`ItemRow` が PlayerLoadout の wire row schema、
`InventoryRow` が HUD インベントリパネル行の shape を保持する。scene 生成と
network send は `main.gd` 側。

（`client/test/*.gd` は `world_presentation_test.gd` を含め 15 ファイル・合計 1,808 行。
ケース数は175。§「テストカバレッジ」参照）

---

## main.gd 内部構造（行範囲別、C-1 完了後）

> 注: 以下の行範囲は C-1 完了時点（1094行）のもの。その後 ADR-0029（ワープ演出・単位整形・
> 原点リベース）・ADR-0031（O/K/[/] キー）・ADR-0032（インベントリ行クリック）・PR #33
> （fitting 抽出後の呼び出し変更）が加わり現在 1241行で、範囲は下方にずれている。区分
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

## 問題一覧

### 解消済み（要約）

| ID | 内容 | 解消内容 |
|---|---|---|
| C-1 | `main.gd` god object（13以上の異種責務） | 補助 module を抽出して orchestration 層へ縮小。 |
| C-5 | client World Session module | `world_session.gd` に live world state を集約。 |
| C-6 | client HUD Surface module | `hud_surface.gd` を新設し、HUD 参照所有と dirty-tracking を集約。 |
| C-7 | client World Interaction module | `world_interaction.gd` に selection / click→intent / key action を集約。 |
| C-4 | PlayerLoadout dict のスキーマ非検証 | `ModuleRow` / `ItemRow` を導入し typed row 化。 |
| C-2 | マーカー生成/ピッキング/ワープ着地点計算の同型ロジック2重実装 | 共通 helper と補助 module へ抽出。 |
| C-3 | シーンツリー直パス参照の脆さ（`@onready` の `$Connection` 等8箇所、null チェックなし） | `_assert_scene_tree_refs()` による fail-fast 検証を追加。 |
| C-8 | インベントリ行 Dictionary が stringly-typed のまま main.gd と合意している | `InventoryRow` を導入し typed row 化。 |

C-1 の抽出先（`ShipPicking` / `NavigationMarkerRenderer` / `InputDecoder` / `HudManager`）と
追加の deep modules（`WorldSession` / `HudSurface`）はいずれも GdUnit4 テスト付き。
各規模は「ファイルサイズ一覧」を参照。
マウス入力は scene-tree 依存の end-to-end 配線こそ `main.gd` に残るが、状態依存の本体
（selection ownership / double-click / click→intent）は `WorldInteraction` に移動済み。

**運用上の注意**: `class_name` を新規追加した直後は Godot がプロジェクトを
スキャンするまでグローバル識別子として認識されない（CLI テストが
`Identifier "X" not declared` で失敗する）。`class_name` 抽出のたびに発生する。
```bash
"$GODOT_BIN" --headless --editor --quit-after 3 --path client
```

### テストカバレッジ（C-1 完了時点 + 以降の回帰テスト追加）

| テストファイル | 対象 | ケース数 |
|---|---|---|
| `main_test.gd` | main.gd 残存純粋関数 + モジュールdeactivate判定の回帰テスト | 13 |
| `ship_picking_test.gd` | `ShipPicking`（画面空間ピッキング含む） | 12 |
| `navigation_marker_renderer_test.gd` | `NavigationMarkerRenderer`（選択リング含む） | 12 |
| `input_decoder_test.gd` | `InputDecoder`。2026-07-07、`KEY_X`（Disembark）判定のケースを追加（+2） | 32 |
| `hud_manager_test.gd` | `HudManager` | 21 |
| `hud_surface_test.gd` | `HudSurface`（HUD render frame / fitting更新 / inventory hit-test 委譲 / パネル dirty-tracking 判定。C-4 で `ModuleRow` の `clone()`/`equals()` ベース差分判定のケースを追加）。2026-07-08、station roster / `source` タグ付けのケースを追加（+2） | 16 |
| `billboard_ring_test.gd` | `BillboardRing` | 3 |
| `camera_controller_test.gd` | `CameraController`（orbit drag） | 2 |
| `unit_format_test.gd` | `UnitFormat`（ADR-0029 速度/距離単位整形） | 8 |
| `world_space_test.gd` | `WorldSpace`（ADR-0029 浮動原点リベース） | 4 |
| `connection_test.gd` | `connection.gd`（URL正規化・module activated signal・PlayerLoadout wire message rename の回帰テスト） | 6 |
| `player_loadout_test.gd` | `PlayerLoadout`（PR #33 起点、後に rename。C-4 で `ModuleRow`/`ItemRow` の `from_json()` 検証ケースを追加）。2026-07-07〜08、`active_ship_id`/`owned_ships` のケースを追加（+3） | 17 |
| `world_session_test.gd` | `WorldSession`（InitialState / ship registry / HP / lock / tick-cap / destroy / dock state） | 11 |
| `world_interaction_test.gd` | `WorldInteraction`（selection ownership / double-click / lock intent / key action 解釈） | 8 |
| `world_presentation_test.gd` | `WorldPresentation`（marker clamp / warp tunnel easing / sun state） | 6 |
| **合計** | | **175**（`func test_` 実測、2026-07-08） |

テスト導入で見つかった不具合・定着した手順（詳細: `docs/process/godot-client-testing.md`）:
- `Node3D` をシーンツリーに追加せず `global_position` を読むと `(0,0,0)` 固定になる
- `class_name` 新規追加直後はキャッシュ未更新で全件失敗する（上記コマンドで解消）
- `add_child()` しない Control ノードは `auto_free()` で明示的に解放する（orphan node 検出）

`main.gd` に残るのは input event の配線、イベント dispatch、scene spawning といった
シーンインスタンス化やネットワーク接続が絡む領域で、ここは引き続き視覚的な確認が主な検証手段になる。

（Medium debt: 保留項目なし。C-1〜C-8 は「解消済み」参照）

---

## 改善ロードマップ

C-1〜C-8 はすべて解消済み（上記「問題一覧」参照）。

`main.gd` の god object 問題は実質解消し、C-4（PlayerLoadout dict のスキーマ非検証）も
C-8（インベントリ行 Dictionary の stringly-typed 設計）も typed row 化で解消したため、
クライアント側の次の課題は構造リファクタではなく機能側（戦闘の深み、ADR-0016 §5）が妥当。

### 採らない方針

- main.gd を複数の `.tscn` 化されたコンポーネント（個別シーン+スクリプト）に分割することは、
  シーン参照切れのリスクが高い。pin 済み Godot CLI で構文・実行エラーは検出できるようになったが、
  シーンツリー構成の妥当性（ノードパスの解決・レイアウト）はヘッドレス実行だけでは確認しきれない。
  GDScript ファイル内の `class_name` 抽出（同一シーンに留める）の方が安全で、C-1 では
  実際にこの方式で4クラスを抽出し、GdUnit4 で検証できた。
- raw `InputEvent` をそのまま deep module に飲ませることは当面行わない。`WorldInteraction` は
  正規化された input facts を受けて intent を返す形に留め、Godot の scene-tree / `InputEvent`
  依存を抱え込まない。これにより GdUnit4 での scene-tree なしテスト可能性を保っている。

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
  `hud_manager.gd` / `hud_surface.gd` / `world_interaction.gd` — C-1 以降に新設した HUD・入力・描画支援モジュール群。いずれも GdUnit4
  テスト付きで挙動が固定されている。他クラスへの依存はメソッド引数経由のみ
  （Camera3D・候補データ・Callable・refs Dictionary・`BillboardRing`・HUD root Control・正規化 input facts）で、main.tscn のノード構成
  変更の影響を受けにくい
- `main.gd` のイベント dispatch 層（455–788行） — 個々のハンドラは narrow で ADR 参照付き
