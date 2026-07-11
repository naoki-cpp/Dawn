---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 完了済み作業ログ
audience : AI Agent / Human Developer
update   : /architecture-review が issue を解消済みへ移動するたびに追記
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-07-10
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
| C-5 | client World Session module | `world_session.gd` に live world state を集約。 |
| C-6 | client HUD Surface module | `hud_surface.gd` を新設し、HUD 参照所有と dirty-tracking を集約。 |
| C-7 | client World Interaction module | `world_interaction.gd` に selection / click→intent / key action を集約。 |
| C-4 | PlayerLoadout dict のスキーマ非検証 | `ModuleRow` / `ItemRow` を導入し typed row 化。2026-07-10、GDScript実装をRustへさらに移植（ADR-0039/0040: `dawn-client-core`が型・純粋関数を所有、`dawn-client-gdext`が同名クラスとしてGDScriptへ公開、`cargo test`対象化）。 |
| C-2 | マーカー生成/ピッキング/ワープ着地点計算の同型ロジック2重実装 | 共通 helper と補助 module へ抽出。 |
| C-3 | シーンツリー直パス参照の脆さ（`@onready` の `$Connection` 等8箇所、null チェックなし） | `_assert_scene_tree_refs()` による fail-fast 検証を追加。 |
| C-8 | インベントリ行 Dictionary が stringly-typed のまま main.gd と合意している | `InventoryRow` を導入し typed row 化。 |
| C-9 | `hud_manager.gd` が watch 帯（850行）に到達 | 2026-07-10、`/improve-codebase-architecture` 候補2。ヒットテスト4関数（`module_slot_at`/`inventory_panel_row_at`/`column_at`/`inventory_panel_consumes`）を新設 `hud_hit_test.gd`（`HudHitTest`）へ抽出し、`hud_manager.gd` は HUD構築・更新専任に戻した（850→789）。`fitted_header.clip_text` インシデントが「今は変えない」判断を覆すトリガーになった。テストは `hud_hit_test_test.gd` へ移動（新規追加なし、GdUnit4 186/186 維持）。 |

C-1 の抽出先（`ShipPicking` / `NavigationMarkerRenderer` / `InputDecoder` / `HudManager`）と
追加の deep modules（`WorldSession` / `HudSurface`）はいずれも GdUnit4 テスト付き。
各規模は client.md の「ファイルサイズ一覧」を参照。
マウス入力は scene-tree 依存の end-to-end 配線こそ `main.gd` に残るが、状態依存の本体
（selection ownership / double-click / click→intent）は `WorldInteraction` に移動済み。

---

## テストカバレッジ（C-1 完了時点 + 以降の回帰テスト追加、2026-07-11 実測で更新）

| テストファイル | 対象 | ケース数 |
|---|---|---|
| `main_test.gd` | main.gd 残存純粋関数 + モジュールdeactivate判定の回帰テスト | 31 |
| `ship_picking_test.gd` | `ShipPicking`（画面空間ピッキング含む） | 12 |
| `navigation_marker_renderer_test.gd` | `NavigationMarkerRenderer`（選択リング含む） | 12 |
| `input_decoder_test.gd` | `InputDecoder`。2026-07-07、`KEY_X`（Disembark）判定のケースを追加（+2）。2026-07-10、`I`キーのshipless soft-lockケースを修正（既存ケースの動作を反転、件数は変わらず） | 32 |
| `hud_manager_test.gd` | `HudManager`（2026-07-10、C-9解消でヒットテスト系4ケースを `hud_hit_test_test.gd` へ移動） | 26 |
| `hud_hit_test_test.gd` | `HudHitTest`（2026-07-10新設、C-9解消。`module_slot_at`/`column_at` のヒットテストケース） | 7 |
| `hud_surface_test.gd` | `HudSurface`（HUD render frame / fitting更新 / inventory hit-test 委譲 / パネル dirty-tracking 判定。C-4 で `ModuleRow` の `clone()`/`equals()` ベース差分判定のケースを追加）。2026-07-08、station roster / `source` タグ付けのケースを追加（+2） | 16 |
| `billboard_ring_test.gd` | `BillboardRing` | 3 |
| `camera_controller_test.gd` | `CameraController`（orbit drag） | 2 |
| `unit_format_test.gd` | `UnitFormat`（ADR-0029 速度/距離単位整形） | 8 |
| `world_space_test.gd` | `WorldSpace`（ADR-0029 浮動原点リベース） | 4 |
| `connection_test.gd` | `connection.gd`（URL正規化・module activated signal・PlayerLoadout wire message rename の回帰テスト）。2026-07-10、`player_fitting_received`が生JSON文字列を運ぶ変更に伴い既存2ケースを更新（件数は変わらず） | 6 |
| `world_session_test.gd` | `WorldSession`（InitialState / ship registry / HP / lock / tick-cap / destroy / dock state） | 13 |
| `world_interaction_test.gd` | `WorldInteraction`（selection ownership / double-click / lock intent / key action 解釈） | 8 |
| `world_presentation_test.gd` | `WorldPresentation`（marker clamp / warp tunnel easing / sun state） | 6 |
| `client_command_gd_test.gd` | `ClientCommand`/`ClientMessageDecoder`（ADR-0041、前回計測時に本表への記載漏れ）。2026-07-11、ADR-0042でpostcardバイナリ化に伴い`ClientMessageDecoder`（テスト専用デコーダ）経由のアサーションへ書き換え、`build()`契約テスト3件・`hello_command`テスト2件を追加 | 16 |
| **合計** | | **202**（`func test_` 実測、2026-07-11。`player_loadout_test.gd`（旧18ケース）は ADR-0039/0040 で `dawn-client-core` の Rust ユニットテストへ移植し削除。C-9解消でのファイル分割は移動のみで件数変化なし。`client_command_gd_test.gd` の前回計測漏れ（11件）+ ADR-0042の新規テスト5件を今回追加） |

テスト導入で見つかった不具合・定着した手順（詳細: `docs/process/godot-client-testing.md`）:
- `Node3D` をシーンツリーに追加せず `global_position` を読むと `(0,0,0)` 固定になる
- `class_name` 新規追加直後はキャッシュ未更新で全件失敗する（`client.md` の
  「運用上の注意」コマンドで解消）
- `add_child()` しない Control ノードは `auto_free()` で明示的に解放する（orphan node 検出）

`main.gd` に残るのは input event の配線、イベント dispatch、scene spawning といった
シーンインスタンス化やネットワーク接続が絡む領域で、ここは引き続き視覚的な確認が主な検証手段になる。
