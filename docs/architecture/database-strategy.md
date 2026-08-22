---
scope    : Database/storage selection and migration strategy for Sector recovery, public Event history, Station projection, admission/identity repositories, and Market
audience : AI Agent / Human Developer
update   : When persistence ownership, deployment topology, consistency requirements, or database adapters change
related  : architecture.md, recovery-contract.md, ../adr/ADR-0003-local-first-development.md, ../adr/ADR-0017-snapshot-compaction.md, ../adr/ADR-0034-economy-foundations.md, ../adr/ADR-0038-station-inventory-sqlite.md, ../adr/ADR-0049-sector-recovery-state-delta-wal.md
---

# Dawn Database Strategy

## 1. 結論

SQLite は Dawn の **node-local Station projection**、**admission/identity repository**、
および **単一 `MarketRuntime` が所有する Market** に引き続き適している。今すぐ別DBへ移行する
必要はない。

ただし、ADR-0049により「SQLiteを使うか」と「何がauthorityか」は明確に分離された。

- **Station inventoryのexact Sector-world authority** はversioned recovery journal/checkpointであり、
  SQLiteはidempotent projection/read modelである。
- **prepared admission / resume-ticket lifecycle** はShip materialization前にも存在するため、#277の
  admission/identity repositoryがdurable protocol authorityを持てる。これはStation projectionとは
  別のbounded-context authorityであり、Sector transitionとのreconciliation/promotion条件を
  ADR-0049/recovery-contractが規定する。
- **Market** は別bounded contextであり、現時点では自身のSQLite transactionがMarket内部の
  authorityを持つ。

| 用途 | 現在の実装 | Normative / 将来方針 |
|---|---|---|
| Sector exact recovery | `FileEventStore` + `StateSnapshot` に依存するlegacy path | ADR-0049 authoritative state-delta journal + versioned checkpoint。#271/#272で移行 |
| Public `DomainEvent` history | `FileEventStore` hot/cold 2層ログ | append-only public fact/archiveとして維持。exact recoveryとは別watermark |
| Station inventory | `SectorRepository::station_inventory()` が読む node-local SQLite projection | Sector recovery authorityをidempotentにproject。global contiguous watermarkとtransition dedupを保存 |
| Prepared admission / identity / resume tickets | `AdmissionRepository` / `IdentityRepository` view | #277のdurable protocol authority。予約済みIDはallocatorとconsumed-ID表で再利用しない |
| 単一プロセスMarket | SQLite | SQLite継続 |
| 複数プロセス共有Market | 現在対象外 | PostgreSQLを第一候補として別ADR |
| 世界全体の分散transaction | 導入しない | 実測要件が出た時点で別設計 |

重要なのはDB製品名ではなく、**authority、writer ownership、transaction/recovery boundary、failure
domain**である。

## 2. Sector persistence topology

### 2.1 Exact recovery journal

#284 / ADR-0049 はSector exact operational recoveryを次で固定する。

```text
newest complete compatible checkpoint
    + every contiguous committed authoritative RecoveryDelta after it
```

現在の `FileEventStore` API/formatはこの最終contractをまだ実装していない。#271はfallible atomic
journal framing、commit evidence、fsync/durability evidence、index/receipt、corruption/compactionを実装する。
#272はjournal ownershipをpure Sector engineの外へ出し、prepare -> durable -> live applyを実装する。

ここで「journal」はRDBMSを意味しない。append-oriented file journalのままでも要件を満たせる。
DBへ移すかどうかは#271のmechanicsとは別判断である。

### 2.2 Public Event history

`DomainEvent`はappend-only public/business factであり、audit/projection用途を持つ。ADR-0017の
hot/cold archival価値は維持するが、public Event tailをexact ECS recovery tailと呼ばない。

state-delta checkpoint coverageだけを理由に未配信/未archive public eventを捨ててはならない。
physical retention/index mechanicsは#271が実装する。

普通のWebSocket/AoI clientはdurable consumerではない。disconnectしたpresentation clientのcursorを
public-event retentionの必須watermarkにしてはならず、reconnect/current-state syncで修復する。

### 2.3 Station inventory projection

各Sectorはnode-local SQLiteの`SectorRepository`を使う。Station read modelは
`StationInventoryRepository` viewから直接読むため、`SimulationNode`内に全プレイヤーの
inventoryやinterior-mutability cacheを保持しない。キャッシュを追加する場合は、projectionの
watermarkと無効化を所有するruntime側で行う。

Normative ordering:

