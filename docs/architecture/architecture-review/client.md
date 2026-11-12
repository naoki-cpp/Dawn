---
scope    : GodotクライアントとRust/GDExtension境界の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / architecture issue更新時
related  : docs/architecture/architecture-review/server.md（サーバー側）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-11-12（doc-sync: #399でC-22を反映し、現行行数を照合）
---

# Architecture Review — Dawn Client（現行構造評価）

詳細な判断とtriggerは[client-pending.md](./client-pending.md)、完了履歴とtest内訳は
[client-completed.md](./client-completed.md)を参照する。

## 現状評価

**総合: B+。** `main.gd`は1276行、`hud_manager.gd`は859行。state、presentation、HUD、wire adapterの
所有者は分離済み。radial action availabilityもC-22で既存Rust policyへ集約され、Godotはtarget factsの提供と
menu presentationを担う。scene composition以外の変更理由が再び増えないかはR-2で監視する。
C-16では、thin Godot decode/outcome seam、canonical inbound delivery、debug fixtureを
`server_message_gd.rs`、`inbound_delivery.rs`、`server_message_fixture.rs`へ分離し、world messageは
必要なstate commit後に`main.gd`の最終handlerへ一度だけ到達する構造へ整理した。
現在、`world_presentation.gd`は595行で、environment・warp/player effectを調停する。
恒星光源・prominence処理は`StellarSource` / `SolarProminences`へ分離され、700行triggerは未発火（C-20）。
`starfield.gd`（335行）は銀河モデルからの星生成とMultiMesh packingが凝集したdeep moduleである。

issue #238で復号済みwire型をGodot `Dictionary`へ投影してRustへ戻す経路を削除し、
#248で旧client adapterを削除した。#251ではtick、lock、dock、system、loadout、module、
ship lifecycleのwire非依存policyを`dawn-client-core::ClientState`へ移した。
GDExtensionはwire検証、wire→`ClientFact`変換、fact適用、presentation変換、
typed client request/action boundaryだけを行う。`server_message_gd.rs`はdecodeとGodot公開outcome、
`inbound_delivery.rs`はcanonical `ServerMessage` match、変換、commit、最終handler選択を所有する。
入力の意味付け、selection/double-click policy、radial action availabilityは
`dawn-client-core::ClientInteraction`が所有する。Godot側はkey/hit-testの正規化、proximity / distance factsの提供、
radial menuの描画とpointer selection、scene effectを担当する。menu open時に候補を固定し、release時は最新factsで
Rustが選択actionを再検証する。既定actionは設けない。
Godot-facing `ServerMessageOutcome::dispatch`が唯一のpresentation seamであり、内部の
`inbound_delivery::dispatch`へ一度だけ委譲する。そこでstate-bearing messageはcommit後に、
presentation-only messageは直接、connection lifecycle callbackまたは最終world handlerを一度だけ呼ぶ。
world factは中間mirrorを作らず、canonical `ServerFact`
matchから最終callbackへ直接変換する。InitialState、PlayerLoadout、
module activation、MarketSnapshot、MotionCorrectionも同じ直接経路を使う。#258で削除した
Godot公開のserver-state mutation backdoorは、#251との統合後も復活させない。
2026-08-20: HUDのframe projectionを`dawn-client-core::HudReadModel`へ移し、
typed panel snapshot / display text / dirty decisionをRustで一元化した。`HudSurface`は
Control参照、snapshotからのpaint、inventory hit-testの委譲だけを担当し、旧frame Dictionary、
panel Dictionary、`ModuleRow`/`ShipHealth`のclone/equality workaround、GDScriptの
unit formatterを削除した。
review修正後のGDExtension境界、追加したClientState回帰test、明示targetを使うGdUnit fixtureは、
固定Rust 1.97.1でformat、client-core/gdext test、clippyを検証した。
#339では、PlayerLoadoutの`ModuleKind`を`dawn_core::ModuleKind`へ一本化した。
`dawn-client-gdext`はwire値をそのままclient-coreへ渡し、Godotのkind stringは
canonical variantへの明示的な`Option` parseに失敗した場合はrange policyへ到達しない。
これにより、loadout policyとpostcard protocol decodeに同じenumのstrictなvariant集合を使う。
#200も実装済みで、render scaleは`WorldSpace::render_scale()`、warp guidanceは
`dawn_core::MIN_WARP_DISTANCE`を読む`ClientRules`が唯一のauthorityになった。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| ファイル分割 | B+ | 26スクリプト。`main.gd` 1276行、`hud_manager.gd` 859行、`world_presentation.gd` 595行を監視。radial action projectionはC-22、Rust message adapterはC-16で責務分離済み |
| `main.gd`責務 | B+ | scene lifecycle / node registry / scene facts assemblyとradial menu lifecycle。action availability policyはC-22で既存coreへ移行。残るcomposition splitはR-2でDefer |
| 型境界 | A | wire decode → `ClientFact` → Rust state commit → typed presentation、input fact → `ClientAction` → typed request/local effect、radial candidate facts → Rust availability projectionが単一経路 |
| 重複 | A | radial action availabilityとrelease-time validationが同じRust policyを共有。その他shadow state、JSON往復、二段event dispatch、#200/#342のauthority重複は解消済み |
| デッドコード | A− | 旧outcome mirrorは削除済み。C-21のtest専用`WorldSession.system_names()` Dictionary投影も削除済み |
| テスト可能性 | A | pure Rust policy tests + typed fixtureのGdUnit4。C-22のRust/GdUnit focused testsは成功。実画面のpointer操作とWebSocket統合確認、peak frame-time計測は手動領域 |

