---
id      : ADR-0038
title   : Station Inventory — SQLite-backed lazy projection
status  : accepted
date    : 2026-07-08
deciders: [human, ai-agent]
related : ADR-0034（Economy Foundations）, ADR-0049（Sector recovery authority）, docs/process/roadmap.md §12
---

# ADR-0038 — Station Inventory SQLite Backing

> **ADR-0049 amendment (2026-08-07):** SQLite は Station inventory の独立した
> durable authority ではない。Sector journal の authoritative Station delta が真実の
> 情報源であり、SQLite は `(sector_id, transition_id)` で冪等に更新される bounded/lazy
> query projection である。旧版で許容していた「SQLite と journal の片側だけ durable」な
> crash window は撤回する。commit ordering・ack・recovery は ADR-0049 と
> `docs/architecture/recovery-contract.md` が normative である。

## 背景

`SimulationNode.station_inventories: BTreeMap<(PlayerId, StationId), BTreeMap<ItemId, u64>>`
は全プレイヤー分の Station inventory を起動から終了までメモリに常駐させ、Snapshotにも
全体を含めていた。人口と inventory が増えるほど memory/checkpoint cost が増えるため、
ADR-0034 が残した storage seam を DB-backed implementation へ差し替える必要がある。

`station_inventory_storage()`/`station_inventory_storage_mut()` の seam により、呼び出し側は
生の全件 map を直接知らない。SQLite はこの seam の内側で lazy query/projection backend
として使う。

## 決定

### 1. Authority は Sector journal に置く

Build/Disassemble/Assemble/Transfer 等で Station inventory が変わる場合、その item delta は
ECS/ownership/public outputs と同じ `DurableTransitionBatch` に含めて原子的に commit する。
SQLite を先に更新してから journal を追記する経路、または SQLite だけを success authority
とする経路は禁止する。

Station transition の ordering は:

```text
prepare authoritative Station + ECS mutation
  -> durable journal commit
  -> apply live authoritative reducer
  -> idempotently apply SQLite projection
  -> publish outputs / acknowledge
```

journal commit 後に SQLite projection が失敗した場合、通常の command rejection には戻さず
Sector を fail-stop/fence する。journal から authoritative state を復旧し、SQLite projection を
catch-up/rebuild してから serving を再開する。

### 2. SQLite は bounded lazy projection

メモリ上には直近に触れた `(player, station)` だけの有界 cache を持つ。cache miss では SQLite
から読む。SQLite row は authoritative journal transition を projection した結果であり、各適用は
`(sector_id, transition_id)` で冪等でなければならない。

projection storage は少なくとも:

- applied transition identity の重複検出
- contiguous applied watermark
- Station item rows

を持ち、同じ transition の再適用を no-op にする。replica promotion / recovery 後の serving は、
この watermark が必要な authoritative position に追いつくまで禁止する。

### 3. Checkpoint は Station aggregate を分離可能

全 Station inventory を ECS `StateSnapshot` に戻す必要はない。ADR-0049 の checkpoint manifest は
ECS snapshot と別の versioned Station aggregate checkpoint を同じ covered journal position で
束ねられる。

compaction は ECS snapshot、Station checkpoint、manifest が全て durable/validated になるまで
行ってはならない。SQLite 自体を唯一の checkpoint と仮定せず、projection rebuild に必要な
Station checkpoint + committed Station delta tail を保持する。

### 4. Public event replay は Station authority ではない

`PackagedShipBuilt`/`ShipDisassembled`/`ShipAssembled` 等の `DomainEvent` は durable public fact だが、
Station inventory の exact reducer authority は RecoveryDelta である。したがって public-event replay
だけで SQLite を更新する設計には戻さない。

既存の `apply_event` から Station credit/debit を外した判断は、「public event を二重適用しない」
という意味では維持する。ただしその理由は「SQLite が独立 authority だから」ではなく、
**Station aggregate delta が authoritative recovery stream にあるから**である。

## ADR-0034 との関係

ADR-0034 が避けたのは、毎 Tick/高頻度 read を DB round-trip に依存させる設計である。この ADR は:

- read は bounded cache 優先、miss のみ SQLite
- write は低頻度 Station action の durable transition に限定
- command validation は可能な限り loaded projection/cache 上で行う
- durability authority は SQLite ではなく Sector journal

とするため、DB-backed lazy storage seam の目的を保ちながら cross-store authority を一本化する。

## Crash / retry semantics

- journal append 前に失敗: SQLite projection を変更せず、operation は未commit。
- journal append 後、live reducer 前に crash: recovery が authoritative Station delta を適用する。
- live reducer 後、SQLite 前に crash: recovery/catch-up が同じ transition id を SQLite に適用する。
- SQLite apply 後、ack 前に crash: retry で同じ transition id を再適用しても no-op。
- replica promotion: local SQLite applied watermark が promotion position 未満なら catch-up-only。

これにより旧ADRが許容していた「SQLiteだけ更新済み / journal未commit」または逆方向の片側 authority
状態を正常系契約から除外する。

## 却下した案

- **SQLiteを独立 durable authority のまま維持する**: journal/ECS と cross-store atomicity がなく、
  recoverable 2PC/participant protocol が必要になる。ADR-0049 はより単純な single journal authority
  + idempotent projection を採用するため却下。
- **SQLiteを完全に廃止して全inventoryをECS snapshotへ戻す**: 起動時/メモリ使用量の問題を再導入する。
- **非同期 write-behind で projection lag を無制限に許す**: Station read の正しさと promotion eligibility
  が曖昧になる。projection は遅延可能でも watermark で明示し、authoritative serving 前に catch-up する。

## 帰結

- `dawn-sector` の `rusqlite` 利用と bounded lazy cache は維持する。
- SQLite write API は transition identity を受け取り、冪等適用と contiguous watermark を提供する必要がある。
- Station command は journal durable commit 前に SQLite を authority として変更してはならない。
- Station checkpoint/delta と SQLite projection catch-up test は #271/#272/#284 の recovery implementation に含める。
- 旧版で明記した narrow inconsistency window の許容は撤回する。
