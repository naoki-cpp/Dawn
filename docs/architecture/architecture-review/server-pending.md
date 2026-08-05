---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-08-05
---

# Architecture Review — Dawn Codebase（未完項目）

実装詳細と完了条件は各GitHub Issueに置き、この文書は判断と再評価triggerだけを保持する。

## Medium

### M-3（保留）: `SectorSimulatorActor`と`SimulationNode`の密結合

本番パス外のin-process test/bench adapterで、handlerもmessage → node method → replyの薄い変換である。
**再評価:** production runtimeとのdriftが不具合化するか、in-process clusterを本番構成へ近づける場合。

### M-9（保留）: `EventStore::append`のinfallible contract

`FileEventStore`はwrite/flush失敗時にpanicするが、1 Sector = 1 processのcrash-only recoveryと整合する。
**再評価:** disk-full crashが運用問題になるか、1 processが複数Sectorを所有する場合。

## リファクタロードマップ

### R-2（保留）: client `main.gd`追加分割

live state、interaction、presentationは分離済み。残るscene lifecycle / node generation / network send / HUD assemblyは凝集している。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

### R-3（部分発火・継続監視）: `node/`系ファイルの再肥大

2026-08-05、`node/commands.rs`ではflight / module / station / loadout-refreshという
独立した変更理由が一つの入口に混在していたためtriggerが発火した。issue #264で、外側の
網羅的なfamily選択とfollow-up射影だけを`commands.rs`に残し、各policyを
`command_flight.rs` / `command_module.rs` / `command_loadout.rs` / `command_station.rs`へ
移した。wire shape、domain result、event semanticsは変更していない（ADR-0047 amendment）。

同日、TransitのRequest/Commit/Ack、retry/recovery、idempotency、cleanup判定は
`transit/handoff.rs`へ集約し、`transit/pipeline.rs`はEventStore factの再構築とRaft effect
変換だけを担当する。`node/transit.rs`はECS materializationとsnapshot mapping、その回帰
テストを保持するが、consensus policyとの変更理由は分離された。

`node/warp.rs`は引き続き監視対象とする。
**再評価:** テストを除く実装部分が約700行を超え、かつ独立した変更理由が混在する、または
module間のdriftが実害になる場合。行数だけでは発火させない。

## 一覧

| 項目 | 状態 |
|---|---|
| R-2 | 保留・trigger付き |
| R-3 | commands / transit slice 完了、warp継続監視 |
| M-3 / M-9 | 保留・trigger付き |

採らない方針: CRDT/LWW、protobuf、薄いadapterのための共有runtime crate、行数削減目的の網羅match・domain型の破壊、初回LAN検証でのTLS/認証。
