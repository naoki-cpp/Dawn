---
scope    : GodotクライアントとRust/GDExtension境界の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / architecture issue更新時
related  : docs/architecture/architecture-review/server.md（サーバー側）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-07-29（PR #181〜#195と冗長性レビューを反映した部分再計測）
---

# Architecture Review — Dawn Client（現行構造評価）

詳細な判断とtriggerは[client-pending.md](./client-pending.md)、完了履歴とtest内訳は
[client-completed.md](./client-completed.md)を参照する。

## 現状評価

**総合: A−。** `main.gd`は長いがgod objectには戻っていない。state、interaction、presentation、HUD、wire adapterの所有者は分離済み。

直近ではhealth/navigation/register_shipのJSON往復、`main.gd`のWorldSession shadow state、
WebSocket text fallbackを削除し、PR #195でWorldSessionのDictionary returnをtyped recordへ置換した。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| ファイル分割 | A | `WorldSession` / `WorldInteraction` / `WorldPresentation` / HUD各層の所有者が明確 |
| `main.gd`責務 | A− | scene lifecycle、node generation、event dispatch、network send、HUD assemblyに限定 |
| 型境界 | A− | WorldSessionはtyped化済み。PlayerLoadoutの残件を#201で整理 |
| 重複 | A− | shadow stateとJSON往復は解消。残るauthority/API重複は#200〜#202 |
| デッドコード | A | text-frame fallbackと旧shimを削除 |
| テスト可能性 | A− | pure Rust client core + GdUnit4。scene-tree/実WebSocket E2Eのみ手動領域 |

## State ownership

- `WorldSessionState`: live world stateとtyped transition
- `WorldSession`: Godot adapter
- `PlayerLoadout`: fitting/inventory/capacitor表示用state
- `main.gd`: scene node registryと短命なoptimistic state
- `WorldInteraction`: selection / input facts → intent
- `WorldPresentation`: floating originとvisual effects
- `HudSurface` / `HudManager`: Control参照とpanel構築・更新

navigation map cacheはSector内でwrite-onceに近いpresentation cacheとして許容し、毎frameのRust→Godot再構築は行わない。

## ファイルサイズ（部分再計測）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `client/scripts/main.gd` | 1338 | 🟢 orchestration |
| `client/scripts/hud_manager.gd` | 892 | 🟡 C-9 watch |
| `client/scripts/connection.gd` | 395 | 🟢 binary WebSocket I/O |
| `client/scripts/world_interaction.gd` | 133 | 🟢 selection / click→intent |

全`client/scripts`合計は今回再計測していない。2026-07-26の5131行を最後の全面計測値とする。

## Issue登録簿

| ID | GitHub | 内容 | 状態 |
|---|---:|---|---|
| C-1〜C-8 | — | god object、同型ロジック、scene refs、typed rows、各deep module抽出 | 解消済み |
| C-9 | — | `hud_manager.gd` watch帯 | 再観測・保留 |
| C-10 | #200 | render scale / warp thresholdのauthority重複 | P2 |
| C-11 | #201 | `PlayerLoadout`のDictionary再投影 | P2 |
| C-12 | #202 | selection read API二重化 | P3 |

`main.gd`の機械的な`.tscn`分割、raw `InputEvent`のdeep module流入、typed recordのDictionary回帰は行わない。
