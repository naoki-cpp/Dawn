---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 完了済み作業ログ
audience : AI Agent / Human Developer
update   : /architecture-review が issue を解消済みへ移動するたびに追記
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-08-25
---

# Architecture Review — Dawn Client（完了済みログ）

[client.md](./client.md) の issue のうち、
解消済みのものの詳細な経緯と、GdUnit4 テストカバレッジの内訳を記録する。
**分析のみ。過去分の削除・改変は行わない（監査ログとして追記のみ）。**

---

## 解消済み issue の詳細

### 2026-07-30（2026-08-25再確認） — C-10 / #200 render scale・warp threshold authority

Godot側の`WORLD_SCALE`と`MIN_WARP_DISTANCE`の手動同期を撤去した。
render scaleは`dawn-client-core::WorldSpace::render_scale()`をGDExtension経由で問い合わせ、
`WorldPresentation`とnavigation geometryが同じ値を使う。warp guidanceは
`dawn_core::MIN_WARP_DISTANCE`を読む`dawn-client-core::ClientRules`へ統一した。
authorityを変更するとclient-visible behaviorも追従するRust/GdUnit regression testsがあり、
2026-08-25の再計測ではGdUnit test functionは21 files・220 casesだった。

### 2026-08-24 — #339 canonical ModuleKind boundary

`dawn-client-core`の`ModuleKind` mirrorと`Unknown` fallbackを削除し、
`ModuleRow.kind`は`dawn_core::ModuleKind`を直接保持するようにした。
`dawn-client-gdext`のPlayerLoadout conversionはwire値をそのまま渡し、
Godot-facing kind stringはcanonical variantへの`Option` parseで検証する。
全variantのconversion、exact spelling、invalid string rejection、unknown postcard
discriminant rejectionをRust testsで固定した。既存のloadout range/activation/HUD/capacitor
policyは変更していない。

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
| C-13 | server outcomeのtyped stateをDictionary経由でRustへ戻す二重変換（#238） | `ServerMessageOutcome::dispatch` がdecoded wire valueを`ClientFact`へ変換し、`ClientState`を通じて`WorldSessionState`へ適用してからtyped presentation recordをGDScriptへ通知する単一経路へ移行。navigation/ship lifecycle/AoI/health/lock/motion/dock/system/loadout/marketのDictionary再入力を削除し、pure Rust testとtyped fixture GdUnitを追加。 |
| C-15 | Dictionary/string-tag intentとMarket JSON builder（#281） | `ClientIntent`/`ClientSelection` GDExtension型を追加し、`InputDecoder`/`WorldInteraction`/`main.gd`をsemantic predicateとtyped accessorへ移行。Marketの専用builder化、`MarketOrderSide` enum化、`ClientCommandResult`による明示的エラー、fallible `ClientMessage::encode`を導入し、JSON往復と空byte sentinelを削除。 |
| C-17 | Client Action ladder | `ClientIntent`/`ClientSelection`/`InputDecoder`を削除。selection・double-click・入力ポリシーを`dawn-client-core::ClientInteraction`へ移し、`ClientAction`の`Request`/`Local`へ一度だけ分類。GDScriptはkey/hit-test正規化、scene effect、typed requestの`connection.gd::send_action` transportを担当し、camera-dependentなdouble-click移動だけは`send_move_command()`を使う。 |
| C-18 | Station Inventory interaction ladder | Station Inventoryのクリック、Fit/Unfit、Reorder、Cargo transfer、Assemble、Build、Disassemble、owned-ship selectionの方針と既存`ClientRequest`構築を`dawn-client-core::StationInventoryInteraction`へ移した。`dawn-client-gdext`はtyped row/actionの薄いadapter、Godotは描画・行hit-test・drag geometry・build picker表示を担当する。Unfit Allの非atomicな個別送信、shipless docked Assemble/SelectActiveShip、active+docked必須のBuild/Disassemble、canonical `ItemId` transferを直接Rust testとGdUnit4境界testで固定した。 |

### 2026-08-17 — Client Action ladderの削除

`dawn-client-core::ClientInteraction`が、相互排他的なselection、double-click timing、
keyboard policy、typed `dawn_core::ClientRequest` constructionを所有するようにした。
`ClientAction`はserver requestとGodot-only local effectを単一の型で表し、
`main.gd`の25分岐と入力経路のsend wrapper列を、1つのexecutorと`send_action()`へ集約した。
ただしカメラのscreen ray投影が必要なdouble-click移動だけは、local actionを受けた後に
`send_move_command()`を呼ぶ。Godot側にはengine-specificなkey/hit-test normalizationと
scene/presentation side effectだけを残した。

Rust unit testでinteraction policyを検証し、GdUnit4はGDExtension境界を検証する。
旧`input_decoder_test.gd`は削除し、`world_interaction_test.gd`は10件の薄い境界テストへ整理した。

### 2026-08-19 — Station Inventory interaction policyの抽出

`main.gd`に残っていたStation Inventoryのstring action分岐と、
`connection.gd`の個別送信wrapperを削除した。Rust coreが既存wire requestを
構築し、Godot側には表示・hit-test・drag geometry・local build-picker effectだけを残した。
coreの直接テスト97件と、Station Inventoryを含むGdUnit4 218件で検証した。

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

