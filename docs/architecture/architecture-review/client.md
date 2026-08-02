---
scope    : GodotクライアントとRust/GDExtension境界の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / architecture issue更新時
related  : docs/architecture/architecture-review/server.md（サーバー側）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）,
           docs/architecture/architecture-review/client-pending.md（未完項目）
date     : 2026-08-02（#251 server-fact policy ownership、#258統合反映）
---

# Architecture Review — Dawn Client（現行構造評価）

詳細な判断とtriggerは[client-pending.md](./client-pending.md)、完了履歴とtest内訳は
[client-completed.md](./client-completed.md)を参照する。

## 現状評価

**総合: A。** `main.gd`は長いがgod objectには戻っていない。state、interaction、presentation、HUD、wire adapterの所有者は分離済み。

issue #238で復号済みwire型をGodot `Dictionary`へ投影してRustへ戻す経路を削除し、
#248で旧client adapterを削除した。#251ではtick、lock、dock、system、loadout、module、
ship lifecycleのwire非依存policyを`dawn-client-core::ClientState`へ移した。
GDExtensionはwire検証、wire→`ClientFact`変換、fact適用、presentation変換だけを行う。
`ServerMessageOutcome::dispatch`が唯一のpresentation seamであり、state commit後に
connection callbackまたは最終world handlerを一度だけ呼ぶ。#258で削除した
Godot公開のserver-state mutation backdoorは、#251との統合後も復活させない。
統合後のGDExtension境界は、固定Rust 1.97.1でformat、focused check、clippyを検証した。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| ファイル分割 | A | `WorldSession` / `WorldInteraction` / `WorldPresentation` / HUD各層の所有者が明確。18スクリプトへ分割済み |
| `main.gd`責務 | A− | scene lifecycle、node generation、event dispatch、network send、HUD assemblyに限定 |
| 型境界 | A | wire decode → `ClientFact` → Rust state commit → typed presentationの単一経路。Dictionary再入力なし |
| 重複 | A− | shadow state、JSON往復、adapter内domain policy、二段event dispatchは解消。残るauthority/API重複は#200・#202 |
| デッドコード | A | `ClientOutcome` mirrorと旧`ServerEventOutcome`互換classを削除 |
| テスト可能性 | A | pure Rust `ClientState` transition test + typed outcome fixtureを使うGdUnit4。scene-tree/実WebSocket E2Eのみ手動領域 |

## State ownership

- `ClientState`: server factをsession/loadoutへ適用するwire非依存policy
- `WorldSessionState`: live world stateと低レベルtyped transition
- `WorldSession`: Godot adapter。公開write面はreset/client prediction tickに限定
- `PlayerLoadout`: fitting/inventory/capacitor state。server replacement/module activationは`ClientState`経由
- `ServerMessageOutcome::dispatch`: GDScriptが明示したconnection/world targetへ、state commit後に一度だけpresentationを渡す境界
- `main.gd`: scene node registryと短命なoptimistic state
- `WorldInteraction`: selection / input facts → intent
- `WorldPresentation`: floating originとvisual effects
- `HudSurface` / `HudManager`: Control参照とpanel構築・更新

navigation map cacheはSector内でwrite-onceに近いpresentation cacheとして許容し、毎frameのRust→Godot再構築は行わない。

## Issue登録簿

| ID | GitHub | 内容 | 状態 |
|---|---:|---|---|
| C-1〜C-8 | — | god object、同型ロジック、scene refs、typed rows、各deep module抽出 | 解消済み |
| C-9 | — | `hud_manager.gd` watch帯 | 再観測・保留 |
| C-10 | #200 | render scale / warp thresholdのauthority重複 | P2 |
| C-11 | #201 | `PlayerLoadout`のDictionary再投影 | 解消済み |
| C-12 | #202 | selection read API二重化 | P3 |
| C-13 | #238 | server outcomeのtyped stateをDictionary経由でRustへ戻す二重変換 | 解消済み |
| C-14 | #251 | server-fact policyがGodot adapterに残る | 解消済み |

`main.gd`の機械的な`.tscn`分割、raw `InputEvent`のdeep module流入、typed recordのDictionary回帰は行わない。
