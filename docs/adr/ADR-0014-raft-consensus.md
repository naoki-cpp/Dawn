---
id      : ADR-0014
title   : 分散コンセンサス — Raft による Sector Transit
status  : accepted
date    : 2026-06-12
updated : 2026-08-02
deciders: [human, ai-agent]
related : ADR-0001, ADR-0002, ADR-0003, ADR-0009, ADR-0017, INV-002, INV-003, INV-006
---

# ADR-0014 — Raft による Sector Transit

## 背景

Sector Transit は Ship の実行責任を Sector A から Sector B へ移す操作である。
通常の移動・戦闘イベントと違い、消失・二重実行・再起動後の巻き戻りを許容できない。

Raft はプロセス再起動後に復元されない制御プレーンであり、永続的な世界の真実は
引き続き各 Sector の EventStore が持つ。したがって、復旧は snapshot + tail log
だけで完結しなければならない。

## 決定

### 1. Raft の適用範囲

Raft は低頻度で強い整合性が必要な制御操作だけに使う。

- 対象: Sector Transit、Sector→Node mapping
- 対象外: 移動、module cycle、lock、戦闘などの Sector-local tick処理

Command は Raft Log、確定した事実は EventStore に記録する。両者を同じ型にしない
（INV-006）。

### 2. Transit protocol

Transit は Request → Commit → Ack の3段階とする。

```text
Request committed
  source:
    SectorTransitRequested をappend
    ShipをInTransitとしてsource ECSに保持
    TransitHandoffStateを含むCommitをproposal

Commit committed
  destination:
    request identity markerをEventStoreへappend
    Shipを冪等にmaterialize・re-anchor
    SectorTransitCompletedをappend
    Ackをproposal

Ack committed
  source:
    元のrequest identityを検証
    frozen recovery copyを削除
    SectorTransitCompletedをappend
```

source削除はdestinationのdurable completionより後に行う。これにより、途中停止時に
Shipがどちらにも存在しないwindowを作らない。Commit後Ack前は両ECSにcopyが存在するが、
source copyは凍結された復旧用でありsimulationへ参加しない。active ownerはdestinationだけである。

### 3. Durable retry

source側の未解決`SectorTransitRequested`をdurable outboxとして扱う。

- `Requested - Completed/Aborted`をEventStoreから再構築する
- 再起動後はfrozen Shipから同じ`TransitHandoffState`を再生成してCommitを再proposalする
- duplicate Commitはdestinationでmaterializeし直さずAckだけ再発行する
- duplicate Ackはsourceでno-opになる
- outgoing Requestが未解決の間はcheckpoint compactionを延期し、retry recordをhot logに残す

Raft stateの永続化やプロセス生存を前提にしない。

### 4. Event payload

`SectorTransitCompleted`はdestination replayに必要な状態を自己完結で持つ。

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

同じ`TransitHandoffState`をRaft Commitと`SectorTransitCompleted`が共有する。
`owner_player_id`もhandoff/eventの一部としてdestinationへ移送し、destinationの
durable owner bindingとlive ownership mapを同じCommitから復元する。NPCや未所有Shipは
`None`とする。checkpoint後にCompletedがcold archiveへ移動しても、snapshotのShip→Player
bindingがresumeのcompare-and-setを維持する。
`SectorTransitRequested`、retry reconstruction、Raft Commit、`SectorTransitCompleted`は
到着事実を一つの`entry_pos: AbsolutePosition`として伝播する。destinationはこの絶対座標から
anchorとlocal offsetをmaterialization時に導出し、live Commitとreplayは同じ実装を通る。
永続化用`ShipSnapshot`はsnapshot/restore境界だけに留まり、consensus payloadへ流用しない。
`position`・`anchor`はdestination-localな派生表現であり、`tackled_by`も
Sector-localなのでhandoffへ含めない。AckはShip stateを返さず、
`ship_id + from + to + request_tick`だけでattemptを照合する。
`request_tick`はsource-localなattempt identityであり、Request → Commit → Completed → Ackの
全経路で変更せず伝播する。同じShipが同じ経路を複数回通っても別attemptとして照合する。

### 5. Replay

- `SectorTransitRequested`: sourceにShipが存在すれば`InTransit { to }`へ戻す
- `SectorTransitCompleted` on source: Shipを削除する
- `SectorTransitCompleted` on destination: `handoff`と絶対`entry_pos`をlive importと同じmaterialization seamへ渡し、同じanchor・offsetを導出する
- `SectorTransitAborted`: `InTransit`を解除する

live importはmaterialization seamが返す`AnchorRebased`を`SectorTransitCompleted`より先に記録する。
destination replayは同じseamで状態だけを再構築し、既にlogにあるeventは再appendしない。

### 6. InTransit freeze

Request後Ack前のsource Shipはsimulation stateを変更しない。

- 新規Move / Stop / Approach / Orbit / Keep-at-range / Warp / Jump / Transitを拒否
- steering・warp・movementを進めない
- capacitor recharge、module cycle、forced deactivationを進めない
- lock admission、combat、repairを適用しない
- bot commandも同じcommand guardを通す

freeze前に取得したCommit payloadとsource stateを一致させるための要件である。

### 7. Tickと順序

各Sector eventの`tick`はそのSectorのlocal logical Tickであり、Sector間で比較しない。
同一transferのidentityは`ship_id + from + to + request_tick`で照合する。
Raft timerもwall clockではなくlogical Tickで駆動する。

## 結果

- snapshot + tail replayだけでsource/destinationを復旧できる
- Request後、Commit後、Ack前後の再起動を冪等再試行で収束できる
- 一時的なfrozen recovery copyを許容し、ゼロcopyと二重active ownerを防ぐ
- EventStore scanとretry proposalの小さなコストを受け入れる

## 検証

PR #206とPR #210は次を固定する。

- Request後Commit前の再起動でsource Shipが残り、Commitを再proposalする
- destination Commit後にAckが発行され、Ack後source copyが消える
- Ack待ちの往路frozen copyが残るSectorへShipが戻っても、旧outboxを先に閉じ、遅延Ackで返送Shipを消失させない
- duplicate Commitが1回だけmaterializeしAckを再発行する
- completed transitのsnapshot + tail replayでdestinationだけがactiveになる
- InTransit中のposition・velocity・HP・capacitor・fitting・inventoryがtickで変化しない
