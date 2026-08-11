---
scope    : コードベース全体の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時 / architecture issue更新時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-08-11（#305 RuntimeFrameHost統合後、全Rustクレートを再計測）
---

# Architecture Review — Dawn Codebase（現行構造評価）

詳細な判断とtriggerは[server-pending.md](./server-pending.md)、完了履歴は
[server-completed.md](./server-completed.md)を参照する。

## 現状評価

**総合: B。** crate DAGとdeep module境界は健全で、production / single-sector / cluster /
in-process driverは`RuntimeFrameHost`へ統合された。一方、`repositories.rs`（1964行）と
`node/transit.rs`（1902行）は、テストを除いても複数の独立した変更理由を抱え、分割triggerが
発火している。今回の再計測では、共有ランタイムの改善を反映しつつ、実装行数と責務混在を
分けて再評価した。

2026-07-30の調査では、行数よりも**同じ状態・projection・authorityの二重所有**を優先課題とした。
Transitについては、Raftの回復判断とShipの状態変更を別moduleに分け、後者を`node::transit`へ集約した。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| クレート構成 | A− | `dawn-server` が `simulate` と production `sector-node` の唯一のcomposition boundary。`dawn-core` / `dawn-sector` / `dawn-protocol` / client 2 crateへの依存方向も維持 |
| ファイルサイズ | B | `repositories.rs` はAdmission / Identity / Station projectionを一つのSQLite境界に実装し、`transit.rs` はhandoff / materialization / replayを761行の実装に集約。R-3、R-6、R-7へ記録 |
| 型設計 | A− | domain固有のResult/Outcomeを維持。dispatcher都合で共通型へ潰さない（ADR-0047） |
| 重複 | A− | live/replayのShip materialization、Station runtime apply、SectorMap projectionを解消。Transit policy/state mutationも分離 |
| 永続化 | A− | snapshot seamとpost-snapshot tail replayの同値性を#197で固定済み |
| Rust固有 | A− | 網羅matchとexhaustive destructuringを変更検出器として利用 |
| AI開発誘発 | A− | `RuntimeFrameHost`の薄いadapter統合は完了。残るclosure-scoped mutation bridgeと大きなrepository入口は、責務とtriggerを明記してから分割する |

## 冗長性

解消済み:

- protocol、ClientCommand dispatch、AoI、data loader、NPC spawn loop
- fitting再計算と`ShipFitted` emission tail
- postcard encode/decode
- snapshot constructor/read field list

Open:

1. **M-9** `EventStore::append`のinfallible contract（保留）
2. **R-3** `node/transit.rs`のhandoff / replay責務分離（部分発火）
3. **R-6** `RuntimeFrameHost`のFrameInput境界（Fix候補）
4. **R-7** `SectorRepository`のbounded-context分割（Fix候補）

Resolved in #278: production, single-sector, clustered, and in-process test
drivers now call the shared durable runtime frame. `SectorRuntimeDriver` remains
only as an async in-memory adapter; it is not a second Tick implementation.

`ClientCommand`外側matchと`StationDispatchCommand`、domain固有の戻り値、process model固有の薄いadapterは
意図的に維持する。

