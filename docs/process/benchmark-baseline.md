---
scope    : Phase 7 着手前のパフォーマンス基準値の記録
audience : AI Agent / Human Developer
update   : フェーズ完了時にベンチマークを再実行して追記する
related  : ../architecture/tick-model.md, roadmap.md
---

# Benchmark Baseline

Raft（Phase 7）等の導入によるオーバーヘッドを定量比較するための基準値。
計測コマンド: `cargo run -p dawn-simulation --bin simulate --release`

## Phase 6 完了時点（2026-06-11, commit 450aa8a）

計測環境: Windows 11 Pro / 開発機（絶対値は環境依存。比較は同一環境で行うこと）

### Phase 1 benchmark — single node, 10,000 ships, 100 ticks

| 指標 | 値 |
|---|---|
| spawn | 10,000 ships / 12.4 ms |
| tick min | 83,316 µs |
| tick mean | 87,485 µs |
| tick p95 | 92,506 µs |
| tick max | 96,823 µs |
| SLA (≤16,000 µs) | ✗ FAIL |

注: 実行ごとのばらつきが大きい（同日別実行で mean 131,851 µs を記録）。
比較時は複数回実行して傾向を見ること。

### Phase 2 demo — 3 nodes × 1,000 ships, 20 ticks

| 指標 | 値 |
|---|---|
| spawn | 3,000 ships / 65 ms |
| 20 ticks × 3 nodes | 73 ms |
| replicated | 3,000 events |

### Phase 3 demo — persistence round-trip, 100 ships, 10 ticks

snapshot 保存・復元・イベント再生のラウンドトリップが完走することを確認。

## Phase 7（ADR-0014 Raft Consensus）

計測コマンド: `cargo test -p dawn-simulation --release transit_latency_benchmark -- --ignored --nocapture`

### Sector Transit レイテンシ（ローカル ECS 操作のみ）

| 指標 | 値 |
|---|---|
| propose_transit + export_transit + import_transit (avg, 1,000 iterations) | 10.664 µs |

注: この値は `SimulationNode` 上の ECS 操作（TransitState 設定 /
Snapshot 抽出 / ECS への復元）のみのコストであり、Raft Log への
Proposal がコミットされるまでのレイテンシ（election/heartbeat の
Tick 駆動タイマー、INV-005）は含まれない。Raft 経由のコミットレイテンシは
`heartbeat_interval`（cluster.rs では 3 ticks）のオーダーで決まり、
ECS 操作コスト自体は無視できるほど小さい。

## 既知の問題（Phase 6 時点・基準値とは別件）

1. **Phase 1 SLA FAIL**: 10,000 隻で mean 87 ms（目標 16 ms）。
   Phase 6 で Capacitor / Lock / Combat / Bot システムが追加され
   Tick が重くなったため。10,000 隻の SLA 達成は Phase 8（Anti-TiDi /
   Spatial Index）のスコープであり、現時点では未達で想定どおり。

Phase 2 / Phase 3 の判定ロジックの陳腐化（ADR-0008 以前の毎 Tick
イベント前提）は commit 4408ca8 で修正済み。両方とも PASS する。
