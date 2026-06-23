---
id      : ADR-0014
title   : 分散コンセンサス — Raft による Sector Transit の整合性保証
status  : accepted
date    : 2026-06-12
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0002（Actor モデル）,
          ADR-0003（Local-First）, ADR-0009（星系間ナビゲーション・deferred）,
          CLAUDE.md §3（Dependency DAG）, §5（Entity Ownership）, INV-003, FBD-006
---

# ADR-0014 — 分散コンセンサス（Raft）

## 背景

Phase 6 までのシミュレーションは以下の状態にある。

- 3 ノード構成（`MultiNodeCluster`）は存在するが、各ノードは独立した Sector を
  シミュレートしているだけで、ノード間の合意は存在しない。
- イベント伝播は `dawn-replication::InMemoryReplicationBus`（In-Memory broadcast channel）による
  ベストエフォート転送のみ。
- `SectorTransitRequested` / `SectorTransitCompleted` イベントは
  CLAUDE.md のスコープに記載されているが、**まだ実装されていない**。
  Ship は生成された Sector から出られない。

Phase 7 の完了基準（roadmap §10）:

> ノード障害後に Sector Transit が正しく完了する

Sector Transit は「Ship の所有権を Sector A から Sector B へ移す」操作であり、
2 つのノードが同一 Ship の所有権を同時に主張するスプリットブレインを
構造的に防がなければならない（INV-003 / FBD-006 / CLAUDE.md パターン5）。

---

## 決定

### 0. ADR-0003 の制約解除

ADR-0003（Local-First）は「分散コンセンサスなし（Raft 不使用）」を意図的制約として
列挙し、解除には新しい ADR と人間の承認を要求している。
**本 ADR はこの制約のうち「Raft 不使用」のみを解除する。**
以下の制約は引き続き維持される:

```
維持: Single Process / ネットワーク通信なし / コンテナなし /
      ノード間通信は In-Memory Channel のみ / CRDT なし（Phase 8）
```

### 1. Raft の適用範囲 — 制御プレーンのみ

Raft は**低頻度・高整合性が必要な操作だけ**に適用する。
高頻度の Sector-local イベント（移動・戦闘）には適用しない。

```
Raft を経由する（制御プレーン）:
  - Sector Transit（Ship 所有権の移転）
  - Sector → Node マッピングの変更（リーダー選出によるフェイルオーバー）

Raft を経由しない（データプレーン）:
  - VelocityChanged / WeaponFired / DamageTaken 等の Sector-local イベント
    → 現行は dawn-replication::InMemoryReplicationBus。Phase 8D 後続で TCP gossip に置換予定（CRDT/LWW は ADR-0021 により不採用）
```

理由: Raft のスループットはリーダーの fsync + 過半数 ACK に律速される。
毎 Tick 数千イベントを Raft に流すと Tick SLA（INV-TiDi）を破壊する。
これは本プロジェクトの中心仮説「CRDT と Raft の責務分離による高スループット同期」
（CLAUDE.md §1）そのものである。

### 2. Raft は自前実装する（dawn-consensus クレート）

外部クレート（`openraft`, `raft-rs`）は使わず、
tokio primitive の上に最小 Raft を自前実装する。

```
crates/dawn-consensus/
  src/
    lib.rs
    state.rs      // Follower / Candidate / Leader 状態機械
    log.rs        // Raft Log（EventStore とは別物・後述）
    rpc.rs        // RequestVote / AppendEntries メッセージ型
    actor.rs      // RaftActor（Mailbox 経由で通信・FBD-004 準拠）
    timer.rs      // election timeout / heartbeat（論理駆動・後述）
```

依存方向（CLAUDE.md §3 の予定どおり）:

```
dawn-actor ← dawn-consensus ← dawn-simulation
```

実装する Raft 機能の範囲:

```
実装する:
  - Leader 選出（RequestVote, randomized election timeout）
  - Log Replication（AppendEntries, 過半数コミット）
  - リーダー障害からのフェイルオーバー

実装しない（スコープ外）:
  - Membership Change（3 ノード固定のため不要）
  - Log Compaction / Snapshot Install（Transit ログは小さい）
  - Pre-Vote / Learner などの拡張
```

### 3. Raft Log と EventStore の関係 — Command と Event の分離

Raft Log に積むのは **Command（提案）** であり、
EventStore に積むのは **Event（確定した事実）** である（INV-006）。

```
[1] Sector A のノードが TransitProposal { ship_id, from, to } を
    Raft リーダーに提出する
[2] リーダーが Raft Log に Append し、過半数へ複製する
[3] コミットされた時点で初めて「事実」になる
    → 各ノードが自分の EventStore に SectorTransitRequested を Append
[4] 宛先ノード（Sector B）が Ship を自 ECS に追加完了したら
    TransitCommit を再び Raft に提出
[5] コミット → 各ノードが SectorTransitCompleted を Append
    → 所有権が Sector B に移る
```