```text
prepare Sector/Station authoritative mutation
    -> durable ADR-0049 transition
    -> local live apply
    -> idempotent Station SQLite/repository projection
    -> publish / acknowledge
```

projectionは少なくとも:

- Station-changing transitionのstable identityによるdedup
- global contiguous `projection_applied_through`
- Station item/read rows

を表現できなければならない。非Station transitionもprojection workerのglobal watermark上では
explicit no-opとして通過する。これによりpromotion pointとprojection freshnessを同じjournal
coordinateで比較できる。

SQLiteファイルをnetwork filesystemへ置き、複数nodeから直接openする設計は採らない。

### 2.4 Admission / identity protocol repositories

`crates/dawn-sector/src/node/repositories.rs`の`SectorRepository`はnode-local SQLite connectionの
atomic boundaryだが、呼び出し側には`AdmissionRepository`、`IdentityRepository`、
`StationInventoryRepository`の明示的なviewだけを返す。catch-allの名前や汎用的なforwarding APIを
使わない。複数viewをまたぐ処理だけが`SectorTransaction`を要求する。

このうち**prepared admission / identity / resume-ticket stateはStation projectionではない**。
Ship materialization前のprepared rowはSector ECSに対応するRecoveryDeltaがまだ存在しないため、
#277 repository自身がdurable protocol authorityを持つことをADR-0049は許可する。

必要なordering/reconciliationは次の通り。

```text
fresh reservation:
  reserve PlayerId / ShipId / resume ticket durably in #277 repository
    -> only then expose Welcome

materialization:
  stable admission identity
    -> durable Sector RecoveryDelta for Ship/ownership/active routing
    -> live apply
    -> idempotent repository grant/ticket finalization
    -> acknowledge/serve resume path
```

world transition commit後にrepository finalizationが失敗した場合、同じstable admission identityから
restart時に再試行する。再開時のidentity reconciliationはallocator watermarkだけを再構築し、
すでに消費されたstarter itemを再付与してはならない。別Ship/Playerを再allocateして穴埋めしてはならない。

Replica promotion時も、ECS/RecoveryDeltaがcurrentなだけでは不十分で、promoted nodeがserveする
identityについて#277 repositoryがcaught-upまたはdeterministically reconciledでなければならない。
#278がruntime ordering/error policyを所有し、#280が必要なrepository catch-up data/metadataを運ぶ。

### 2.5 Transit persistence

> **#276 implementation status (2026-08-09):** the historical EventStore scan
> described below is no longer the recovery authority. `TransitSagaSnapshot` is
> stored in `StateSnapshot` and `TickRecoveryDelta`; `TransitAttemptId` provides
> direct lookup, while `OutgoingTransitAttempt` and `IncomingTransitReceipt` own
> retry, terminal, and destination deduplication state. The public EventStore
> remains an audit/projection stream only. The following historical notes describe
> the pre-#276 split and are retained only as migration context.

現行Transitはpublic EventStore scanとsnapshot receiptへretry/dedup authorityが分散している。
これは#276が置換するlegacy implementationである。

#276はdurable `TransitAttemptId`、outgoing attempt、incoming receipt、retry/terminal stateをdirect
lookupできるSagaへ移行する。Sagaをgeneral recovery journalに直接含めるか、別repositoryと明示的に
reconcileするかは#276が決める。ただしADR-0049のRPO/checkpoint/compaction/promotion contractを
弱めてはならない。

## 3. Runtime durability orchestration

`ReplicatedDurable`は単なる「#271がfsyncを増やす」「#280がpacketを送る」という機能ではない。
Runtimeは、どのreplica setを使うか、quorumが何台か、どのowner epochのreceiptを有効とするか、
いつacknowledgementしてよいかを一つのpolicyとして扱う必要がある。

役割分担:

- #271: local/remote durable recordとreceipt/evidenceのstorage semantics
- #278: configured durability profile、replica-set/quorum policy、receipt aggregation、owner epoch/fencing、ack gating
- #280: durability request/receiptとsnapshot/catch-up byte transport、traffic isolation
- #284: RPO/failure-domain semantic requirement

`ReplicatedDurable`は#271/#278/#280が一つのquorum/fencing modelを定義・試験するまでproductionで
有効化/宣伝しない。

## 4. Market persistence

`dawn-market::MarketDb` は注文帳、Currency、Bid escrowを一つのSQLite DBに置く。発注とキャンセルは
DB transaction内で処理されるため、**Market DB内部**の原子性を持つ。

