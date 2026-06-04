---
scope    : シミュレーションの時間モデルと 1 Tick 内の処理順序の完全仕様
audience : AI Agent / Human Developer
update   : Tick 処理順序が変わったとき / パフォーマンス目標が変わったとき
related  : event-catalog.md, ownership.md, CLAUDE.md §6
---

# Tick Model

## 1. Tick の定義

### Tick とは何か

```
Tick は論理的な時間単位である。
物理時刻（システムクロック）とは無関係。
単調増加する u64 の newtype。
```

Tick は「シミュレーションが何ステップ進んだか」を表す。
「現実時間で何ミリ秒経過したか」ではない。

### 物理時刻を使用することが禁止される理由

```
問題1: ノード間のクロックは必ずずれる（NTP の精度は数十ミリ秒）
問題2: NTP のステップ補正で時刻が逆行することがある
問題3: テスト環境と本番環境で同じ結果を再現できない
```

物理時刻を因果順序の判定に使うと非決定論的な結果が生じる。  
**`std::time::SystemTime` を Tick の代わりに使うことを禁止する（INV-005）。**

### Tick の比較可能範囲

```
現在: 単一 Node 内でのみ比較可能（全処理が同一プロセス内）
将来: Sector 間の因果順序は VectorClock で表現する（未実装）
```

---

## 2. Tick と物理時刻の関係

### 現在（Phase 0–1）: 制限なし

Tick ループは制限なく実行される（できるだけ速く）。  
1 Tick の処理時間はハードウェアとエンティティ数に依存する。

### 将来（Phase 2 以降）: 固定間隔への移行

```
目標間隔: 16 ms / Tick（60 Tick/秒）
実装方法: tokio::time::interval による非同期タイマー（未実装）
```

処理が 16 ms を超えた場合は Tick を遅延させる（スキップしない）。  
EVE Online の "Time Dilation" に相当する概念として将来検討する。

---

## 3. 1 Tick の処理順序（規範的定義）

**この順序は変更してはならない。** 変更には ADR が必要。

```
Step 1: Tick カウンタをインクリメント
         current_tick = current_tick + 1

Step 2: 未処理の Command を収集する
         （現在: MovementSystem が直接 Velocity を使用するため省略可）

Step 3: Movement System を実行する（ECS バッチ処理）
         MovementSystem::run(&mut world, &bounds, tick)

Step 4: 生成されたイベントを EventStore に Append する
         event_store.append_batch(events)

Step 5: （将来）Replication Actor に差分を通知する
         replication_tx.send(delta)  ← Phase 2 以降
```

### Step 4 より前に Step 5 を実行してはならない理由

EventStore への Append が完了する前に他のノードへ伝播すると、
受信側が「存在しないイベントを参照する」状態になる。  
**Append の完了 = Commit** であり、Commit 前のデータは存在しないものとして扱う。

---

## 4. Tick とイベントの対応規則

### ShipMoved への tick フィールド必須化

```rust
// 正しい: tick を含む
ShipMoved { ship_id, from, to, tick: Tick(42) }

// 禁止: tick を省略（INV-005 違反）
ShipMoved { ship_id, from, to }  // コンパイルエラーになる設計にする
```

`tick` フィールドを省略できない理由:  
tick なしでは Event の因果順序が不明になり、リプレイ時の順序保証ができない。

### 同一 Tick 内で同一 Ship が複数回移動した場合

```
現在の設計: 1 Tick につき Ship は 1 回だけ移動する
           （MovementSystem が 1 回だけ Velocity を適用する）

将来: Command キューを処理する場合、
      同一 Ship への複数 Command は次の Tick に持ち越す（未定）
```

---

## 5. Tick の単調性保証

### Tick は逆行しない

```
保証: tick.next() > tick は常に成立する
実装: u64 のオーバーフローは u64::MAX（約 1.8 × 10^19）Tick 後
      現実的な運用期間内でオーバーフローは発生しない
```

### ノード再起動後の Tick の扱い

```
現在（Phase 0–1）: プロセス終了で Tick がリセットされる
将来（Phase 3 以降）: Snapshot から Tick 値を復元し継続する
```

Snapshot 実装前は、再起動後の Tick 継続性は保証されない。  
これは現在のフェーズでは許容される。

---

## 6. パフォーマンス目標

### 目標値

| 指標 | 目標値 | 現在の計測状況 |
|---|---|---|
| 1 Tick 処理時間 (10,000 ships) | ≤ 16,000 µs | `cargo run --release` で計測 |
| P95 Tick 処理時間 | ≤ 12,000 µs | — |
| 最大 Tick 処理時間 | ≤ 16,000 µs | — |

### 計測対象の定義

```
計測開始: Tick カウンタのインクリメント直前
計測終了: EventStore::append_batch() の完了直後

Step 5（Replication）は計測対象外（未実装のため）
```

### ベンチマーク実行方法

```bash
cargo run -p dawn-simulation --bin simulate --release
```

---

## 7. Tick ループの実装責任

| フェーズ | 実装 | 実行モデル |
|---|---|---|
| Phase 0–1（現在） | `SimulationNode::tick()` | 同期・単純ループ |
| Phase 2（予定） | `SectorSimulatorActor` | 非同期・tokio task |
| Phase 3 以降（未定） | 固定間隔タイマー | tokio::time::interval |

`SimulationNode::tick()` は現在同期処理で実装されており、
呼び出し元がループ速度をコントロールする。  
非同期化は Phase 2 の Actor 導入時に対応する。
