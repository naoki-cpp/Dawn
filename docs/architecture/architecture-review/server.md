---
scope    : コードベース全体の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時 / architecture issue更新時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-08-23（#336反映。全Rustクレートの再計測は2026-08-20）
---

# Architecture Review — Dawn Codebase（現行構造評価）

詳細な判断とtriggerは[server-pending.md](./server-pending.md)、完了履歴は
[server-completed.md](./server-completed.md)を参照する。

## 現状評価

**総合: B+。** crate DAGとdeep module境界は健全で、production / single-sector / cluster /
in-process driverは`RuntimeFrameHost`へ統合された。一方、`repositories.rs`（2104行）は複数の
独立した変更理由を抱え、分割triggerが発火している。Transit handoffは今回、root入口を11行へ
縮小し、lifecycle / materializationをprivate moduleへ分離した。今回の再計測では、
共有ランタイムとTransit deepeningを反映しつつ、実装行数と責務混在を分けて再評価した。

2026-07-30の調査では、行数よりも**同じ状態・projection・authorityの二重所有**を優先課題とした。
Transitについては、Raftの回復判断を`transit::handoff`に残し、Shipの状態変更を`node::transit`
配下のlifecycle / materializationへ分けた。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| クレート構成 | A− | `dawn-server` が `simulate` と production `sector-node` の唯一のcomposition boundary。`dawn-core` / `dawn-sector` / `dawn-protocol` / client 2 crateへの依存方向も維持 |
| ファイルサイズ | B+ | `repositories.rs` はAdmission / Identity / Station projectionを一つのSQLite境界に実装しているためR-7へ記録。Transitはroot入口とlifecycle / materializationへ分離済み。旧public-event reverse reducerは削除済み |
| 型設計 | A− | domain固有のResult/Outcomeを維持。dispatcher都合で共通型へ潰さない（ADR-0047） |
| 重複 | A− | Station runtime apply、SectorMap projectionを解消。Transit policy/state mutationも分離し、live専用materializationへ集約 |
| 永続化 | A− | checkpoint + contiguous RecoveryDeltaを唯一のSector復旧境界とし、public-event tailとは独立したcursorをcheckpoint/catch-upへ渡す |
| Rust固有 | A− | 網羅matchとexhaustive destructuringを変更検出器として利用 |
| AI開発誘発 | A− | `RuntimeFrameHost`の薄いadapter統合は完了。残るclosure-scoped mutation bridgeと大きなrepository入口は、責務とtriggerを明記してから分割する |

## 冗長性

解消済み:

- protocol、ClientCommand dispatch、AoI、data loader、NPC spawn loop
- fitting再計算と`ShipFitted` emission tail
- postcard encode/decode
- canonical NodeState checkpoint/delta capture/restore

Open:
1. **R-6** `RuntimeFrameHost`のFrameInput境界（Fix候補）
2. **R-7** `SectorRepository`のbounded-context分割（Fix候補）

Resolved in #336: the legacy EventStore/FileEventStore path is deleted. The
DurableJournal is the sole persistent source of committed public facts, and
replication/catch-up consume the bounded rebuildable PublicEventTail. Cursor
expiry explicitly selects snapshot fallback; the former infallible append
contract and duplicate persistence path no longer exist.

Resolved in #278: production, single-sector, clustered, and in-process test
drivers now call the shared durable runtime frame. `SectorRuntimeDriver` remains
only as an async in-memory adapter; it is not a second Tick implementation.

Station projection production wiring is also complete: preparation carries only
touched-key overlay mutations, the durable RecoveryDelta is applied before the
SQLite read model, and the projection cursor advances by complete journal batch
ranges with no-op progression for non-Station transitions. Production recovery
attaches the real repository before tail replay, and success-side loadout refreshes
run only after the required projection completes.

`ClientCommand`外側matchと`StationDispatchCommand`、domain固有の戻り値、process model固有の薄いadapterは
意図的に維持する。

