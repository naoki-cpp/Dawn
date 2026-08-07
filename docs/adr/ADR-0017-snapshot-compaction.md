---
id      : ADR-0017
title   : イベントログのスナップショット圧縮と2層ログ（INV-002 改訂）
status  : accepted
date    : 2026-06-14
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0014（Raft / failover）, ADR-0049（Exact Sector recovery）, AI_DEVELOPMENT_GUIDE.md「Architecture Invariants」
---

# ADR-0017 — スナップショット圧縮と2層ログ

> **ADR-0049 amendment (2026-08-07):** 本 ADR の「snapshot + public-event tail」
> および「末尾 Tick を再実行して exact state を得る」という operational recovery の
> 記述は superseded された。Sector の exact operational recovery は
> **compatible versioned checkpoint set + committed authoritative state-delta tail** である。
> `DomainEvent` hot/cold archive は public fact・audit・projection の append-only 履歴として
> 維持する。recovery/commit ordering は ADR-0049 と
> `docs/architecture/recovery-contract.md` が normative である。

## 背景

長寿命シャードでは、全履歴を hot storage に残し続けて genesis から replay する方式は
recovery time と disk 使用量が無制限に増える。一方、過去の public/business fact を
破壊的に書き換えることも避ける必要がある。

当初は `DomainEvent` replay を state reconstruction の中心としていたが、位置・capacitor・
lock countdown・module cycle・eventless Tick 等が public event catalog に完全には現れないことが
明確になった。ADR-0049 は exact state authority を versioned RecoveryDelta に分離した。

## 決定

### 1. Public-event 履歴は hot/cold 2層を維持する

| 層 | 性質 | 用途 |
|---|---|---|
| Hot public-event log | bounded, append-only segment set | recent publication / projection / audit catch-up |
| Cold archive | append-only, long-term | audit / causal trace / disaster-analysis |

committed `DomainEvent` は in-place update/delete/rewrite しない。hot tier から外す場合も、
retention/archive policy と durable consumer cursor を満たした immutable segment 単位で扱う。

### 2. INV-002 の operational recovery 意味を ADR-0049 に合わせる

INV-002 は次の意味に改訂する。

```text
Authoritative Sector state is reconstructed from:
  newest complete compatible checkpoint set
  + every committed authoritative RecoveryDelta after its covered position.

DomainEvent is a durable public/business fact, not the exact state reducer.
Eventless Ticks still have durable recovery transitions.
```

したがって genesis public-event replay、snapshot + public-event tail、historical Tick 再実行は
exact operational recovery の fallback/authority ではない。audit/projection/debug 用途として
明示的に支援することはできる。

### 3. Checkpoint は authoritative recovery point である

checkpoint は単なる性能最適化ではなく、ADR-0049 の recovery authority の一部である。
manifest は少なくとも:

- format/version
- Sector/catalog fingerprint
- covered committed recovery position
- ECS/aggregate snapshot member
- Station aggregate checkpoint member（必要な場合）
- checksums/lengths
- retained event/outbox location/retention metadata

を束ねる。全 member が同一 position で durable/validated になるまで publish/compaction してはならない。

postcard は自己記述形式ではないため、構造変更は明示的な format version change として扱い、
`#[serde(default)]` を後方互換保証と見なさない。

### 4. State-delta compaction と public-event retention を分離する

ADR-0049 の logical atomic transition envelope は state/event/outbox substream を同時に commit するが、
retention lifetime は独立できる。

- state delta: complete checkpoint が覆った範囲を crash-safe manifest swap 後に compact 可
- public event: audit/archive/consumer policy が覆うまで保持
- outbox intent: required consumer が downstream ack 後に durable cursor を進めるまで保持

state checkpoint が存在するだけでは undelivered event/outbox の削除根拠にならない。

### 5. Compaction は copy-and-publish

```text
1. replacement checkpoint / segment set を write + validate
2. files と replacement manifest を fsync
3. manifest を atomic publish
4. publish 後のみ old state-delta segments を retire
5. previous valid manifest へ rollback 可能な material を必要期間保持
```

crash がどの段階で起きても、旧または新の complete recovery path のどちらかが残らなければならない。

### 6. Failover / replica promotion

failover は genesis replay に依存しない。replica は checkpoint + authoritative delta tail を適用し、
さらに retained public-event/outbox segments、durable consumer cursors、Station SQLite projection watermark
を promotion point まで揃える。

ECS state が一致していても、未配信 output/cursor または Station projection が stale なら promotion 不可。

### 7. Transit / Raft との関係

Transit ownership transfer は引き続き ADR-0014 の consensus path を通る。ADR-0049 で reliable runtime
proposal が必要な場合は outbox/idempotency protocol を使う。auto-jump Warp 完了後の Raft proposal は
crash-lossy runtime queue に置かず durable outbox intent とする。

## FBD-001 との関係

FBD-001 の「committed public event を destructive に update/delete/in-place rewrite しない」は維持する。
ADR-0049 の authoritative state-delta stream は public `EventStore` と意味が異なる recovery stream であり、
checkpoint coverage に基づく segment retirement は committed public event の履歴改変ではない。

## Recovery / replay terminology

- **Recovery:** checkpoint + authoritative state-delta tail を適用して exact committed Sector state を復元。
- **Public-event replay:** public projection/audit/debug のために `DomainEvent` を適用。
- **Genesis reconstruction:** operational recovery contract ではない。明示的な offline tool が提供する場合のみ保証範囲を定義する。

## Consequences

- public event archive の監査価値は維持される。
- exact recovery correctness は public event completeness に依存しない。
- state-delta compaction と event/outbox retention を別 watermark で管理できる。
- checkpoint schema/version/manifest は #284/#271 で明示実装する。
- numeric RTO は ADR-0049/#284 の benchmark が完了するまで未定義。

## Implementation checklist

- [x] Public-event hot/cold archive と exact recovery authority を分離する方針を確定。
- [x] Operational recovery を checkpoint + authoritative delta tail に改訂。
- [x] Independent retention / crash-safe compaction semantics を定義。
- [ ] Versioned checkpoint manifest を実装。
- [ ] State/event/outbox substream retention watermark を実装。
- [ ] Replica promotion eligibility と Station projection watermark を実装。
- [ ] #284 で replay benchmark と numeric RTO/checkpoint cadence を確定。
