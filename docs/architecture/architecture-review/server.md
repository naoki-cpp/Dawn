---
scope    : コードベース全体の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時 / architecture issue更新時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-07-30（Transit state mutation deepening後の再計測）
---

# Architecture Review — Dawn Codebase（現行構造評価）

詳細な判断とtriggerは[server-pending.md](./server-pending.md)、完了履歴は
[server-completed.md](./server-completed.md)を参照する。

## 現状評価

**総合: B+。** crate DAGとdeep module境界は健全。直近では、live/replayのShip materialization、
Station runtime apply、SectorMap projection、client read APIのtyped化、
Transit state mutation deepeningが完了した。

2026-07-30の調査では、行数よりも**同じ状態・projection・authorityの二重所有**を優先課題とした。
Transitについては、Raftの回復判断とShipの状態変更を別moduleに分け、後者を`node::transit`へ集約した。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| クレート構成 | A− | `dawn-core` / `dawn-sector` / `dawn-wire` / client 2 crateのDAGは健全。共有runtime crateは不要 |
| 型設計 | A− | domain固有のResult/Outcomeを維持。dispatcher都合で共通型へ潰さない（ADR-0047） |
| 重複 | A− | live/replayのShip materialization、Station runtime apply、SectorMap projectionを解消。Transit policy/state mutationも分離 |
| 永続化 | A− | snapshot seamとpost-snapshot tail replayの同値性を#197で固定済み |
| Rust固有 | A− | 網羅matchとexhaustive destructuringを変更検出器として利用 |

## 冗長性

解消済み:

- protocol、ClientCommand dispatch、AoI、data loader、NPC spawn loop
- fitting再計算と`ShipFitted` emission tail
- postcard encode/decode
- snapshot constructor/read field list

Open:

1. **M-3** `SectorSimulatorActor`と`SimulationNode`の密結合（保留）
2. **M-9** `EventStore::append`のinfallible contract（保留）

`ClientCommand`外側matchと`StationDispatchCommand`、domain固有の戻り値、process model固有の薄いadapterは
意図的に維持する。

## ファイルサイズ（部分再計測）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-sector/src/node/commands.rs` | 1625 | 🟡 網羅dispatcher・module command・tests |
| `crates/dawn-sector/src/node/transit.rs` | 1793 | 🟡 Transit state mutation・live/replay tests |
| `crates/dawn-sector/src/transit/pipeline.rs` | 581 | 🟢 retry・idempotency・recovery policy |
| `crates/dawn-sector/src/node/command_station.rs` | 264 | 🟢 station family private dispatch |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 923 | 🟡 snapshot/checkpointと復旧tests |
| `client/scripts/main.gd` | 1332 | client側orchestration |

全体表は2026-07-30計測を基準とする。総行数だけでは分割せず、
実装部分約700行超、独立した変更理由の混在、またはdriftの実害をtriggerとする。
