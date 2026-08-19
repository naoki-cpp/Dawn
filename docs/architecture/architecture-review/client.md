---
scope    : GodotクライアントとRust/GDExtension境界の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / architecture issue更新時
related  : docs/architecture/architecture-review/server.md（サーバー側）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-08-17（Client Action ladder削除後に再計測）
---

# Architecture Review — Dawn Client（現行構造評価）

詳細な判断とtriggerは[client-pending.md](./client-pending.md)、完了履歴とtest内訳は
[client-completed.md](./client-completed.md)を参照する。

## 現状評価

**総合: B+。** `main.gd`は1143行、`hud_manager.gd`は877行まで増えているが、state、
interaction、presentation、HUD、wire adapterの所有者は分離済みで、直ちにgod objectへ戻ったとは
判断しない。一方、Rust/GDExtensionの`server_message_gd.rs`はdecode、ClientFact変換、state apply、
Godot callback dispatchが一つのadapterに同居しているため、C-16を部分完了のFix候補として記録する。

issue #238で復号済みwire型をGodot `Dictionary`へ投影してRustへ戻す経路を削除し、
#248で旧client adapterを削除した。#251ではtick、lock、dock、system、loadout、module、
ship lifecycleのwire非依存policyを`dawn-client-core::ClientState`へ移した。
GDExtensionはwire検証、wire→`ClientFact`変換、fact適用、presentation変換、
typed client request/action boundaryだけを行う。入力の意味付けとselection/double-click
policyは`dawn-client-core::ClientInteraction`が所有し、Godot側はkey/hit-testの正規化と
scene effectだけを担当する。
`ServerMessageOutcome::dispatch`が唯一のpresentation seamであり、state commit後に
connection callbackまたは最終world handlerを一度だけ呼ぶ。world factは中間mirrorを作らず、
canonical `ServerFact` matchから最終callbackへ直接変換する。#258で削除した
Godot公開のserver-state mutation backdoorは、#251との統合後も復活させない。
review修正後のGDExtension境界、追加したClientState回帰test、明示targetを使うGdUnit fixtureは、
固定Rust 1.97.1でformat、client-core/gdext test、clippyを検証した。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| ファイル分割 | B+ | 19スクリプトへの分割は維持。`main.gd` 1143行と`hud_manager.gd` 877行は監視帯、Rust message adapterはC-16で分割候補 |
| `main.gd`責務 | A− | scene lifecycle、node generation、event dispatch、network send、HUD assemblyに限定 |
| 型境界 | A | wire decode → `ClientFact` → Rust state commit → typed presentationと、input fact → `ClientAction` → typed request/local effectの単一経路。Dictionary再入力なし |
| 重複 | A− | shadow state、JSON往復、adapter内domain policy、二段event dispatchは解消。残るauthority重複は#200 |
| デッドコード | A | `ClientOutcome` mirrorと旧`ServerEventOutcome`互換classを削除 |
| テスト可能性 | A | pure Rust `ClientState` transition test + typed outcome fixtureを使うGdUnit4。scene-tree/実WebSocket E2Eのみ手動領域 |

2026-08-19: Station Inventoryのクリック/ドロップ方針と既存`ClientRequest`構築は
`dawn-client-core::StationInventoryInteraction`へ移し、GDExtensionはtyped row/actionの
薄いadapter、Godotは描画・hit-test・drag geometry・local picker effectだけを担当する。

## State ownership

- `ClientState`: server factをsession/loadoutへ適用するwire非依存policy
- `WorldSessionState`: live world stateと低レベルtyped transition
- `WorldSession`: Godot adapter。公開write面はreset/client prediction tickに限定
- `PlayerLoadout`: fitting/inventory/capacitor state。server replacement/module activationは`ClientState`経由
- `StationInventoryInteraction`（core）: station inventoryの行クリック/ドロップ方針とtyped `ClientRequest`構築
- `StationInventoryRow` / `StationInventoryAction`（GDExtension）: Godot境界のtyped metadata/result adapter
- `ServerMessageOutcome::dispatch`: GDScriptが明示したconnection/world targetへ、state commit後に一度だけpresentationを渡す境界
- `main.gd`: scene node registryと短命なoptimistic state
- `ClientInteraction`（core）: selection、double-click、input facts → `ClientAction`
- `WorldInteraction`（Godot）: Key/hit-test normalizationとcore adapter
- `WorldPresentation`: floating originとvisual effects
- `HudSurface` / `HudManager`: Control参照とpanel構築・更新

navigation map cacheはSector内でwrite-onceに近いpresentation cacheとして許容し、毎frameのRust→Godot再構築は行わない。

## ファイルサイズ（2026-08-17再計測）