Raft Log は合意のための一時的な機構であり、世界の真実は引き続き
EventStore のみが保持する（INV-001 / INV-002）。
Replay に Raft は不要 — Replay は EventStore の
`SectorTransitRequested` / `SectorTransitCompleted` だけで完結する。

### 4. 新規イベント（dawn-core/src/events.rs）

```rust
/// Sector Transit が Raft で合意された（所有権は from のまま）。
pub struct SectorTransitRequested {
    pub ship_id : ShipId,
    pub from    : SectorId,
    pub to      : SectorId,
    pub tick    : Tick,
}

/// Sector Transit が完了した（所有権が to に移った）。
pub struct SectorTransitCompleted {
    pub ship_id  : ShipId,
    pub from     : SectorId,
    pub to       : SectorId,
    pub entry_pos: Position,   // 宛先 Sector での出現座標
    pub velocity : Velocity,   // Transit をまたいで速度を保存（INV-MOVE）
    pub tick     : Tick,
}

/// Sector Transit が中断された（宛先ノード障害など）。所有権は from に残る。
pub struct SectorTransitAborted {
    pub ship_id: ShipId,
    pub from   : SectorId,
    pub to     : SectorId,
    pub tick   : Tick,
}
```

命名規則 — `Rejected` イベントは存在しない:
バリデーション段階の拒否（Ship 不在・Transit 中など）はイベントではなく
`CommandRejected` の返却で表現する（CLAUDE.md §4 / INV-006）。
イベントになるのは `Requested` がコミットされた**後**の中断のみであり、
これを `SectorTransitAborted` と命名する。

`tick` フィールドの解釈 — Sector 間の順序は Raft Log Index が保証する:
Tick は同一 Sector 内でのみ比較可能である（CLAUDE.md §6）。
Transit イベントは 2 つの Sector をまたぐため、**各ノードは自分の
ローカル Tick を刻んで自分の EventStore に Append する**
（同一 Transit でもノードごとに tick 値は異なってよい）。
Sector 間の因果順序（Requested → Completed の順など）は
Raft Log Index が全ノードで一意に定めるため、VectorClock は導入しない。

対応する Command（dawn-core/src/commands.rs）:

```rust
/// Ship を別 Sector へ移す。拒否条件: Ship 不在 / 既に Transit 中。
pub struct TransitCommand {
    pub ship_id: ShipId,
    pub to     : SectorId,
}
```

Transit 中の Ship は CLAUDE.md §5 のとおり `TransitState::InTransit` となり、
Move / Despawn / 二重 Transit を拒否する。

### 5. タイマーは Tick 駆動にする（INV-005 / FBD-003）

Raft の election timeout / heartbeat interval を物理時刻ではなく
**論理 Tick 数**で表現する。

```rust
// 違反: tokio::time::sleep(Duration::from_millis(150)).await
// 正:   heartbeat_interval: u64 = 2   (ticks)
//       election_timeout : u64 = 10 + rng(0..10)  (ticks)
```

各ノードの Tick ループの末尾で RaftActor に `TickElapsed` メッセージを送り、
タイマーの進行を駆動する。テストでは Tick を手動で進めるだけで
リーダー選出・タイムアウトを決定論的に再現できる。

注: Raft の安全性（Safety）はタイマーに依存しない。タイマーは活性（Liveness）
のためだけにあるので、Tick 駆動にしても正しさは損なわれない。

### 6. トランスポートは trait で抽象化し、Phase 7 は In-Process

ADR-0003（Local-First）に従い、Phase 7 では実ネットワークを使わない。

```rust
/// RaftActor 間のメッセージ転送。Phase 7 は mpsc、将来は QUIC/gRPC。
pub trait RaftTransport: Send + Sync {
    fn send(&self, to: NodeId, msg: RaftMessage);
}
```

ノード障害は「該当ノードの RaftActor へのメッセージを遮断する」ことで
シミュレートする（テスト用 `PartitionableTransport`）。

### 7. Tick 処理順序への組み込み

tick-model.md §3 の処理順序は「変更には ADR が必要」と定められている。
本 ADR は以下の変更を承認する（Phase 7 適用時に tick-model.md / CLAUDE.md §6
を更新すること）。

```
Step 2  : コマンドキューを処理する
          TransitCommand → バリデーション後、RaftActor へ TransitProposal を
          提出する（この時点ではイベントを発行しない。提案は非同期に
          コミットされる）
          Transit 中の Ship への Move / Despawn / 二重 Transit は拒否

Step 7.5: コミット済み Raft エントリを適用する（新規ステップ）
          RaftActor から受信したコミット済みエントリを ECS に適用し、
          SectorTransitRequested / Completed / Aborted イベントを生成する
          - from ノード: Completed 適用時に Ship を自 ECS から削除
          - to   ノード: Completed 適用時に Ship を entry_pos に追加
          ※ Step 8（Append）の前に行うこと — 同 Tick の Append に含めるため
          ※ 実装上は Tick ハンドラ冒頭（Step 1 の前）で実行する
            （SectorSimulatorActor::apply_committed_raft_entries()）。
            前 Tick までにコミットされたエントリを今 Tick の Step 1〜9 へ
            確実に反映させるための配置であり、上記の論理的位置
            （Step 8 の前）という制約とは矛盾しない。

Step 10 : RaftActor に TickElapsed を送る（新規ステップ・最後）
          election timeout / heartbeat タイマーを 1 Tick 進める
```

