---
scope    : 何を・どの順番で・なぜその順番で作るか。現在地と次のステップの明示
audience : AI Agent / Human Developer
update   : フェーズ完了時 / タスクが完了するたびに更新する
related  : architecture.md, CLAUDE.md §1
---

# Roadmap

## 1. このドキュメントの使い方

### 現在地の確認

「現在のフェーズ」セクションを見ること。  
次に着手すべきタスクは **1 つだけ** 太字で明記される。

### フェーズを飛ばしてはならない理由

各フェーズは次のフェーズの前提となる。  
例: Phase 1 の完了（単一ノードで 10,000 ships が動く）なしに
Phase 2（複数ノード）を実装すると、「動かない上に複雑」なコードになる。

### 完了基準の意味

完了基準は「感覚的に完成した」ではなく「このコマンドが成功する」で定義される。
曖昧な基準は採用しない。

---

## 2. 現在地

```
現在のフェーズ : Phase 0 — 基盤確立
フェーズの状態 : 進行中
```

### 完了済み

- [x] Cargo Workspace 初期化
- [x] `dawn-core`: EntityId / ShipId / NodeId / SectorId
- [x] `dawn-core`: Position / Velocity / SectorBounds / Tick
- [x] `dawn-core`: DomainEvent（ShipSpawned / ShipMoved / ShipDespawned）
- [x] `dawn-core`: MoveCommand
- [x] `dawn-event-store`: EventStore trait / InMemoryEventStore
- [x] `dawn-ecs`: SimWorld / MovementSystem
- [x] `dawn-simulation`: SimulationNode / Spawner / main benchmark
- [x] CLAUDE.md（AI 開発ガイド）
- [x] docs/（設計ドキュメント群）

### 次に着手すべきタスク

**Rust のインストールと `cargo test --workspace` の通過確認**

---

## 3. Phase 0 — 基盤確立

**完了基準:** `cargo test --workspace` がゼロエラーで通過する

| タスク | 状態 | 備考 |
|---|---|---|
| Cargo Workspace 初期化 | ✅ 完了 | |
| `dawn-core` 全型定義 + テスト | ✅ 完了 | 49 テスト |
| `dawn-event-store` InMemoryEventStore | ✅ 完了 | |
| `dawn-ecs` SimWorld + MovementSystem | ✅ 完了 | |
| `dawn-simulation` SimulationNode + Spawner | ✅ 完了 | |
| CLAUDE.md 初版 | ✅ 完了 | |
| docs/ 設計ドキュメント群 | ✅ 完了 | |
| Rust インストール + ビルド確認 | ⬜ 未完了 | |
| `cargo test --workspace` 通過 | ⬜ 未完了 | |

---

## 4. Phase 1 — Single Node シミュレーション検証

**完了基準:** 10,000 ships が 1 Tick を 16ms 以内に処理できることを計測で確認する

| タスク | 状態 | 備考 |
|---|---|---|
| `cargo run --release` でベンチマーク実行 | ⬜ 未完了 | |
| Tick 処理時間の計測と目標達成確認 | ⬜ 未完了 | 目標: ≤ 16,000 µs |
| P95 計測値の記録 | ⬜ 未完了 | |
| Event Log の増加ペース確認 | ⬜ 未完了 | |

### Phase 1 完了後に記録すること

```
- 計測した Tick 処理時間（min / mean / p95 / max）
- 1 Tick あたりの ShipMoved イベント数
- 使用したハードウェアスペック
```

---

## 5. Phase 2 — In-Memory Multi-Node

**完了基準:** 3 つの論理ノードが In-Memory Channel 経由でイベントを同期し、
全ノードのイベントログが一致することをテストで確認する

| タスク | 状態 | 依存 |
|---|---|---|
| `dawn-actor` クレート作成（Actor 基盤） | ⬜ 未着手 | Phase 1 完了後 |
| `SectorSimulatorActor` 実装 | ⬜ 未着手 | |
| `EventStoreActor` 実装 | ⬜ 未着手 | |
| ノード間 In-Memory Channel 接続 | ⬜ 未着手 | |
| 3 ノード整合性テスト | ⬜ 未着手 | |

### Phase 2 の制約（変えない）

```
通信: In-Memory Channel（tokio::mpsc）のみ
ネットワーク: 引き続き不使用
ノード数: 3 固定
```

---

## 6. Phase 3 — Event 永続化

**完了基準:** ノードを再起動した後、Snapshot + Event Replay によって
シャットダウン直前の Ship 状態が完全に復元される

| タスク | 状態 | 依存 |
|---|---|---|
| ファイルベース EventStore 実装 | ⬜ 未着手 | Phase 2 完了後 |
| Snapshot 取得ロジック | ⬜ 未着手 | |
| Snapshot からの State 復元 | ⬜ 未着手 | |
| 再起動後の整合性テスト | ⬜ 未着手 | |

---

## 7. Phase 4 以降（方向性のみ）

詳細設計は Phase 3 完了後に行う。現時点では方向性のみ記録する。

```
Phase 4: ネットワーク層（gRPC / QUIC）
          In-Memory Channel を gRPC に置き換える
          trait による抽象化で dawn-actor への変更を最小化する

Phase 5: 分散コンセンサス（Raft）
          Sector Transit の整合性保証
          Leader 選出 / Log Replication

Phase 6: CRDT による位置の最終一貫性
          Sector-local Move を Raft から分離
          LWW-Register によるスループット向上
```

---

## 8. 廃止・変更された計画の記録

変更があった場合のみ追記する。

現時点での変更履歴: **なし**
