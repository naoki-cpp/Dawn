---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 完了済み作業ログ
audience : AI Agent / Human Developer
update   : /architecture-review が issue を解消済みへ移動するたびに追記
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-08-10
---

# Architecture Review — Dawn Client（完了済みログ）

[client.md](./client.md) の issue のうち、
解消済みのものの詳細な経緯と、GdUnit4 テストカバレッジの内訳を記録する。
**分析のみ。過去分の削除・改変は行わない（監査ログとして追記のみ）。**

---

## 解消済み issue の詳細

| ID | 内容 | 解消内容 |
|---|---|---|
| C-1 | `main.gd` god object（13以上の異種責務） | 補助 module を抽出して orchestration 層へ縮小。 |
| C-5 | client World Session module | 当時は `world_session.gd` に live world state を集約。現在は `dawn-client-core::WorldSessionState` と `dawn-client-gdext::WorldSession` adapter が所有（ADR-0046）。 |
| C-6 | client HUD Surface module | `hud_surface.gd` を新設し、HUD 参照所有と dirty-tracking を集約。 |
| C-7 | client World Interaction module | `world_interaction.gd` に selection / click→intent / key action を集約。 |
| C-4 | PlayerLoadout dict のスキーマ非検証 | `ModuleRow` / `ItemRow` を導入し typed row 化。2026-07-10、GDScript実装をRustへさらに移植（ADR-0039/0040: `dawn-client-core`が型・純粋関数を所有、`dawn-client-gdext`が同名クラスとしてGDScriptへ公開、`cargo test`対象化）。 |
| C-2 | マーカー生成/ピッキング/ワープ着地点計算の同型ロジック2重実装 | 共通 helper と補助 module へ抽出。 |
| C-3 | シーンツリー直パス参照の脆さ（`@onready` の `$Connection` 等8箇所、null チェックなし） | `_assert_scene_tree_refs()` による fail-fast 検証を追加。 |
| C-8 | インベントリ行 Dictionary が stringly-typed のまま main.gd と合意している | `InventoryRow` を導入し typed row 化。 |
| C-9 | `hud_manager.gd` が watch 帯（850行）に到達 | 2026-07-10、`/improve-codebase-architecture` 候補2。ヒットテスト4関数（`module_slot_at`/`inventory_panel_row_at`/`column_at`/`inventory_panel_consumes`）を新設 `hud_hit_test.gd`（`HudHitTest`）へ抽出し、`hud_manager.gd` は HUD構築・更新専任に戻した（850→789）。`fitted_header.clip_text` インシデントが「今は変えない」判断を覆すトリガーになった。テストは `hud_hit_test_test.gd` へ移動（新規追加なし、GdUnit4 186/186 維持）。 |
| C-12 | `WorldInteraction` selection read API二重化（#202） | `selection_state() -> Dictionary` を削除し、`main.gd` とテストを `selected_target_id()` / `selected_gate_id()` / `selected_body_id()` のscalar accessorへ統一。選択の相互排他性を維持したままstring-keyed境界を除去。 |
| C-13 | server outcomeのtyped stateをDictionary経由でRustへ戻す二重変換（#238） | `ServerMessageOutcome::dispatch` が `WorldSessionUpdate` を直接 `WorldSessionState` に適用してからtyped presentation recordをGDScriptへ通知する単一経路へ移行。navigation/ship lifecycle/AoI/health/lock/motion/dock/system/loadout/marketのDictionary再入力を削除し、pure Rust testとtyped fixture GdUnitを追加。 |
| C-15 | Dictionary/string-tag intentとMarket JSON builder（#281） | `ClientIntent`/`ClientSelection` GDExtension型を追加し、`InputDecoder`/`WorldInteraction`/`main.gd`をsemantic predicateとtyped accessorへ移行。Marketの専用builder化、`MarketOrderSide` enum化、`ClientCommandResult`による明示的エラー、fallible `ClientMessage::encode`を導入し、JSON往復と空byte sentinelを削除。 |

### 2026-07-24: client WorldSpace の座標計算をRustへ移管

`dawn-client-core::WorldSpace` が絶対座標・浮動原点・軸変換・距離計算を `f64` で所有し、
`dawn-client-gdext::WorldSpace` はGodot型への最終変換だけを担当する構成にした。
`client/scripts/world_space.gd` は削除し、GDScript側にはNode3Dの配置と原点リベース時のシーンツリー更新だけを残した。
Rust単体テスト33件と `world_space_test.gd` のGdUnit4 6件（6/6）で、true-AU近傍の精度・相互変換・リベース連続性を確認した。

### 2026-07-26: client ship-motion / coordinate policy seam を深掘り

`dawn-client-core::ShipMotion` を追加し、`MotionPredictor` と `WorldSpace` の順序制御を
`MotionCommand` → `MotionFrame` に集約した。絶対位置は `PackedFloat64Array` でRustへ渡し、
Godot側はrender frameのNode3D適用だけを担当する。Rustのclient-coreテスト44件と
GDExtension buildを確認し、Godotの手動Prediction / dock / warp確認はPhase 10-5へ残した。

