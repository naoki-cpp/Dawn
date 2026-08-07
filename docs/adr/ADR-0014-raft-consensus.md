---
id      : ADR-0014
title   : 分散コンセンサス — Raft による Sector Transit
status  : accepted
date    : 2026-06-12
updated : 2026-08-07
deciders: [human, ai-agent]
related : ADR-0001, ADR-0002, ADR-0003, ADR-0009, ADR-0017, ADR-0049, INV-002, INV-003, INV-006
---

# ADR-0014 — Raft による Sector Transit

> **ADR-0049 amendment (2026-08-07):** 本 ADR の ownership-transfer behavior と
> Raft consensus requirement は維持する。一方、`EventStore` を exact recovery authority
> とみなし、`SectorTransitRequested`/`Completed` の scan を durable retry repository とする
> persistence 部分は superseded された。Sector exact recovery は ADR-0049 の versioned
> checkpoint + authoritative state-delta tail に従い、Transit の long-term retry/receipt authority
> は #276 の durable handoff Saga が担う。本 ADR の event-scan/replay 記述は #276 migration
> 前の**現行実装・歴史的baseline**として読む。

## 背景

Sector Transit は Ship の実行責任を Sector A から Sector B へ移す操作である。
通常の移動・戦闘イベントと違い、消失・二重実行・再起動後の巻き戻りを許容できない。

本 ADR の原決定時、Raft はプロセス再起動後に復元されない制御プレーンであり、永続的な
世界状態は各 Sector の EventStore + snapshot から復旧する設計だった。その後 #284 /
ADR-0049 で、public `DomainEvent` tail が exact per-Tick state を完全には表現しないことを
明示し、**versioned authoritative state-delta journal + checkpoint** を operational recovery
authority として選択した。

したがって現在の境界は:

- Raft: cross-Sector ownership transition の consensus/control path
- RecoveryDelta/checkpoint: exact committed Sector/Transit state の recovery authority
- DomainEvent: durable public/business fact / audit / projection
- #276 Saga: Transit attempt/receipt/retry/terminal lifecycle の long-term durable authority

である。

## 決定

### 1. Raft の適用範囲

Raft は低頻度で強い整合性が必要な制御操作だけに使う。

- 対象: Sector Transit、Sector→Node mapping
- 対象外: 移動、module cycle、lock、戦闘などの Sector-local tick処理

Command/consensus operation と public `DomainEvent` は同じ型にしない（INV-006）。確定した
public/business fact は引き続き event として記録できるが、exact recovery state は ADR-0049
の `RecoveryDelta` が authority である。

### 2. Transit protocol

Transit の behavioral baseline は Request → Commit → Ack の3段階とする。

```text
Request committed
  source:
    SectorTransitRequested public fact
    ShipをInTransitとしてsource authorityに保持
    canonical handoffを含むCommit proposal

Commit committed
  destination:
    attempt identityを冪等に受理
    Shipをmaterialize・re-anchor
    SectorTransitCompleted public fact
    Ack proposal

Ack committed
  source:
    attempt identityを検証
    frozen recovery copyを削除
    SectorTransitCompleted public fact
```

source削除はdestinationのdurable completionより後に行う。これにより途中停止時にShipが
どちらにも存在しないwindowを作らない。Commit後Ack前は両側にcopyが存在し得るが、source
copyは凍結された復旧用でsimulationへ参加せず、active ownerはdestinationだけである。

現在の実装では Request 適用時に Ship を一時的に`InTransit`へ変更してcanonicalな
`TransitHandoffState`を構築し、`SectorTransitRequested`をappendする。これは #272 の
prepare -> durable -> live apply 境界へ移行される対象であり、**live mutation before durable
append は normative ordering ではない**。

Request準備、destination materialization、source cleanupの低水準操作はcrate-privateとし、
外部crateは`transit/handoff.rs`のidentity検証・冪等性・cleanup判定を迂回しないという既存
behavioral seamは維持する。

### 3. Durable retry — legacy baseline と #276 への移行

現行コードは未解決`SectorTransitRequested`を事実上のdurable outboxとして扱う:

- `Requested - Completed/Aborted`をEventStoreから再構築する
- 再起動後はfrozen Shipから同じ`TransitHandoffState`を再生成してCommitを再proposalする
- duplicate Commitはdestinationでmaterializeし直さずAckだけ再発行する
- duplicate Ackはsourceでno-opになる
- outgoing Requestが未解決の間はcheckpoint compactionを延期し、retry recordをhot logに残す

この方式は **#276 が置換する legacy persistence model** であり、ADR-0049後の最終 recovery
authorityではない。#276は少なくとも:

- first-class `TransitAttemptId`
- direct keyed outgoing attempt state
- destination inbox/receipt authority
- retry count/deadline/terminal outcome
- crash-safe source freeze / proposal / destination materialization / Ack / cleanup ordering
- checkpoint/compaction/replica catch-up integration

を ADR-0049 の durability boundary の下で定義する。

