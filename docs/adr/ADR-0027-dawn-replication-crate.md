---
id      : ADR-0027
title   : dawn-replication クレートの新設（ゴシップ配布 + アンチエントロピー + スナップショット転送）
status  : accepted
date    : 2026-06-19
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0002（Actor）, ADR-0017（Snapshot / Compaction）,
          ADR-0021（Sector-local 複製戦略・追記ログのゴシップ配布）, ADR-0026（dawn-sector）,
          AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」（Dependency DAG / Crate 別責務早見表）,
          .claude/commands/ai-change-checklist.md（新クレート手順）
---

# ADR-0027 — dawn-replication クレートの新設

## 背景

ADR-0021 は Sector-local 複製の戦略を「追記ログのゴシップ配布（log shipping）」に確定した
（CRDT / LWW は採らない）。Phase 8D でこれを物理ノード上で動作させるには、
当該責務を独立したクレートとして実装する必要がある。

現在 `dawn-actor` に存在する `ReplicationBus`（In-Memory broadcast チャネル）は
"シングルプロセス内テスト用スタンドイン"であり、ネットワーク越しの実装ではない。
本 ADR は `dawn-replication` クレートを新設し、ADR-0021 が定めた 3 責務を実装する。

## 決定

### 1. クレートの新設と責務

`crates/dawn-replication/` を新設する。責務（ADR-0021 §決定 3 を再確認）:

```
1. 追記ログのゴシップ配布（push-pull, log index ベース差分）
2. アンチエントロピー（iter_from を使った取りこぼし再要求）
3. スナップショット転送（遅れた複製が snapshot + tail catch-up で追いつく）
```

クレートは **ワイヤ層（トランスポート）を抽象化する trait** を提供し、
In-Memory（テスト用・`ReplicationBus` の後継）と TCP（本番用）の
2 実装を持つ。postcard + serde を wire 形式に再利用する（§3 方針）。

### 2. Dependency DAG 上の位置

```
dawn-core
    ↑
    ├── dawn-ecs
    ├── dawn-consensus
    └── dawn-event-store
            ↑
            ├── dawn-actor          ← ClientConnection trait は残留
            ├── dawn-replication    ← 新設（ReplicationBus の後継 + ゴシップ実装）
            └── dawn-sector
                    ↑
                    └── dawn-simulation
```

`dawn-replication` は `dawn-event-store`（`EventStore` trait + `iter_from`）に
依存し、`dawn-sector` からは依存しない（矢印の方向に従う）。

### 3. dawn-actor の ReplicationBus の扱い

`dawn-actor::ReplicationBus`（In-Memory broadcast）は `dawn-replication` の
`InMemoryReplicationBus` へ置き換える。移行後、`dawn-actor` から削除する。
`dawn-actor` は `ClientConnection` trait と実装（In-Process / WebSocket）のみを保持する。

### 4. 公開インターフェース（最小 MVP）

```rust
// dawn-replication/src/lib.rs

/// A single gossip message: one or more events from a source node's log.
pub struct LogBatch {
    pub sector_id  : SectorId,
    pub from_index : u64,
    pub events     : Vec<DomainEvent>,
}

/// Core trait — wire-format-agnostic replication transport.
pub trait ReplicationTransport: Send + Sync {
    /// Send a batch of events to all interested peers.
    fn broadcast(&self, batch: LogBatch);
    /// Subscribe to incoming batches from peers.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LogBatch>;
}

/// In-memory implementation for single-process tests (replaces ReplicationBus).
pub struct InMemoryReplicationBus { ... }
impl ReplicationTransport for InMemoryReplicationBus { ... }

/// Sender-side cursor and LogBatch construction for an owning Sector's append log.
pub struct OutboundLogPublisher<T: ReplicationTransport> { ... }

/// Anti-entropy: request missing events from a peer by log index range.
pub struct AntiEntropy { ... }

/// Snapshot transfer: send/receive a StateSnapshot for far-behind replicas.
pub struct SnapshotTransfer { ... }
```

### 5. Phase 8D の実装順序

```
8D-2a: dawn-replication クレート新設
        - InMemoryReplicationBus（dawn-actor::ReplicationBus の移植）
        - ReplicationTransport trait
        - dawn-actor から ReplicationBus を削除
        - dawn-simulation の配線を InMemoryReplicationBus に差し替え

8D-2b: AntiEntropy（iter_from ベース取りこぼし再要求）
        - ゴシップ受信側が log index gap を検出して再要求
        - 重複受信は (SectorId, log_index) で冪等に drop

8D-2c: TCP ゴシップ実装（TcpReplicationTransport）
        - framing: 4-byte length prefix + postcard（wire 形式）
        - plaintext（Phase 8D first milestone: LAN 平文 Pi 検証まで TLS 不要）

8D-2d: SnapshotTransfer（遅れた複製の追いつき）
        - 送信側: StateSnapshot を postcard でシリアライズして transfer
        - 受信側: restore_from（ADR-0017）で復元してから tail catch-up
```

---

## Dependency DAG 上の許可依存（dawn-replication/Cargo.toml）

```toml
[dependencies]
dawn-core        = { path = "../dawn-core" }
dawn-event-store = { path = "../dawn-event-store" }
serde            = { version = "1", features = ["derive"] }
postcard         = { version = "1", features = ["alloc"] }
tokio            = { version = "1", features = ["sync", "net", "io-util"] }
thiserror        = "1"
```

禁止: `dawn-ecs`, `dawn-sector`, `dawn-consensus`, `dawn-simulation`

---

## 不変条件との関係

| 不変条件 | 確認 |
|---|---|
| INV-001 | ゴシップは append-only ログを「配る」のみ。上書き・削除なし |
| INV-002 | 遅れた複製は snapshot + tail catch-up で復元（ADR-0017）|
| INV-004 | 重複受信は (SectorId, log_index) で冪等に drop |
| INV-005 | 整列は (Tick, NodeId)。物理時刻不使用（FBD-003）|
| FBD-004 | Actor 間通信は Mailbox 経由のみ。Transport は mpsc/broadcast を通す |

---

## 代替案

- **`dawn-actor` に直接 TCP 実装を追加**: `dawn-actor` が ClientConnection と
  Replication の 2 責務を持ち肥大化する。独立クレートにした方が責務が明確。却下。
- **`dawn-sector` 内に閉じる**: DAG 違反（replication が sector より上位に位置すべき）。
  テスト時の In-Memory 差し替えも困難。却下。

---

## 実装チェックリスト

- [x] 人間が本 ADR を承認する（proposed → accepted）
- [x] `crates/dawn-replication/` を新設（Cargo.toml + src/lib.rs）
- [x] `ReplicationTransport` trait + `InMemoryReplicationBus` を実装
- [x] `OutboundLogPublisher` で送信側 cursor と `LogBatch` suffix 構築を集約
- [x] `dawn-actor` から `ReplicationBus` を削除し、`dawn-simulation` を差し替え
- [x] `AntiEntropy`（iter_from ベース）を実装しテストを書く
- [x] `TcpReplicationTransport`（LAN plaintext）を実装
- [x] `SnapshotTransfer` を実装しテストを書く
- [x] `cargo test --workspace` がゼロエラーで通過する
- [x] AI_DEVELOPMENT_GUIDE.md §3（Dependency DAG）§11（Crate 別責務早見表）を更新する
- [x] architecture-review-server.md のファイルサイズ一覧を更新する（P7-1 テスト移動後の実数値に修正）

---

*提案: 2026-06-19。同日 accepted（人間承認済み）。*
