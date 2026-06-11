---
scope    : Phase 7 着手前のパフォーマンス基準値の記録
audience : AI Agent / Human Developer
update   : フェーズ完了時にベンチマークを再実行して追記する
related  : tick-model.md, roadmap.md
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

## 既知の問題（Phase 6 時点・基準値とは別件）

ベンチマークバイナリの判定ロジック自体に陳腐化があり、以下の FAIL は
基準値の異常ではなく判定側の問題である（変更前コミット 2be8491 でも同様に FAIL）。

1. **Phase 1 SLA FAIL**: 10,000 隻で mean 87 ms（目標 16 ms）。
   Phase 6 で Capacitor / Lock / Combat / Bot システムが追加され
   Tick が重くなったため。10,000 隻の SLA 達成は Phase 8（Anti-TiDi /
   Spatial Index）のスコープであり、現時点では未達で想定どおり。
2. **Phase 2 consistency FAIL**: 期待値計算（expected 63,000）が
   現在のイベント発行モデル（ADR-0008: VelocityChanged のみ）に
   追従しておらず、毎 Tick イベント前提の古い式のまま。
3. **Phase 3 tick/position FAIL**: デモは等速 NPC のみを使うため
   snapshot 以降にイベントが発行されず（ADR-0008 設計どおり）、
   restore 後に残り Tick を回さない判定ロジックでは一致しない。
   正しい検証は node.rs の INV-002 テスト
   `ecs_state_is_fully_restored_from_snapshot_and_event_replay_after_simulated_restart`
   が担っている。

→ 2 と 3 は判定ロジックの修正候補（別タスク）。