2026-08-19: Station Inventoryのクリック/ドロップ方針と既存`ClientRequest`構築は
`dawn-client-core::StationInventoryInteraction`へ移し、GDExtensionはtyped row/actionの
薄いadapter、Godotは描画・hit-test・drag geometry・local picker effectだけを担当する。

2026-10-31: 19 GDScriptのうち`connection.gd`は313→254行。ログ抑制後も接続とtyped deliveryの責務は維持。
Rust境界は`world_session.rs` 903→980行、`inbound_delivery.rs` 627→652行へ更新。
2026-11-12: #397で`main.gd`は977→1263行（純増+286行）。Rustの`ClientInteraction::resolve_target_action`
は選択後のtarget validationを担う一方、action候補のavailability/default priorityはGDScriptにあったためC-22をFixとして登録。
radial rendering自体は`radial_menu.gd`へ分離済み。R-2はscene compositionの機械的分割を引き続きDeferする。
C-20は595行で700行trigger未満。C-9/R-2/C-20の既存Defer triggerに変化はない。

2026-11-12: C-22を実装。候補の可否と実行時validationを`ClientInteraction`へ集約し、default actionを廃止。
Godotは候補一覧をmenu open時に固定し、release時に更新した距離・近接factsをRustへ渡して再検証する。

## State ownership

- `ShipMotion`（core）: 操船対象と移動状態選択、表示上限・他船の表示可否、reset/undock、f64座標を所有。Godotはrole通知とscene反映のみ
- `ClientState`: server factをsession/loadoutへ適用するwire非依存policy
- `WorldSessionState`: live world stateと低レベルtyped transition
- `WorldSession`: Godot adapter。公開write面はreset/client prediction tickに限定
- `PlayerLoadout`: fitting/inventory/capacitor state。server replacement/module activationは`ClientState`経由
- `StationInventoryInteraction`（core）: station inventoryの行クリック/ドロップ方針とtyped `ClientRequest`構築
- `StationInventoryRow` / `StationInventoryAction`（GDExtension）: Godot境界のtyped metadata/result adapter
- `ServerMessageOutcome::dispatch`: decoded messageをinternal deliveryへ委譲するthin Godot公開境界
- `inbound_delivery::dispatch`: canonical message match、wire→`ClientFact`変換、state commit、connection lifecycleまたは最終world handlerへの一回のdelivery
- `main.gd`: scene node registry、short-lived optimistic state、radial menu lifecycle。action availability projectionは`ClientInteraction`が所有
- `ClientInteraction`（core）: selection、double-click、input facts → `ClientAction`
- `WorldInteraction`（Godot）: Key/hit-test normalizationとcore adapter
- `WorldPresentation`: floating originとvisual effects
- `HudReadModel`（core）: HUD panel projection、display text、value-based dirty decision
- `HudSurface` / `HudManager`: Control参照とtyped snapshotのpaint・panel構築

