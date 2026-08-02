---
scope    : GodotクライアントとRust/GDExtension境界の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / architecture issue更新時
related  : docs/architecture/architecture-review/server.md（サーバー側）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-08-02（#248 legacy adapter removal後の再計測）
---

# Architecture Review — Dawn Client（現行構造評価）

詳細な判断とtriggerは[client-pending.md](./client-pending.md)、完了履歴とtest内訳は
[client-completed.md](./client-completed.md)を参照する。

## 現状評価

**総合: A。** `main.gd`は長いがgod objectには戻っていない。state、interaction、presentation、HUD、wire adapterの所有者は分離済み。

直近ではissue #238で、復号済みwire型を一度Godot `Dictionary`へ投影してから
`WorldSession`へ戻す経路を削除し、#248で旧client adapterを削除した。`ServerMessageOutcome::dispatch`が
`WorldSessionUpdate`を直接Rust-owned stateへ適用し、その後にtyped presentation recordを
GDScriptへ渡す。PlayerLoadoutとMarketも同じ順序で処理される。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| ファイル分割 | A | `WorldSession` / `WorldInteraction` / `WorldPresentation` / HUD各層の所有者が明確。18スクリプトへ分割済み |
| `main.gd`責務 | A− | 1054行。scene lifecycle、node generation、event dispatch、network send、HUD assemblyに限定 |
| 型境界 | A | wire decode → Rust state mutation → typed presentationの単一経路。Dictionary再入力なし |
| 重複 | A− | shadow state、JSON往復、server outcomeのDictionary往復は解消。残るauthority/API重複は#200・#202 |
| デッドコード | A | text-frame fallback、旧shim、JSON/Dictionary再構築adapterを削除 |
| テスト可能性 | A | pure Rust transition test + typed outcome fixtureを使うGdUnit4。scene-tree/実WebSocket E2Eのみ手動領域 |

## State ownership

- `WorldSessionState`: live world stateと`WorldSessionUpdate`のtyped transition
- `WorldSession`: Godot adapter。state更新はserver outcome dispatch内でGDScript callbackより先に適用
- `PlayerLoadout`: fitting/inventory/capacitor表示用state
- `main.gd`: scene node registryと短命なoptimistic state
- `WorldInteraction`: selection / input facts → intent
- `WorldPresentation`: floating originとvisual effects
- `HudSurface` / `HudManager`: Control参照とpanel構築・更新

navigation map cacheはSector内でwrite-onceに近いpresentation cacheとして許容し、毎frameのRust→Godot再構築は行わない。

## ファイルサイズ（2026-08-02、`client/scripts`全18ファイル）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `client/scripts/main.gd` | 1146 | 🟡 orchestration。R-2の再評価trigger内 |
| `client/scripts/hud_manager.gd` | 877 | 🟡 HUD type/build/updateの凝集。C-9は再観測・保留 |
| `client/scripts/ship_controller.gd` | 437 | 🟢 ShipMotion適用とNode3D writer |
| `client/scripts/connection.gd` | 388 | 🟢 binary WebSocket I/O |
| `client/scripts/world_presentation.gd` | 337 | 🟢 floating origin / visual presentation |
| `client/scripts/market_surface.gd` | 270 | 🟢 Market surface |
| `client/scripts/hud_surface.gd` | 266 | 🟢 HUD surface |
| `client/scripts/navigation_marker_renderer.gd` | 227 | 🟢 navigation marker rendering |
| `client/scripts/input_decoder.gd` | 164 | 🟢 input facts → intents |
| `client/scripts/camera_controller.gd` | 145 | 🟢 camera orbit input |
| `client/scripts/world_interaction.gd` | 125 | 🟢 selection / click→intent |
| `client/scripts/ship_picking.gd` | 104 | 🟢 screen-space picking |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 tactical overlay |
| `client/scripts/inventory_row.gd` | 87 | 🟢 typed inventory row |
| `client/scripts/hud_hit_test.gd` | 80 | 🟢 HUD hit testing |
| `client/scripts/billboard_ring.gd` | 65 | 🟢 selection ring presentation |
| `client/scripts/unit_format.gd` | 38 | 🟢 unit formatting |
| `client/scripts/warp_tunnel_effect.gd` | 10 | 🟢 warp effect |
| **合計** | **4859** | **18ファイル** |

## Issue登録簿

| ID | GitHub | 内容 | 状態 |
|---|---:|---|---|
| C-1〜C-8 | — | god object、同型ロジック、scene refs、typed rows、各deep module抽出 | 解消済み |
| C-9 | — | `hud_manager.gd` watch帯 | 再観測・保留 |
| C-10 | #200 | render scale / warp thresholdのauthority重複 | P2 |
| C-11 | #201 | `PlayerLoadout`のDictionary再投影 | 解消済み |
| C-12 | #202 | selection read API二重化 | P3 |
| C-13 | #238 | server outcomeのtyped stateをDictionary経由でRustへ戻す二重変換 | 解消済み |

`main.gd`の機械的な`.tscn`分割、raw `InputEvent`のdeep module流入、typed recordのDictionary回帰は行わない。