## ファイルサイズ（2026-08-11再計測、500行以上）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-sector/src/node/repositories.rs` | 1964 | 🔴 Admission / Identity / Station projectionのschema・codec・transaction・testsを一つの入口に集約。R-7でbounded-context分割をFix |
| `crates/dawn-sector/src/node/transit.rs` | 1902 | 🔴 handoff、source/destination materialization、replayを761行の実装に集約。R-3でlifecycle / replay分離をFix |
| `crates/dawn-sector/src/node/tick.rs` | 1520 | 🟢 authoritative tick orderとprepare→durable→applyのkernel・tests。単一の順序機械なので分割しない |
| `crates/dawn-sector/src/node/commands.rs` | 1391 | 🟢 外側のfamily選択・runtime command collection・follow-up射影・統合tests。policyは専用moduleへ分離済み |
| `crates/dawn-storage/src/file_journal.rs` | 1366 | 🟢 versioned journal framing・compaction・corruption recoveryの一つのstorage kernel・tests |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 1246 | 🟢 checkpoint envelope・atomic publication・platform adapter・tests。単一のsnapshot publication boundary |
| `crates/dawn-sector/src/node/warp.rs` | 1203 | 🟡 warp state machine・geometry kernel・tests。実装573行のためR-3の再評価trigger待ち |
| `crates/dawn-sector/src/node/mod.rs` | 1112 | 🟢 node composition・identity/accessor・population/repository boundary。座標helperはR-4で分離済み |
| `crates/dawn-distributed/src/catch_up.rs` | 1052 | 🟢 catch-up / snapshot-tail policy・tests |
| `crates/dawn-market/src/order_book.rs` | 1044 | 🟢 pure order/matching/SettlementIntent policy。SQLは`repository.rs`へ分離済み（#279） |
| `crates/dawn-sector/src/transit.rs` | 1025 | 🟢 runtime consensus / durable transition policy。Ship handoff state mutationとは分離済み |
| `crates/dawn-sector/src/node/orbit.rs` | 990 | 🟢 Orbit / Keep-at-Range steering kernel・tests |
| `crates/dawn-distributed/src/peer_transport.rs` | 963 | 🟡 shared peer framing/lifecycle・control/bulk isolation・tests。adapter surfaceが増えたらprotocol/framing分割 |
| `crates/dawn-sector/src/transit/handoff.rs` | 963 | 🟢 Transit Saga request/commit/ack policy・retry/idempotency/recovery |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 961 | 🟢 snapshot/checkpoint/restore seam・tests |
| `crates/dawn-sector/src/node/inventory.rs` | 940 | 🟢 fitting mutation boundary・tests。cargo操作は`ship_cargo.rs`へ分離済み |
| `crates/dawn-sector/src/client_admission.rs` | 906 | 🟢 admission protocol state machine・tests |
| `crates/dawn-protocol/src/server_fact.rs` | 806 | 🟢 server fact projection/schema・tests |
| `crates/dawn-core/src/commands.rs` | 794 | 🟢 domain command types/validation data・tests |
| `crates/dawn-sector/src/aoi.rs` | 781 | 🟢 AoI index/delta delivery contract・tests |
| `crates/dawn-core/src/events.rs` | 776 | 🟢 domain event catalog/type definitions・tests |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 732 | 🟢 spawn policy・tests |
| `crates/dawn-sector/src/transit/tests.rs` | 676 | 🟢 transit integration tests |
| `crates/dawn-sector/src/node/station_materialization.rs` | 650 | 🟢 station assemble/disassemble materialization・tests |
| `crates/dawn-server/src/cluster.rs` | 648 | 🟢 in-process cluster wiring・fault tests |
| `crates/dawn-server/src/serve/cluster.rs` | 646 | 🟢 clustered serve composition・admission/jump tests |
| `crates/dawn-sector/src/node/approach.rs` | 631 | 🟢 approach steering state machine・tests |
| `crates/dawn-sector/src/node/ship_cargo.rs` | 631 | 🟢 ship cargo ownership/bridge boundary・tests |
| `crates/dawn-market/src/repository.rs` | 623 | 🟡 SQLite order/Currency/outbox persistence。bounded-memory streamingはfollow-up |
| `crates/dawn-distributed/src/state.rs` | 593 | 🟢 Raft state transition/persistence boundary・tests |
| `crates/dawn-ecs/src/systems/combat.rs` | 584 | 🟢 combat system・tests |
| `crates/dawn-server/src/bin/sector-node.rs` | 571 | 🟢 production node bootstrap/config |
| `crates/dawn-sector/src/transition.rs` | 565 | 🟢 durable transition preparation / output boundary |
| `crates/dawn-sector/src/node/apply_event/tests.rs` | 559 | 🟢 event replay tests |
| `crates/dawn-protocol/src/lib.rs` | 554 | 🟢 wire envelope/schema exports・tests |
| `crates/dawn-sector/src/node/serialization.rs` | 540 | 🟢 observer-scoped state projection |
| `crates/dawn-sector/src/node/player_loadout_projection.rs` | 535 | 🟢 PlayerLoadout wire projection |
| `crates/dawn-server/src/runtime_frame.rs` | 522 | 🟢 shared one-Sector frame host・policy injection・output boundary・tests |
| `crates/dawn-sector/src/node/station_operation_execution.rs` | 515 | 🟢 accepted station-operation effects |
| `crates/dawn-server/src/bench.rs` | 510 | 🟢 benchmark scenarios |
| `crates/dawn-sector/src/node/station_lifecycle.rs` | 506 | 🟢 station operation validation/planning |

全体の再計測ではテストコードが行数の大きな割合を占めるファイルが多かった。総行数だけでは
分割せず、実装部分約700行超、独立した変更理由の混在、またはdriftの実害をtriggerとする。
