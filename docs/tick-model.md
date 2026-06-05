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

処理が 16 ms を超えた場合はシステム異常として記録する。
Tick を遅延させることも、スキップすることも許容しない。
→ 負荷超過は「Tick を遅らせる」のではなく「Sector への入場を制限する」ことで解決する。
詳細は §8 を参照。

---

## 3. 1 Tick の処理順序（規範的定義）

**この順序は変更してはならない。** 変更には ADR が必要。

```
現在の実装（Phase 4 Cycle 3）:

Step 1: Tick カウンタをインクリメント
         current_tick = current_tick + 1

Step 2: コマンドキューを処理する
         MoveCommand   → ThrustComp を更新
         LockOnCommand → LockSystem に渡す（次のステップで処理）

Step 3: Movement System を実行する（ECS バッチ処理）
         MovementSystem::run(&mut world, tick)
         → 生成: Vec<VelocityChanged>（速度が変化した船のみ）
         ※ ShipMoved は @deprecated（ADR-0008）。位置は派生状態であり記録しない。

Step 4: Lock System を実行する
         LockSystem::run(&mut world, tick, &lock_commands)
         → 生成: Vec<TargetLocked | LockLost>
         ※ Movement の後に実行すること（位置確定後にロック判定）

Step 5: Combat System を実行する
         CombatSystem::run(&mut world, tick)
         → 生成: Vec<WeaponFired | DamageTaken | ShipDestroyed>
         ※ Lock System の後に実行すること（Locked 状態を参照するため）
         ※ 破壊された Ship は呼び出し元が ECS と ship_index から削除する

Step 6: 全イベントを EventStore に Append する
         event_store.append_batch(move_events + lock_events + combat_events)

Step 7: Replication Actor に差分を通知する
         replication_tx.send(delta)
```

### Step 6 より前に Step 7 を実行してはならない理由

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

## 8. 負荷制御設計（Anti-TiDi）

### EVE Online の TiDi とその問題

EVE Online は Sector（ソーラーシステム）の負荷が高くなると
**Time Dilation（TiDi）** を発動し、シミュレーション速度を最大 10% まで低下させる。

```
EVE の TiDi:
  通常: 1秒 = 1秒
  TiDi: 1秒 = 10秒（10倍スロー）
  効果: 処理が追いつかなくても「世界の時間を遅らせる」ことで整合性を維持
  問題: プレイヤー体験が著しく悪化する（操作が効かない・戦闘が長時間化）
        コミュニティから長年にわたり不評
```

### このシステムの方針：TiDi を発生させない

TiDi は「過負荷になった後の救済措置」である。
このシステムは **過負荷になる前に Sector への入場を制限する**ことで
TiDi を構造的に発生させない。

```
EVE（事後対処）:  負荷超過 → TiDi で時間を遅らせる
Dawn（事前規制）: 入場制限 → Sector は常に定員内 → Tick は常に 16ms 以内
```

### Sector Population Cap（入場制限）

各 Sector はエンティティ数の上限（`population_cap`）を持つ。

```
population_cap : Sector が受け入れる Ship の最大数
警告閾値       : population_cap × 0.8（80%）到達でアラート
制限閾値       : population_cap × 0.95（95%）到達で SpawnCommand を拒否
```

**SpawnCommand のアドミッションコントロール:**

```
SpawnCommand 受信
    │
    ▼
Sector の現在人口を確認
    │
    ├─ population < 制限閾値 → 通常処理
    │
    └─ population ≥ 制限閾値 → SpawnRejected { reason: SectorAtCapacity }
                               （隣接 Sector への誘導情報を含める）
```

SpawnRejected はドメインイベントとして EventLog に記録する。
「なぜその Sector が満員になったか」の履歴が残る。

### Dynamic Sector Fission（動的分割）

population_cap の 80% を超えたタイミングで Sector の分割を準備する。
負荷が閾値を超える「前」に分割を開始することが重要。

```
[Sector A: 4,000/5,000 ships]  ← 80% アラート
         │
         │ Sector Fission 開始
         ▼
[Sector A1: 2,000 ships] + [Sector A2: 2,000 ships]
```

分割戦略：空間的中央分割（X 軸または Y 軸の中点で二分）。
→ SectorTransit の設計と密接に関連する（ownership.md 参照）。

### Tick SLA の強制

Tick 処理時間が目標を超えた場合は「TiDi を発動する」のではなく
「システム異常として記録し、アラートを発する」。

```
Tick 処理時間 ≤ 12ms : 正常
Tick 処理時間 ≤ 16ms : 警告（warn! ログ）
Tick 処理時間 > 16ms : 異常（error! ログ + メトリクス）
                       → 根本原因の調査が必要
                       → population_cap の見直しをトリガーする
```

Tick SLA 超過は「許容された動作」ではなく「修正が必要なバグ」として扱う。
これが EVE の TiDi と根本的に異なる設計思想である。

### 設計上の不変条件（追加）

```
INV-TIDI: Tick の論理速度は一定である。
          「Tick を遅らせる」実装を追加してはならない。
          負荷超過は Sector への入場制限で対処する。
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
