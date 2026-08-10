---
scope    : コードベース全体の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時 / architecture issue更新時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-08-09（#277 repository split、#278 shared runtime frame後の再計測）
---

# Architecture Review — Dawn Codebase（現行構造評価）

詳細な判断とtriggerは[server-pending.md](./server-pending.md)、完了履歴は
[server-completed.md](./server-completed.md)を参照する。

## 現状評価

**総合: B+。** crate DAGとdeep module境界は健全。直近では、live/replayのShip materialization、
Station runtime apply、SectorMap projection、client read APIのtyped化、
Transit state mutation deepeningが完了した。今回の再計測では、テストを同居させた
大きなRustファイルを含めて現行の責務と分割triggerを再確認した。

2026-07-30の調査では、行数よりも**同じ状態・projection・authorityの二重所有**を優先課題とした。
Transitについては、Raftの回復判断とShipの状態変更を別moduleに分け、後者を`node::transit`へ集約した。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| クレート構成 | A− | `dawn-core` / `dawn-sector` / `dawn-wire` / client 2 crateのDAGは健全。共有runtime crateは不要 |
| ファイルサイズ | B+ | 500行超のRustファイルは複数あるが、主な超過は同居テストまたは単一の状態機械。`commands.rs` はfamily policyを分離済み、`transit.rs` / `warp.rs` はR-3でtriggerを管理 |
| 型設計 | A− | domain固有のResult/Outcomeを維持。dispatcher都合で共通型へ潰さない（ADR-0047） |
| 重複 | A− | live/replayのShip materialization、Station runtime apply、SectorMap projectionを解消。Transit policy/state mutationも分離 |
| 永続化 | A− | snapshot seamとpost-snapshot tail replayの同値性を#197で固定済み |
| Rust固有 | A− | 網羅matchとexhaustive destructuringを変更検出器として利用 |
| AI開発誘発 | A− | deep module候補と薄いadapterの判断をADR/architecture reviewに記録し、行数だけの分割を避けている |

## 冗長性

解消済み:

- protocol、ClientCommand dispatch、AoI、data loader、NPC spawn loop
- fitting再計算と`ShipFitted` emission tail
- postcard encode/decode
- snapshot constructor/read field list

Open:

1. **M-9** `EventStore::append`のinfallible contract（保留）

Resolved in #278: production, single-sector, clustered, and in-process test
drivers now call the shared durable runtime frame. `SectorRuntimeDriver` remains
only as an async in-memory adapter; it is not a second Tick implementation.

`ClientCommand`外側matchと`StationDispatchCommand`、domain固有の戻り値、process model固有の薄いadapterは
意図的に維持する。

## ファイルサイズ（2026-08-09再計測、500行以上）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-sector/src/node/repositories.rs` | 1860 | 🟡 #277のSQLite schema・explicit repository views・allocator/projection tests。bounded-contextごとの追加分割は次回architecture reviewで再計測 |
| `crates/dawn-sector/src/node/transit.rs` | 1694 | 🟡 Transit state mutation・live/replay tests。実装とテストの責務は凝集しておりR-3で監視 |
| `crates/dawn-sector/src/node/commands.rs` | 1286 | 🟢 網羅的family選択・共通runtime command collection・follow-up射影・統合tests。family policyは専用moduleへ分離済み（issue #264、ADR-0047 amendment） |
| `crates/dawn-sector/src/node/warp.rs` | 1190 | 🟢 warp state machine・geometry kernel・tests |
| `crates/dawn-market/src/order_book.rs` | 1139 | 🟡 SQLite authority・Currency escrow・order boundary。matching policyは`matching.rs`へ分離済み |
| `crates/dawn-sector/src/node/orbit.rs` | 950 | 🟢 Orbit / Keep-at-Range steering kernel・tests |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 934 | 🟢 snapshot/checkpoint/restore seam・tests |
| `crates/dawn-sector/src/node/mod.rs` | 980 | 🟢 node state・identity/accessor・population/repository composition boundary・tests |
| `crates/dawn-sector/src/node/inventory.rs` | 923 | 🟢 fitting mutation boundary・tests。cargo操作は`ship_cargo.rs`へ分離済み |
| `crates/dawn-wire/src/client_command.rs` | 868 | 🟢 client command wire schema/conversion・tests |
| `crates/dawn-replication/src/catch_up.rs` | 843 | 🟢 catch-up policy・tests |
| `crates/dawn-wire/src/server_fact.rs` | 816 | 🟢 server fact projection/schema・tests |
| `crates/dawn-core/src/events.rs` | 731 | 🟢 domain event catalog/type definitions・tests |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 725 | 🟢 spawn policy・tests |
| `crates/dawn-sector/src/aoi.rs` | 721 | 🟢 AoI index/delta delivery contract・tests |
| `crates/dawn-sector/src/node/station_materialization.rs` | 644 | 🟢 station assemble/disassemble materialization・tests |
| `crates/dawn-core/src/commands.rs` | 644 | 🟢 domain command types/validation data・tests |
| `crates/dawn-simulation/src/cluster.rs` | 632 | 🟢 cluster runtime wiring・tests |
| `crates/dawn-sector/src/node/approach.rs` | 615 | 🟢 approach steering state machine・tests |
| `crates/dawn-peer-transport/src/lib.rs` | 961 | 🟡 shared peer framing/lifecycle・control/bulk isolation・tests; split protocol/framing if the adapter surface grows further |
| `crates/dawn-consensus/src/state.rs` | 593 | 🟢 Raft state transition/persistence boundary・tests |
| `crates/dawn-ecs/src/systems/combat.rs` | 584 | 🟢 combat system・tests |
| `crates/dawn-sector/src/transit/pipeline.rs` | 577 | 🟢 retry/idempotency/recovery policy |
| `crates/dawn-sector/src/node/ship_cargo.rs` | 577 | 🟢 ship cargo ownership/bridge boundary・tests |
| `crates/dawn-sector/src/node/apply_event/tests.rs` | 539 | 🟢 event replay tests |
| `crates/dawn-sector/src/node/serialization.rs` | 536 | 🟢 observer-scoped state projection |
| `crates/dawn-sector-node/src/main.rs` | 508 | 🟢 production node bootstrap/config |
| `crates/dawn-sector/src/transit/tests.rs` | 534 | 🟢 transit integration tests |
| `crates/dawn-sector/src/node/tick.rs` | 531 | 🟢 authoritative tick ordering |
| `crates/dawn-sector/src/node/station_operation_execution.rs` | 525 | 🟢 accepted station-operation effects |
| `crates/dawn-simulation/src/serve/market_settlement.rs` | 508 | 🟢 Market result to sector bridge |
| `crates/dawn-sector/src/node/player_loadout_projection.rs` | 507 | 🟢 PlayerLoadout wire projection |
| `crates/dawn-simulation/src/bench.rs` | 505 | 🟢 benchmark scenarios |
| `crates/dawn-sector/src/node/station_lifecycle.rs` | 505 | 🟢 station operation validation/planning |

全体の再計測ではテストコードが行数の大きな割合を占めるファイルが多かった。総行数だけでは
分割せず、実装部分約700行超、独立した変更理由の混在、またはdriftの実害をtriggerとする。