### GDScript（`client/scripts/`）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `client/scripts/main.gd` | 1143 | 🟡 R-2。scene lifecycle / node registry / event wiring / HUD assemblyのorchestration。機械的分割はしない |
| `client/scripts/hud_manager.gd` | 877 | 🟡 C-9。typed refsとpanel build/updateが同一責務。独立変更理由が分かれるまで保留 |
| `client/scripts/ship_controller.gd` | 448 | 🟢 motion adapterとvisual effectの一つのShip presentation boundary |
| `client/scripts/connection.gd` | 403 | 🟢 WebSocket接続・reconnect・typed outcome受け渡し・ClientAction transport seam |
| `client/scripts/world_presentation.gd` | 490 | 🟢 marker・floating-origin・celestial lighting presentation |
| `client/scripts/market_surface.gd` | 270 | 🟢 Market panel surface |
| `client/scripts/hud_surface.gd` | 266 | 🟢 HUD reference ownership・dirty tracking |
| `client/scripts/navigation_marker_renderer.gd` | 286 | 🟢 navigation marker・planet surface・EVE-style bracket rendering |
| `client/scripts/sky_catalog.gd` | 57 | 🟢 fixed bright-star landmark data |
| `client/scripts/camera_controller.gd` | 145 | 🟢 camera orbit input |
| `client/scripts/ship_picking.gd` | 117 | 🟢 screen-space picking |
| `client/scripts/world_interaction.gd` | 125 | 🟢 Godot key/hit-test normalizationとClientInteraction adapter |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 tactical overlay |
| `client/scripts/inventory_row.gd` | 87 | 🟢 typed inventory row presentation |
| `client/scripts/hud_hit_test.gd` | 80 | 🟢 HUD hit-test geometry |
| `client/scripts/billboard_ring.gd` | 65 | 🟢 selection/lock ring visual |
| `client/scripts/billboard_bracket.gd` | 67 | 🟢 fixed-size navigation bracket visual |
| `client/scripts/unit_format.gd` | 38 | 🟢 unit formatting |
| `client/scripts/warp_tunnel_effect.gd` | 10 | 🟢 warp tunnel visual |

### Rust/GDExtension boundary（client crates、500行以上）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-client-core/src/world_session.rs` | 903 | 🟢 typed world state machine・lifecycle/reconciliation。単一のsession authority |
| `crates/dawn-client-gdext/src/server_message_gd.rs` | 836 | 🟡 C-16部分完了。中間mirrorは削除済み、decode / wire→ClientFact / state apply / Godot dispatchのmodule分割は未完了 |
| `crates/dawn-client-core/src/client_state.rs` | 842 | 🟢 ClientFactからWorldSessionEffectへの純粋なstate policy |
| `crates/dawn-client-core/src/client_action.rs` | 584 | 🟢 engine-independent selection/input policy・typed ClientAction |
| `crates/dawn-client-core/src/station_inventory.rs` | 729 | 🟢 engine-independent Station Inventory policy・typed request construction |
| `crates/dawn-client-core/src/motion.rs` | 680 | 🟢 client motion/prediction kernel・tests |
| `crates/dawn-client-gdext/src/client_command_gd.rs` | 564 | 🟢 typed request builder・入力検証・encode結果のGDExtension boundary |
| `crates/dawn-client-gdext/src/client_action_gd.rs` | 280 | 🟢 ClientAction/ClientInteractionの薄いGodot adapter |

## Issue登録簿

| ID | GitHub | 内容 | 状態 |
|---|---:|---|---|
| C-1〜C-8 | — | god object、同型ロジック、scene refs、typed rows、各deep module抽出 | 解消済み |
| C-9 | — | `hud_manager.gd` watch帯 | 再観測・保留 |
| C-10 | #200 | render scale / warp thresholdのauthority重複 | P2 |
| C-11 | #201 | `PlayerLoadout`のDictionary再投影 | 解消済み |
| C-12 | #202 | selection read API二重化 | 解消済み |
| C-13 | #238 | server outcomeのtyped stateをDictionary経由でRustへ戻す二重変換 | 解消済み |
| C-14 | #251 | server-fact policyがGodot adapterに残る | 解消済み |
| C-15 | #281 | Dictionary/string-tag intent、Market JSON builder、空byteエラーsentinel | 解消済み |
| C-16 | — | `server_message_gd.rs`のdecode / fact apply / Godot dispatch混在。中間mirrorは削除済み、module分割は未完了 | 部分完了 |
| C-17 | — | ClientIntentのpredicate/accessor ladder、GDScript input policy、network send分岐の重複 | 解消済み |
| C-18 | — | Station Inventory interaction policy / string action tag ladder | 解消済み |

`main.gd`の機械的な`.tscn`分割、raw `InputEvent`のdeep module流入、typed recordのDictionary回帰は行わない。