## テストカバレッジ（C-1 完了時点 + 以降の回帰テスト追加、2026-08-20 実測で更新）

| テストファイル | 対象 | ケース数 |
|---|---|---|
| `main_test.gd` | main.gd 残存純粋関数 + モジュールdeactivate判定の回帰テスト | 41 |
| `ship_picking_test.gd` | `ShipPicking`（画面空間ピッキング含む） | 16 |
| `navigation_marker_renderer_test.gd` | `NavigationMarkerRenderer`（選択リング含む） | 16 |
| `hud_manager_test.gd` | `HudManager`（2026-07-10、C-9解消でヒットテスト系4ケースを `hud_hit_test_test.gd` へ移動） | 26 |
| `hud_hit_test_test.gd` | `HudHitTest`（2026-07-10新設、C-9解消。`module_slot_at`/`column_at` のヒットテストケース） | 7 |
| `hud_surface_test.gd` | `HudReadModel` typed snapshotのpaint、module structure変更、inventory hit-test委譲 | 7 |
| `billboard_ring_test.gd` | `BillboardRing` | 3 |
| `camera_controller_test.gd` | `CameraController`（orbit drag） | 3 |
| `world_space_test.gd` | `WorldSpace`（ADR-0029 浮動原点リベース） | 6 |
| `connection_test.gd` | `connection.gd`（URL正規化・welcome lifecycle・direct final-handler inbound deliveryの回帰テスト） | 16 |
| `market_surface_test.gd` | `MarketSurface` | 1 |
| `planet_visibility_test.gd` | `PlanetVisibility` | 1 |
| `player_loadout_test.gd` | `PlayerLoadout` typed GDExtension boundary | 3 |
| `ship_controller_test.gd` | `ShipController` | 5 |
| `world_session_test.gd` | `WorldSession`（InitialState / ship registry / HP / lock / tick-cap / destroy / dock state） | 6 |
| `world_interaction_test.gd` | `WorldInteraction`/`ClientInteraction`境界（selection / double-click / lock action / key action） | 10 |
| `client_command_gd_test.gd` | `ClientCommandResult`、型付き Sector/Market builder、明示的な入力エラー | 6 |
| `world_presentation_test.gd` | `WorldPresentation`（marker clamp / warp tunnel easing / sun state） | 20 |
| `sky_catalog_test.gd` | bright-star catalogの不変条件 | 2 |
| `station_inventory_test.gd` | typed Station Inventory policy adapter | 5 |
| **合計** | | **200**（`func test_` 実測、2026-08-20） |

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
`toggle_at()`も`ModuleActivationIntent`としてGDExtensionへ公開し、module_id / slot /
activation state / target requirement / effective rangeをtyped accessorで渡す。
Dictionary action payloadとmagic key読み出しは残さない。`player_loadout_test.gd`で
empty stateとtyped activation recordの境界を固定した。


### 2026-08-02 — legacy client adapter removal (#239)

Removed `ClientMessageDecoder`, `json_variant.rs`, JSON row constructors,
`PlayerLoadout.apply_payload`, `PositionComponents`, and duplicate
Dictionary/Vector3 coordinate helpers. GdUnit fixtures now use typed records or
the real binary decoder; postcard command round trips live in `dawn-protocol`
tests. Absolute positions remain f64 components until rendering.

### 2026-08-20 — C-19 HUD Read Model deepening

`dawn-client-core::HudReadModel`がHUD projection、表示整形、value-based change decisionを所有し、
`dawn-client-gdext`はtyped snapshotのGodot adapterになった。`hud_surface.gd`はscene/control参照と
paint forwardingだけを保持し、frame/panel Dictionary、`ModuleRow`/`ShipHealth`のHUD用clone/equality
workaround、`unit_format.gd`を削除した。旧GdUnitの`unit_format_test.gd`はRustのHUD projection/formatting
testsへ移行し、`hud_surface_test.gd`はpaint boundaryを検証する7ケースに更新した。

### 2026-08-20 — C-16 inbound relay ladder collapse

The Godot-facing `ServerMessageOutcome::dispatch` remains compatible and now
delegates once to the internal `inbound_delivery::dispatch`. That module owns
the exhaustive canonical `ServerMessage` match, wire-to-`ClientFact`
conversion, state application, effect extraction, and handler selection. It
sends InitialState, PlayerLoadout, ModuleActivated/Deactivated, MarketSnapshot,
and MotionCorrection directly to the final `main.gd` handlers after any typed
Rust state commit. `connection.gd` retains only connection lifecycle/transport
callbacks: Welcome identity and resume-ticket handling, Redirect, and
request-rejection logging. The no-op welcome relay and all selected
world-message signals were removed.

Debug-only typed fixture construction moved to
`server_message_fixture.rs`; `ServerMessageDecoder.test_outcome()` remains the
GdUnit binary inbound seam. `connection_test.gd` covers each moved family at the
final handler, all typed Market `ItemIdentity` variants, state commit with a
missing final handler, and ShipDocked's accepted effect and station name from
inside the callback. Focused Rust/GDExtension and parse checks passed; full
GdUnit execution remains for the parent verification pass.