navigation map cacheはSector内でwrite-onceに近いpresentation cacheとして許容し、毎frameのRust→Godot再構築は行わない。

## ファイルサイズ（2026-11-12 doc-sync: 掲載表の行数照合）

### GDScript（`client/scripts/`）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `client/scripts/main.gd` | 1276 | 🟡 R-2。scene orchestrationとradial menu lifecycle。action availabilityはRustへ集約済み。compositionを機械分割しない |
| `client/scripts/hud_manager.gd` | 859 | 🟡 C-9。typed refsとpanel build/updateが同一責務。独立変更理由が分かれるまで保留 |
| `client/scripts/ship_controller.gd` | 412 | 🟢 motion adapterとvisual effectの一つのShip presentation boundary |
| `client/scripts/connection.gd` | 291 | 🟢 socket/credential adapter・typed outcome受け渡し。通常送信を一本化、HelloのWelcome前送信は分離（2026-11-08） |
| `client/scripts/world_presentation.gd` | 595 | 🟡 C-20。celestial projectionはRustへ集約、恒星光源とprominenceは別module。environment/warp/player effectを調停し、700行trigger未発火 |
| `client/scripts/market_surface.gd` | 270 | 🟢 Market panel surface |
| `client/scripts/hud_surface.gd` | 157 | 🟢 typed snapshot paint・HUD reference ownership |
| `client/scripts/navigation_marker_renderer.gd` | 226 | 🟢 navigation marker・planet surface・EVE-style bracket rendering |
| `client/scripts/sky_catalog.gd` | 45 | 🟢 fixed bright-star landmark data |
| `client/scripts/starfield.gd` | 335 | 🟢 GalacticModel由来の星分布・spectral flux・MultiMesh packingの一つのrender-data kernel |
| `client/scripts/camera_controller.gd` | 154 | 🟢 camera orbit input |
| `client/scripts/ship_picking.gd` | 135 | 🟢 screen-space picking |
| `client/scripts/world_interaction.gd` | 180 | 🟢 Godot key/hit-test normalizationとClientInteraction adapter |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 tactical overlay |
| `client/scripts/connection_dialog.gd` | 103 | 🟢 connection formの表示と接続状態に応じたdialog lifecycle |
| `client/scripts/galactic_model.gd` | 196 | 🟢 星・塵の銀河分布を決定論的に評価するmodel |
| `client/scripts/lighting_study.gd` | 162 | 🟢 手動capture用の独立したlighting study scene。runtime presentationから分離 |
| `client/scripts/radial_menu.gd` | 196 | 🟢 radial menuの描画、pointer selection、hover label |
| `client/scripts/solar_prominences.gd` | 86 | 🟢 camera-relative prominence geometryとvisibility |
| `client/scripts/stellar_source.gd` | 30 | 🟢 stellar radiance / disc varianceの共有image-plane model |
| `client/scripts/inventory_row.gd` | 22 | 🟢 rendered Panelとtyped Station Inventory policy rowの対 |
| `client/scripts/hud_hit_test.gd` | 79 | 🟢 HUD hit-test geometry |
| `client/scripts/billboard_ring.gd` | 65 | 🟢 selection/lock ring visual |
| `client/scripts/billboard_bracket.gd` | 67 | 🟢 fixed-size navigation bracket visual |
| `client/scripts/warp_tunnel_effect.gd` | 17 | 🟢 warp tunnel visual |
| `client/scripts/object_callouts.gd` | 74 | 🟢 screen-space引き出し線・label配置。入力を遮らずcamera/sceneから描画する |