C-1 の抽出先（`ShipPicking` / `NavigationMarkerRenderer` / `InputDecoder` / `HudManager`）と
追加の deep modules（`WorldSession` / `HudSurface`）はいずれも GdUnit4 テスト付き。
各規模は client.md の「ファイルサイズ一覧」を参照。
マウス入力は scene-tree 依存の end-to-end 配線こそ `main.gd` に残るが、状態依存の本体
（selection ownership / double-click / click→intent）は `WorldInteraction` に移動済み。

---

## テストカバレッジ（C-1 完了時点 + 以降の回帰テスト追加、2026-08-02 実測で更新）

| テストファイル | 対象 | ケース数 |
|---|---|---|
| `main_test.gd` | main.gd 残存純粋関数 + モジュールdeactivate判定の回帰テスト | 38 |
| `ship_picking_test.gd` | `ShipPicking`（画面空間ピッキング含む） | 12 |
| `navigation_marker_renderer_test.gd` | `NavigationMarkerRenderer`（選択リング含む） | 12 |
| `input_decoder_test.gd` | `InputDecoder` の型付き `ClientIntent` 生成とキー判定 | 9 |
| `hud_manager_test.gd` | `HudManager`（2026-07-10、C-9解消でヒットテスト系4ケースを `hud_hit_test_test.gd` へ移動） | 26 |
| `hud_hit_test_test.gd` | `HudHitTest`（2026-07-10新設、C-9解消。`module_slot_at`/`column_at` のヒットテストケース） | 7 |
| `hud_surface_test.gd` | `HudSurface`（HUD render frame / fitting更新 / inventory hit-test 委譲 / パネル dirty-tracking 判定。C-4 で `ModuleRow` の `clone()`/`equals()` ベース差分判定のケースを追加）。2026-07-08、station roster / `source` タグ付けのケースを追加（+2） | 17 |
| `billboard_ring_test.gd` | `BillboardRing` | 3 |
| `camera_controller_test.gd` | `CameraController`（orbit drag） | 3 |
| `unit_format_test.gd` | `UnitFormat`（ADR-0029 速度/距離単位整形） | 8 |
| `world_space_test.gd` | `WorldSpace`（ADR-0029 浮動原点リベース） | 6 |
| `connection_test.gd` | `connection.gd`（URL正規化・module activated signal・typed PlayerLoadout message の回帰テスト） | 15 |
| `market_surface_test.gd` | `MarketSurface` | 1 |
| `planet_visibility_test.gd` | `PlanetVisibility` | 1 |
| `player_loadout_test.gd` | `PlayerLoadout` typed GDExtension boundary | 3 |
| `ship_controller_test.gd` | `ShipController` | 4 |
| `world_session_test.gd` | `WorldSession`（InitialState / ship registry / HP / lock / tick-cap / destroy / dock state） | 13 |
| `world_interaction_test.gd` | `WorldInteraction`（typed selection / double-click / lock intent / key intent 解釈） | 9 |
| `client_command_gd_test.gd` | `ClientCommandResult`、型付き Sector/Market builder、明示的な入力エラー | 5 |
| `world_presentation_test.gd` | `WorldPresentation`（marker clamp / warp tunnel easing / sun state） | 9 |
| **合計** | | **196**（`func test_` 実測、2026-08-10） |

テスト導入で見つかった不具合・定着した手順（詳細: `docs/process/godot-client-testing.md`）:
- `Node3D` をシーンツリーに追加せず `global_position` を読むと `(0,0,0)` 固定になる
- `class_name` 新規追加直後はキャッシュ未更新で全件失敗する（`client.md` の
  「運用上の注意」コマンドで解消）
- `add_child()` しない Control ノードは `auto_free()` で明示的に解放する（orphan node 検出）

`main.gd` に残るのは input event の配線、イベント dispatch、scene spawning といった
シーンインスタンス化やネットワーク接続が絡む領域で、ここは引き続き視覚的な確認が主な検証手段になる。

### 2026-07-30: C-11 / #201 PlayerLoadout read境界をtyped化

`hud_snapshot()`のpack→即unpackを削除し、modules / inventory / station inventoryは
既存のtyped accessorを直接利用するようにした。owned ship rosterは`OwnedShipRow`、
dock contextとweapon rangeはnarrow scalar accessorとしてGDExtension境界を越える。
`toggle_at()`だけは即座にcommandへ変換される小さな閉じたintent境界としてDictionaryを維持した。
新設`player_loadout_test.gd`の3件と、既存HUD fixtureのtyped row移行でsentinel互換と境界shapeを固定した。


### 2026-08-02 — legacy client adapter removal (#239)

Removed `ClientMessageDecoder`, `json_variant.rs`, JSON row constructors,
`PlayerLoadout.apply_payload`, `PositionComponents`, and duplicate
Dictionary/Vector3 coordinate helpers. GdUnit fixtures now use typed records or
the real binary decoder; postcard command round trips live in `dawn-wire`
tests. Absolute positions remain f64 components until rendering.