#279で、同じtransactionにsettlement outboxも含める。各intentは単調増加する
`SettlementId`と配送状態を持ち、注文・Currency・escrowと同時にcommitされる。

現在は一つの`MarketRuntime`が一つのconnectionを所有し、SQLiteのsingle-writer特性は実用上の制約に
なっていない。`dawn-market`はSector recovery journalの一部ではなく独立bounded contextである。

現行`orders` schemaのpre-release互換性は要求しない。必要ならclean schemaへ破壊的に移行できる。

## 5. SQLite の限界と移行トリガー

SQLiteは複数readerを扱えるが、一つのDB fileに対するwriterは一つである。データ量だけではなく
**writer ownershipとfailure model**を移行判断に使う。

次のいずれかが現実要件になったらMarketをPostgreSQLへ移すADRを起票する。

- 複数server/processが同じMarketへ書き込む
- Marketを独立network processとして運用する
- Market writer failover / HAが必要になる
- writer待ちが実測latency/throughput bottleneckになる
- backup/PITR/RPO/RTOをexternal DB運用機能で保証する必要がある
- DB fileを別host/shared filesystemに置きたくなる

最後のケースではSQLite file sharingへ進まずclient/server DBを評価する。

- [SQLite: Appropriate Uses For SQLite](https://sqlite.org/whentouse.html)
- [SQLite: Write-Ahead Logging](https://sqlite.org/wal.html)

## 6. Station と Sector journal の整合性 — ADR-0049で決定済み

以前この文書は次の2案を未決定としていた。

- Event/public logを先にdurableにしSQLiteをprojection化する
- Sector eventとStation rowsを同一DB transactionへ統合する

#284 / ADR-0049により、より正確には次が選択された。

> **Public Event-firstではなく、authoritative RecoveryDelta-first。**
> Station mutationはSectorのlogical durable transitionに入り、SQLite/repositoryはそのauthoritative
> transitionをidempotentにprojectする。

これにより「SQLiteだけ更新済み」「public eventだけappend済み」をsuccess authorityとして許容しない。
journal durable後のprojection failureはordinary rejectionではなくfail-stop/catch-up対象である。

この節はStation inventoryについての決定であり、§2.4のpre-materialization admission/identity protocol
stateまでprojection扱いするものではない。

## 7. Market と Sector settlement

Market DB内部のorder/Currency/escrowとsettlement outboxは一transactionにできるが、Marketから
Sector inventoryへ送る片側の在庫操作は別authorityを跨ぐ。PostgreSQLへ移行してもこの跨ぎが
自動的にatomicになるわけではない。

#279の現在の境界は次の通りである:

- `dawn-market`はSQLを知らない純粋な`MarketState` transitionから`SettlementIntent`を生成する。
- `MarketDb`は注文・残高・escrow・outboxを一SQLite transactionでcommitし、`Pending` intentを保持する。
- `dawn-server::serve::market_settlement`だけがintentを`RemoveItemCommand`/
  `ReturnItemCommand`/`CreditItemCommand`へ変換する。
- 各Sector commandには`SettlementId`を渡し、Sectorはcheckpointと`ShipFitted`イベント再生で復元する適用済みIDにより、重複配送をno-opにする。
- 対象Sectorが利用できない間はintentをPendingのまま残し、失敗時は明示的なcompensationまたはTerminalへ遷移する。

これはSector recovery journalの一般outboxと概念的には似るが、Market bounded contextのtransaction
ownershipを保つ。Marketを独立process化する場合も、このoutboxを配送worker/leaseへ拡張する。

## 8. PostgreSQL を将来候補とする理由

共有Marketの第一候補はPostgreSQLとする。

- 複数client/processからの同時更新をserver側で調停できる
- order/balance/escrow/outboxを一transactionに含められる
- Serializable isolation + retryを使える
- backup/monitoring/standby/replicationの運用経路がある
- SQL/constraints/indexを維持でき、現在のMarket modelからの移行距離が短い

PostgreSQLもSector recovery journalとのdistributed transactionを自動提供するわけではない。
採用理由は共有Market writerの調停/運用性である。

- [PostgreSQL: Transactions](https://www.postgresql.org/docs/current/tutorial-transactions.html)
- [PostgreSQL: Transaction Isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL: High Availability, Load Balancing, and Replication](https://www.postgresql.org/docs/current/high-availability.html)

## 9. その他の候補

| 候補 | 評価 | 現時点で採用しない理由 |
|---|---|---|
| RocksDB / redb / LMDB | Sector-local KVには利用可能 | recovery framing/Market複合検索/schema migrationを自前化し、明確な利益がまだない |
| rqlite / dqlite | SQLite系replication | Dawn自身のconsensus/replicationに別合意系を追加する。#284 recovery問題の代替ではない |
| CockroachDB / FoundationDB | distributed transactionが本当に必要なら再評価 | 現負荷に対して運用/障害解析/latency/設計コストが大きい |
| DuckDB | 分析/オフライン集計 | OLTP authority用途ではない |
| MySQL / MariaDB | 共有Marketを実装可能 | PostgreSQLよりDawn固有の明確な利点が現時点でない |

## 10. Module / repository seam の方針

Station inventoryは呼び出し側からSQLite実装を隠せるseamを維持する。#277の実装は
`repositories.rs`に次の境界を固定する。

- `AdmissionRepository`: prepared fresh-admission rowsとticket lookup
- `IdentityRepository`: ownership、current/pending ticket、allocatorの観測
- `StationInventoryRepository`: Station rows、transition dedup、global
  `projection_applied_through`
- `SectorTransaction`: identity consumption、ownership、prepared-row cleanupを一つのSQLite
  transactionへ束ねる。Admission grantはStation rowを直接更新せず、スターター在庫は
  RecoveryDeltaのStation mutationから投影する

Fresh admissionは `reserve_fresh_admission_identity` で、Player/Ship ID・resume ticket・prepared
row・allocator watermarkをcommitしてからWelcomeを返す。abortはlive claimだけを解放し、
`consumed_*_ids` とallocator watermarkは残す。既存DBを開く際とsnapshot後にmaterialized IDを
観測してwatermarkを単調に引き上げるため、予約行が先に失われた場合でもIDを再利用しない。
fresh commit後もprepared rowは次のdurable frameまで保持し、starter Station mutationの投影後に
同じtransitionの`ClientAdmissionCommitted`からownership/grantをidempotentにfinalizeする。
crash recoveryもpublic recordをdecodeして同じreconciliationを行う。

Station projectionはjournal transitionをglobal index順に受け、同じtransition identityと
rangeを重複適用せず、gapを拒否する。非Station transitionは空のmutation sliceでcursorだけを
進める。cursorはbatchの先頭ではなくexclusive endまで進み、public event/effectを含む複数
record batchでも次のtransitionと連続する。#278の共有runtime frameがこのAPIをlocal live apply
の後段で呼び、projection failure時はfail-stop health gateへ入る。#280はcatch-up transportを
所有する。production起動時は実repositoryをRecoveryDelta tail再生前に接続し、catch-upを
一時in-memory adapterへ誤って適用しない。

現在の`MarketDb` public interfaceが`rusqlite::Result`を返す点は、2つ目のadapterを導入する時点で
Market固有errorへ変換する。将来可能性だけを理由に今すぐ抽象traitを増やさない。

## 11. 現在の実行方針

移行期間は「current implementation」と「accepted target」を区別する。

Current implementation:

- `FileEventStore` public log + `StateSnapshot` snapshot-era restore pathが存在する
- `SectorRepository`はSQLite connectionを所有するが、利用側はadmission/identity/Stationの
  explicit viewを通す。Station inventoryのin-memory cacheはない
- fresh identity reservation、allocator watermark再構築、ownership conflict検証、admission grantの
  atomic finalization/idempotency、Station projectionのdedup/global cursor schemaは実装済み
- #278のruntime projection hookとfail-stop/reconciliation orchestrationは共有frameに実装済み。
  Station aggregateはframe-local touched-key overlayとordered RecoveryDelta mutationで扱い、
  `SimulationNode`/checkpointへ全件を常駐・複製しない
- Marketはsingle `MarketRuntime` + SQLite

Accepted target / work package:

- #284/ADR-0049: exact Sector-world recovery = versioned checkpoint + authoritative state-delta tail
- #271: fallible atomic durable journal
- #272: pure engineからstorage ownershipを除去するruntime orchestration
- #277: explicit repository view、fresh identity consumption、Station projection schema/APIを実装
- #276: Transit EventStore scanをdurable Sagaへ置換（implemented）
- #278: runtime durability profile/quorum/fencing/repository reconciliation/ack policyを統一
- #280: selected recovery/repository catch-up/durability representationをpeer transportへ載せる

SQLite fileのnetwork sharing、PostgreSQL/distributed DBの先行導入は行わない。再評価時はbenchmarkだけでなく
writer数、authority、failure recovery、RPO/RTO、運用負荷を入力として新しいADRを作成する。