### Rust/GDExtension boundary（client crates、C-16関連と500行以上）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-client-core/src/world_session.rs` | 980 | 🟢 typed world state machine・lifecycle/reconciliation。単一のsession authority |
| `crates/dawn-client-gdext/src/inbound_delivery.rs` | 652 | 🟢 canonical message match、wire→`ClientFact`変換、state commit、effect抽出、final handler delivery |
| `crates/dawn-client-gdext/src/server_message_gd.rs` | 65 | 🟢 thin Godot decoder / outcome seam。runtime deliveryは`inbound_delivery.rs`へ一度だけ委譲 |
| `crates/dawn-client-gdext/src/server_message_fixture.rs` | 195 | 🟢 debug-only typed GdUnit fixture construction。production deliveryから分離 |
| `crates/dawn-client-core/src/client_state.rs` | 844 | 🟢 ClientFactからWorldSessionEffectへの純粋なstate policy |
| `crates/dawn-client-core/src/client_action.rs` | 920 | 🟢 engine-independent selection/input/radial action policy・typed ClientAction |
| `crates/dawn-client-core/src/hud.rs` | 555 | 🟢 engine-independent HUD read model・formatting・change decisions |
| `crates/dawn-client-core/src/station_inventory.rs` | 758 | 🟢 engine-independent Station Inventory policy・typed request construction |
| `crates/dawn-client-core/src/motion.rs` | 732 | 🟢 client motion/prediction kernel・tests |
| `crates/dawn-client-gdext/src/client_command_gd.rs` | 282 | 🟢 remaining direct typed request builders・入力検証・encode結果のGDExtension boundary |
| `crates/dawn-client-gdext/src/client_action_gd.rs` | 344 | 🟢 ClientAction/ClientInteractionの薄いGodot adapter |
| `crates/dawn-client-gdext/src/world_session_gd.rs` | 179 | 🟢 typed session read adapter。C-21のtest専用Dictionary投影を削除、core stateは維持 |
| `crates/dawn-client-gdext/src/ship_motion_gd.rs` | 221 | 🟢 ShipMotionの型変換adapter。位置authorityはcoreのf64 |
| `crates/dawn-client-core/src/celestial_projection.rs` | 270 | 🟢 statelessな天体/マーカー投影・角サイズ計算とtests |
| `crates/dawn-client-gdext/src/celestial_projection_gd.rs` | 92 | 🟢 投影結果の型変換adapter。描画資源と移動状態を所有しない |
| `crates/dawn-client-core/src/ship_motion.rs` | 592 | 🟢 motion lifecycleとabsolute/floating-origin policyの単一owner |

## Issue登録簿

| ID | GitHub | 内容 | 状態 |
|---|---:|---|---|
| C-1〜C-8 | — | god object、同型ロジック、scene refs、typed rows、各deep module抽出 | 解消済み |
| C-9 | — | `hud_manager.gd` watch帯 | 再観測・保留 |
| C-10 | #200 | render scale / warp thresholdのauthority重複 | 解消済み（2026-07-30、2026-08-25再確認） |
| C-11 | #201 | `PlayerLoadout`のDictionary再投影 | 解消済み |
| C-12 | #202 | selection read API二重化 | 解消済み |
| C-13 | #238 | server outcomeのtyped stateをDictionary経由でRustへ戻す二重変換 | 解消済み |
| C-14 | #251 | server-fact policyがGodot adapterに残る | 解消済み |
| C-15 | #281 | Dictionary/string-tag intent、Market JSON builder、空byteエラーsentinel | 解消済み |
| C-16 | — | inbound relay ladderとdecode / delivery / debug fixtureの責務混在 | 解消済み（2026-08-20） |
| C-17 | — | ClientIntentのpredicate/accessor ladder、GDScript input policy、network send分岐の重複 | 解消済み |
| C-18 | — | Station Inventory interaction policy / string action tag ladder | 解消済み |
| C-19 | — | HUD frame Dictionary / GDScript dirty tracking workaround | 解消済み（2026-08-20、`HudReadModel`） |
| C-20 | — | `WorldPresentation`への独立visual lifecycle集積 | 再観測・trigger付き保留 |
| C-21 | — | test専用`WorldSession.system_names()` Dictionary投影 | 実装済み。core map/typed navigationは維持 |
| C-22 | — | radial action availability/default priorityがGDScript側に残る | 解消済み（2026-11-12、`ClientInteraction`へ集約） |

`main.gd`の機械的な`.tscn`分割、raw `InputEvent`のdeep module流入、typed recordのDictionary回帰は行わない。