Step 1〜7（既存のシミュレーション処理）と Step 8〜9（Append → Replication）
の順序関係は一切変更しない。Step 7.5 はその前段（Tick ハンドラ冒頭）で
コミット済みエントリを取り込む。

---

## 却下した代替案

### 案A: openraft / raft-rs の採用

**却下理由**: 本プロジェクトは分散同期を**競争優位**に据えており（ADR-0016）、
合意プロトコルの挙動を完全に把握・計測できることが目的に含まれる。
また ADR-0002 と同じ理由 — 外部フレームワーク固有のパターンに
AI 生成コードが依存すると、レビュー可能性が下がる。
openraft は独自のストレージ trait / 非同期モデルを強制し、
Tick 駆動タイマー（決定 5）と整合しない。

### 案B: 2-Phase Commit（2PC）による Transit

**却下理由**: コーディネータ障害でブロックする。
完了基準「ノード障害後に Transit が正しく完了する」を満たせない。

### 案C: 全イベントを Raft に流す（単一 Raft Log）

**却下理由**: スループットがリーダーに律速され Tick SLA を破壊する。
中心仮説（責務分離）の放棄に等しい。

---

## スコープ制約

```
追加する:
  - crates/dawn-consensus（Leader 選出 + Log Replication + フェイルオーバー）
  - SectorTransitRequested / Completed / Aborted イベント
  - TransitCommand コマンド / TransitState コンポーネント
  - MultiNodeCluster への RaftActor 配線
  - シナリオテスト: ノード障害中の Transit 完遂

追加しない:
  - Membership Change / Log Compaction
  - 実ネットワーク（Phase 5 の WebSocket は Godot 用であり Raft とは別）
  - Sector-local log gossip（Phase 8D / dawn-replication。CRDT/LWW は ADR-0021 により不採用）
  - JumpGate / StarSystem（ADR-0009、Phase 7 完了後）
```

---

## 実装チェックリスト

### dawn-core

- [x] `src/events.rs` に `SectorTransitRequested` / `Completed` / `Aborted` 追加
- [x] `src/commands.rs` に `TransitCommand` 追加
- [x] 各型に単体テスト追加
- [x] `docs/architecture/event-catalog.md` 更新（§3.6 の予約 `SectorTransitRejected` を
      `SectorTransitAborted` にリネームし、型定義済みへ移行）
- [x] `docs/architecture/tick-model.md` §3 に Step 7.5（実装済み・Tick ハンドラ冒頭）/ Step 10 を追記
- [x] CLAUDE.md §6 に Step 10 を追記（人間の承認を得て）

### dawn-consensus（新規クレート）

- [x] 状態機械（Follower / Candidate / Leader）+ 単体テスト
- [x] RequestVote / AppendEntries の処理 + 単体テスト
- [x] Tick 駆動 election timeout / heartbeat
- [x] `RaftActor`（Mailbox 経由・FBD-004 準拠）
- [x] `RaftTransport` trait + In-Process 実装 + `PartitionableTransport`
- [x] CLAUDE.md §11 の Crate 表更新（人間の承認を得て）

### dawn-ecs

- [x] `TransitState` コンポーネント（None / InTransit）+ Transit 中の操作拒否

### dawn-simulation

- [x] `SimulationNode` に Transit 処理（提案 → コミット → 完了の 2 段階）
- [x] `MultiNodeCluster` に RaftActor 配線
- [x] シナリオテスト（node.rs / cluster.rs の `#[cfg(test)]`）:
  - [x] 正常系: Transit 後、Ship が宛先 Sector にのみ存在する
  - [x] リーダー障害: パーティション後に新Leaderが高いTermで選出される
  - [x] スプリットブレイン不在: いかなる時点でも所有 Sector は高々 1 つ /
        到達可能なノード間でLeaderは高々 1 つ
  - [x] INV-002: Transit 後の状態がSnapshot + Log Replayで完全再現される
  - [x] 完了基準そのもの: 旧Leaderの障害中に提案されたTransitが、
        新Leader選出後に完遂する（`transit_completes_after_a_new_leader_is_elected_during_node_failure`）
- [x] ベンチマーク: Transit 1 回のレイテンシ（docs/process/benchmark-baseline.md に追記）

---

## 参照

- CLAUDE.md §1（中心仮説: CRDT と Raft の責務分離）, §3, §5, INV-003, INV-005, INV-006, FBD-003, FBD-004, FBD-006
- ADR-0002: Actor モデル（tokio primitive 自前実装の先例）
- ADR-0003: Local-First（In-Process トランスポートの根拠）
- docs/process/roadmap.md §10: Phase 7 完了基準
- Diego Ongaro, John Ousterhout — "In Search of an Understandable Consensus Algorithm" (Raft paper)