現在の`request_tick` identity、Abortのroute照合、event scanは移行baselineとして残すが、#276は
後方互換性を要求されず、より明示的なattempt identityへ変更できる。

Raft actor/process stateそのものの生存を recovery authority として前提にしない。

### 4. Event payload

現在の`SectorTransitCompleted`は public completion fact としてcanonical handoff情報を多く持つ。

```rust
pub struct TransitHandoffState {
    pub ship_id: ShipId,
    pub owner_player_id: Option<PlayerId>,
    pub ship_type_id: ShipTypeId,
    pub velocity: Velocity,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    pub capacitor: Option<f32>,
    pub fitting: FittingSnapshot,
    pub inventory: BTreeMap<ItemId, u64>,
}

pub struct SectorTransitCompleted {
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
    pub entry_pos: AbsolutePosition,
    pub tick: Tick,
}
```

同じ`TransitHandoffState`をRaft Commitとpublic completion eventが共有する現行設計は、wire/audit
上有用な限り維持できる。ただしADR-0049後、**このevent payloadだけでexact destination
recoveryを保証する必要はない**。Player ownership、active-ship routing、position/anchor、Transit
attempt stateなどのexact authorityはRecoveryDelta/checkpoint/#276 Sagaの組合せが担う。

`request_tick`は現行attempt identityである。#276はopaqueな`TransitAttemptId`へ置換可能であり、
public event schemaも必要ならpre-release destructive refactorとして変更できる。

### 5. Public-event Replay と exact Recovery

現行public-event replay semanticsはprojection/debug/legacy restore pathとして:

- `SectorTransitRequested`: source projectionにShipが存在すれば`InTransit { to }`を反映
- `SectorTransitCompleted` on source: source Ship projectionを削除
- `SectorTransitCompleted` on destination: handoff/entry positionからdestination projectionを構築
- `SectorTransitAborted`:一致するpending route projectionを解除

という情報を持つ。

ただし **operational exact recovery はこのevent replayではない**。ADR-0049に従い、compatible
checkpoint + committed authoritative recovery tailを適用する。#276 Saga stateがgeneral recovery
journal外のrepositoryを使う場合、そのrepositoryとのreconciliationも #276 が crash matrixに
明示する。

live importとrecovery reducerは可能な限り同じmaterialization/invariant primitivesを共有する。
既にdurableなpublic eventをrecovery中に再appendしてはならない。

### 6. InTransit freeze

Request後Ack前のsource Shipはsimulation stateを変更しない。

- 新規Move / Stop / Approach / Orbit / Keep-at-range / Warp / Jump / Transitを拒否
- steering・warp・movementを進めない
- capacitor recharge、module cycle、forced deactivationを進めない
- lock admission、combat、repairを適用しない
- bot commandも同じcommand guardを通す

freezeはhandoffの単一active-owner semanticsを守るためのbehavioral invariantであり、#276でも
維持する。freeze state自体はADR-0049のauthoritative recovery対象である。

### 7. Tickと順序

各Sector public eventの`tick`はそのSectorのlocal logical Tickであり、Sector間で比較しない。
現行transfer identityは`ship_id + from + to + request_tick`だが、#276はopaque attempt IDへ
移行できる。Raft timerはwall clockではなくlogical Tickで駆動する。

Reliable Raft proposalがcommitted Sector stateから派生する場合、proposal自体をmemory-only queueに
残してはならない。ADR-0049のdurable retry invariant、または#276 Sagaの同等保証を使う。

## 結果

維持される結果:

- Sector TransitはRaft consensus pathを迂回しない
- source deletionはdestination durable completionより後
- duplicate Request/Commit/Ackは冪等に収束させる
- InTransit source copyはfrozenで、二重active ownerを作らない

ADR-0049/#276で変更される結果:

- exact recoveryはpublic EventStore scanではなくcheckpoint + authoritative recovery tail
- pending Transit retryはevent pair scanを最終repositoryとしない
- checkpoint compactionはpending public Request eventをhot logに残すことへ依存しない
- replica catch-upはauthoritative recovery + Saga/retry authorityを欠いた状態でpromoteしない

## 検証

PR #206 / #210 / #269系で固定された既存behavioral regression testsは、#276 migrationのbaseline
として価値を持つ:

- Request後Commit前の再起動でsource Shipを失わない
- handoff構築失敗で半端なownership transitionを残さない
- destination Commit後にAck、Ack後にsource frozen copyを消す
- 遅延Ackで新しくsourceへ戻ったShipを誤削除しない
- duplicate Commitが二重materializeしない
- 古いrouteのAbortが新しいattemptを解除しない
- InTransit中のposition・velocity・HP・capacitor・fitting・inventoryがtickで変化しない

加えて #276/#284 の最終検証では:

- event-log scanなしでpending attemptをdirect lookupできる
- retry/receipt/terminal stateがcrash後も一意
- auto-jump/Transit proposal直前・直後crashがidempotentに収束する
- checkpoint/compaction後もpending attemptを失わない
- replicaはSaga/recovery authorityが揃うまでpromoteされない

ことを固定する。