## ファイルサイズ（2026-08-20再計測、500行以上）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-sector/src/node/repositories.rs` | 2104 | 🔴 Admission / Identity / Station projectionのschema・codec・transaction・testsを一つの入口に集約。R-7でbounded-context分割をFix |
| `crates/dawn-sector/src/node/transit.rs` | 11 | 🟢 lifecycle / materializationを束ねる薄いprivate module root |
| `crates/dawn-sector/src/node/transit/lifecycle.rs` | 512 | 🟢 source freeze / handoff snapshot / Ack cleanup。Saga policyとはprivate node state seamで接続 |
| `crates/dawn-sector/src/node/transit/materialization.rs` | 134 | 🟢 live Commitのhandoff-to-ECS materialization kernel |
| `crates/dawn-sector/src/node/transit/tests.rs` | 880 | 🟢 Transit lifecycle・materialization・checkpoint recovery・cross-Sector統合tests |
| `crates/dawn-sector/src/node/tick.rs` | 1696 | 🟢 authoritative tick orderとprepare→durable→applyのkernel・tests。単一の順序機械なので分割しない |
| `crates/dawn-sector/src/node/commands.rs` | 1382 | 🟢 外側のfamily選択・runtime command collection・follow-up射影・統合tests。policyは専用moduleへ分離済み |
| `crates/dawn-storage/src/file_journal.rs` | 1367 | 🟢 versioned journal framing・compaction・corruption recoveryの一つのstorage kernel・tests |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 1246 | 🟢 checkpoint envelope・atomic publication・platform adapter・tests。単一のsnapshot publication boundary |
| `crates/dawn-sector/src/node/warp.rs` | 1281 | 🟡 warp state machine・geometry kernel・tests。実装573行のためR-3の再評価trigger待ち |
| `crates/dawn-sector/src/node/mod.rs` | 1071 | 🟢 node composition・identity/accessor・population/repository boundary。座標helperはR-4で分離済み |
| `crates/dawn-distributed/src/catch_up.rs` | 1115 | 🟢 catch-up / snapshot-tail policy・tests |
| `crates/dawn-market/src/order_book.rs` | 1044 | 🟢 pure order/matching/SettlementIntent policy。SQLは`repository.rs`へ分離済み（#279） |
| `crates/dawn-sector/src/transit.rs` | 830 | 🟢 runtime consensus / durable transition policy。Ship handoff state mutationとは分離済み |
| `crates/dawn-sector/src/node/orbit.rs` | 990 | 🟢 Orbit / Keep-at-Range steering kernel・tests |
| `crates/dawn-distributed/src/peer_transport.rs` | 963 | 🟡 shared peer framing/lifecycle・control/bulk isolation・tests。adapter surfaceが増えたらprotocol/framing分割 |
| `crates/dawn-sector/src/transit/handoff.rs` | 944 | 🟢 Transit Saga request/commit/ack policy・retry/idempotency/recovery |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 955 | 🟢 snapshot/checkpoint/restore seam・tests |
| `crates/dawn-sector/src/node/inventory.rs` | 908 | 🟢 fitting mutation boundary・tests。cargo操作は`ship_cargo.rs`へ分離済み |
| `crates/dawn-sector/src/client_admission.rs` | 945 | 🟢 admission protocol state machine・tests |
| `crates/dawn-protocol/src/server_fact.rs` | 806 | 🟢 server fact projection/schema・tests |
| `crates/dawn-core/src/commands.rs` | 795 | 🟢 domain command types/validation data・tests |
| `crates/dawn-sector/src/aoi.rs` | 781 | 🟢 AoI index/delta delivery contract・tests |
| `crates/dawn-core/src/events.rs` | 777 | 🟢 domain event catalog/type definitions・tests |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 810 | 🟢 spawn policy・tests |
| `crates/dawn-sector/src/transit/tests.rs` | 679 | 🟢 transit integration tests |
| `crates/dawn-server/src/serve/market_settlement.rs` | 774 | 🟢 Market settlementのdurable frame input / acknowledgement adapter・tests |
| `crates/dawn-sector/src/node/station_materialization.rs` | 650 | 🟢 station assemble/disassemble materialization・tests |
| `crates/dawn-server/src/cluster.rs` | 670 | 🟢 in-process cluster wiring・fault tests |
| `crates/dawn-server/src/serve/cluster.rs` | 671 | 🟢 clustered serve composition・admission/jump tests |
| `crates/dawn-sector/src/node/approach.rs` | 631 | 🟢 approach steering state machine・tests |
| `crates/dawn-sector/src/node/ship_cargo.rs` | 681 | 🟢 ship cargo ownership/bridge boundary・tests |
| `crates/dawn-market/src/repository.rs` | 623 | 🟡 SQLite order/Currency/outbox persistence。bounded-memory streamingはfollow-up |
| `crates/dawn-distributed/src/state.rs` | 594 | 🟢 Raft state transition/persistence boundary・tests |
| `crates/dawn-ecs/src/systems/combat.rs` | 584 | 🟢 combat system・tests |
| `crates/dawn-server/src/bin/sector-node.rs` | 601 | 🟢 production node bootstrap/config・public tail rebuild wiring |
| `crates/dawn-sector/src/transition.rs` | 737 | 🟢 durable transition preparation / output boundary |
| `crates/dawn-protocol/src/lib.rs` | 554 | 🟢 wire envelope/schema exports・tests |
| `crates/dawn-server/src/sector_runtime_driver.rs` | 547 | 🟢 in-process Sector actor/runtime adapter・tests |
| `crates/dawn-server/src/serve/single.rs` | 545 | 🟢 single-sector serve composition・admission tests |
| `crates/dawn-sector/src/node/serialization.rs` | 542 | 🟢 observer-scoped state projection |
| `crates/dawn-sector/src/node/player_loadout_projection.rs` | 535 | 🟢 PlayerLoadout wire projection |
| `crates/dawn-server/src/runtime_frame.rs` | 960 | 🟢 shared one-Sector frame host・policy injection・output boundary・tests |
| `crates/dawn-sector/src/node/station_operation_execution.rs` | 519 | 🟢 accepted station-operation effects |
| `crates/dawn-server/src/bench.rs` | 535 | 🟢 benchmark scenarios |
| `crates/dawn-sector/src/node/station_lifecycle.rs` | 428 | 🟢 station operation validation/planning |

全体の再計測ではテストコードが行数の大きな割合を占めるファイルが多かった。総行数だけでは
分割せず、実装部分約700行超、独立した変更理由の混在、またはdriftの実害をtriggerとする。
