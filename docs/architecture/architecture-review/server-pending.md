---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-08-02
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

### R-3（保留）: `node/`系ファイルの再肥大

2026-08-02再計測では`node/commands.rs`が1623行、`node/transit.rs`が1757行、
`node/warp.rs`が1190行だった。ただし前二者はdispatcher/state mutationと回帰テストが
同居し、warpは単一のgeometry/state-machine責務である。現時点では即時分割しない。
**再評価:** テストを除く実装部分が約700行を超え、かつ独立した変更理由が混在する、または
module間のdriftが実害になる場合。行数だけでは発火させない。

## 一覧

| 項目 | 状態 |
|---|---|
| R-2 / R-3 | 保留・trigger付き |
| M-3 / M-9 | 保留・trigger付き |

採らない方針: CRDT/LWW、protobuf、薄いadapterのための共有runtime crate、行数削減目的の網羅match・domain型の破壊、初回LAN検証でのTLS/認証。
