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

### ベンチマーク実行（`simulate` バイナリ）: 制限なし

Tick ループは制限なく実行される（できるだけ速く）。  
1 Tick の処理時間はハードウェアとエンティティ数に依存する。

### サーバー実行（`--serve`）: 固定間隔

```
現在の目標間隔: 100 ms / Tick（10 Tick/秒）
実装方法: tokio::time::interval による非同期タイマー（WsServer モードで動作中）
将来の目標  : 16 ms / Tick（62.5 Tick/秒）— Phase 8 以降で再検討
```

処理が 100 ms を超えた場合はシステム異常として記録する。
論理 Tick の単調性・決定性は常に保つ（イベントの並べ替え・欠落・スキップは不可）。
→ 過負荷は「分割 → LoD → 局所 TiDi → 入場制限」の順で対処する（ADR-0018）。
局所 TiDi は論理 Tick の決定性を壊さず実時間ペースのみを落とす最終手段。
詳細は §8 を参照。

---

## 3. 1 Tick の処理順序（規範的定義）

**この順序は変更してはならない。** 変更には ADR が必要。

```
現在の実装（Phase 7 — Raft タイマー駆動 Step 10 追加済み・ADR-0014）:

Step 1: Tick カウンタをインクリメント
         current_tick = current_tick + 1

Step 2: コマンドキューを処理する
         MoveCommand              → ThrustComp.direction を更新（is_braking = false）
         StopCommand              → ThrustComp.is_braking = true（逆推力で減速）
         LockOnCommand            → LockSystem に渡す（次のステップで処理）
         ActivateModuleCommand    → FittedSlot.is_active = true / apply_fitting()
         DeactivateModuleCommand  → FittedSlot.is_active = false / apply_fitting()
         JumpCommand              → can_propose_jump() 検証後、TransitOp::Request
                                    （gate_id 付き）を Raft に提案（ADR-0009）
         ApproachCommand          → ApproachComp を付与（対象 Ship / Jump Gate へ
                                    半自動接近・Move / Stop で解除・ADR-0015）
         WarpCommand              → can_propose_warp() 検証後 WarpComp を付与
                                    （intra-Sector 短距離 Fold = ワープ・ADR-0022。
                                    Move / Stop は align 中のみ解除・warping は無視）
         ※ Transit 中（TransitState::InTransit）の Ship への Move / Stop /
           二重 Transit / Jump / Approach / Warp は拒否する（ADR-0014 / CLAUDE.md §5）

Step 2.5: Approach System を実行する（Movement の前・ADR-0015）
         SimulationNode::process_approach()
         → ApproachComp を持つ Ship のみ対象。対象（Ship / Jump Gate）の位置へ
           thrust を向け直し、到着半径まで詰めたら is_braking = true で停止保持。
           Ship 対象が消失したら ApproachComp を除去して is_braking = true。
         → 生成イベントなし（次 Tick 以降の Movement が VelocityChanged を出す）

Step 2.6: Warp System を実行する（Approach の後・Movement の前・ADR-0022 / ADR-0025）
         SimulationNode::process_warp(tick)
         → WarpComp を持つ Ship のみ対象。Aligning はターゲット方向へ加速し、ターゲット方向の速度が
           max_speed × 75% に達したら Warping へ遷移（EVE 準拠・整列時間は機動性次第・中断可・Tackle 窓）。
           Warping はターゲットへ直進し残距離比例で減速して以下の地点で停止:
             Gate  ターゲット: activation_radius × 0.8 以内（ADR-0022）
             Body  ターゲット: body.radius × 1.5 以内（ADR-0025 BODY_WARP_ARRIVAL_FACTOR）
           到達不能時（ターゲット消失等）は WarpComp を除去してブレーキ。
           auto_jump = true の Gate ターゲットは到着後に pending_auto_jumps へ push（ADR-0023）。
         → warping 中の船は Step 3 の Movement がスキップ（warp 速度をクランプしない）。
           生成イベント: VelocityChanged（warp の移動を記録・新イベント型なし）

Step 3: Movement System を実行する（ECS バッチ処理・warping 中の船はスキップ）
         MovementSystem::run(&mut world, tick)
         → 生成: Vec<VelocityChanged>（速度が変化した船のみ）

Step 4: Capacitor System を実行する
         CapacitorSystem::run(&mut world, tick)
         → 毎 Tick: cap を cap_recharge_per_tick 分回復（cap_max でクランプ）
         → サイクル開始時（cycle_remaining == 0）: cap_cost_per_cycle を消費し
           cycle_remaining = cycle_time_ticks をセット
         → 残りTick時: cycle_remaining を 1 デクリメント
         → cap 不足でサイクル開始できない場合: モジュールを強制 OFF
         → 生成: Vec<ModuleDeactivated>（cap 枯渇による強制 OFF）
         ※ Movement の後、Lock の前に実行すること

Step 4.5: Tackle System を実行する（Capacitor の後・Lock の前・ADR-0024）
         SimulationNode::process_tackle(tick)
         → アクティブな Tackle モジュール（ModuleKind::Tackle、cap ON）を持つ Ship のみ対象。
           ロック済みターゲットが tackle_range 以内にいれば TackledComp に tackler を追加。
           射程外・ロック消失・tackler 破壊の場合は tackler を除去して TackleReleased を発行。
           TackledComp を持つ Ship は can_propose_warp / can_propose_jump が false を返す。
         → 生成: Vec<TackleApplied | TackleReleased>

Step 5: Lock System を実行する
         LockSystem::run(&mut world, tick, &lock_commands)
         → 生成: Vec<TargetLocked | LockLost>
         ※ Movement の後に実行すること（位置確定後にロック判定）

Step 6: Combat System を実行する
         CombatSystem::run(&mut world, tick, &cap.weapon_cycles_started)
         → weapon_cycles_started に含まれる Ship のみ発射判定する（ADR-0012）
         → EVE 命中率式: hit_chance = 0.5^((angular/(tracking×sig))² + (max(0,d−opt)/falloff)²)
         → 生成: Vec<WeaponFired | DamageTaken | ShipDestroyed>
         ※ Lock System の後に実行すること（Locked 状態を参照するため）
         ※ 破壊された Ship は呼び出し元が ECS と ship_index から削除する

Step 7: Bot System を実行する（IsBotComp を持つ Ship のみ）
         SimulationNode::process_bots()
         → Bot が人間プレイヤーと同一のコマンドパイプラインでコマンドを生成・実行
         → 生成イベントなし（コマンド実行の結果は次 Tick 以降のシステムで生成）
         ※ Combat の後に実行すること（破壊判定が終わってから Bot AI を実行）
         ※ Bot コマンドは apply_*_owned() メソッドを通じてプレイヤーと同一のパイプラインを使う

Step 8: 全イベントを EventStore に Append する
         event_store.append_batch(move_events + cap_events + lock_events + combat_events)

Step 9: Replication Actor に差分を通知する
         replication_tx.send(delta)

Step 10: RaftActor に TickElapsed を送る（ADR-0014）
         raft.tick()
         → election timeout / heartbeat タイマーを 1 Tick 進める（INV-005 / FBD-003）
         ※ serve は `transit::step_cluster_node`（7.5 apply → node.tick → raft.tick）
           が `run_cluster_server` 内で実行。actor 経路（テスト/デモ）は
           SectorSimulatorActor の Tick ハンドラで Step 9 flush を挟んで実行する。

Step 7.5: コミット済み Raft エントリを適用する（ADR-0014 §7）
         transit::apply_committed_raft_entries()（serve と actor で共有）
         → コミット済み TransitOp を ECS に適用する:
           TransitOp::Request → 所有ノード: InTransit 化 + SectorTransitRequested を
             Append、Ship 状態を export して TransitOp::Commit を Raft に提案
           TransitOp::Commit  → 宛先ノード: entry_pos に import + SectorTransitCompleted
             gate_id が Some の場合はさらに JumpGateUsed を Append し、
             from/to の StarSystemId が異なる場合は StarSystemChanged も
             Append する（ADR-0009 / SimulationNode::append_jump_events）
         ※ node.tick（Step 1）の前に実行する。actor 経路では生成イベントを
           同 Tick の flush で ReplicationBus に伝播する。
```

### Step 8 より前に Step 9 を実行してはならない理由

EventStore への Append が完了する前に他のノードへ伝播すると、
受信側が「存在しないイベントを参照する」状態になる。  
**Append の完了 = Commit** であり、Commit 前のデータは存在しないものとして扱う。

---

## 4. Tick とイベントの対応規則

### tick フィールドの必須化

すべてのドメインイベントは `tick: Tick` フィールドを含む（INV-005）。

```rust
// 正しい: tick を含む
VelocityChanged { ship_id, velocity, tick: Tick(42) }

// 禁止: tick を省略（INV-005 違反）
VelocityChanged { ship_id, velocity }  // コンパイルエラーになる設計にする
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

`StateSnapshot` が `tick` を保持し、`SimulationNode::restore_from` が
復元する（Phase 3 で実装済み）。再起動後も Tick は継続する。

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

Step 9（Replication）は計測対象外（非同期伝播のため）
```

### ベンチマーク実行方法

```bash
cargo run -p dawn-simulation --bin simulate --release
```

---

## 7. Tick ループの実装責任

| フェーズ | 実装 | 実行モデル |
|---|---|---|
| Phase 0–1 | `SimulationNode::tick()` | 同期・単純ループ（ベンチマーク用） |
| Phase 2 | `SectorSimulatorActor` | 非同期・tokio task |
| Phase 4 以降（現在） | `run_phase4_server()`（単一ノード）/ `run_cluster_server()`（3ノード Raft・Phase 7.5）in `main.rs` | tokio::time::interval（100ms/tick） |

Phase 4 以降は `tokio::time::interval` による固定間隔ループが実装済み。  
`SimulationNode::tick_with_lock_commands()` は同期処理で、呼び出し元の interval が速度をコントロールする。

---

## 8. 負荷制御設計（Anti-TiDi 優先・TiDi は局所的最終手段）

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

### このシステムの方針：TiDi を「稀」にし、出ても局所・短時間に抑える（ADR-0018）

旧方針は「TiDi を一切発生させない（入場制限で事前規制）」だった。
しかし **単一密戦闘は原理的に分割不能** で、その場合の手段が入場制限のみになると
「クライマックスの戦いから締め出す」＝ EVE の TiDi より悪い体験になりうる（eve-reference §11.1）。

ADR-0018 で方針を改める。**過負荷は負荷の性質に応じた劣化ヒエラルキーで対処する。**

| 状況 | 第1手 | 第2手 | 最終バックストップ |
|---|---|---|---|
| 空間的に分離可能 | 動的 Sector 分割（劣化ゼロ） | — | — |
| 単一密戦闘 | LoD（遠方/非戦闘の更新間引き） | 局所 TiDi（全員残る） | 入場制限 |

EVE との差別化は「TiDi が無い」ではなく、**TiDi 閾値が桁違いに高い
（Rust + マルチコア + 空間索引/AoI）/ 出ても当該 Sector 局所 / 短時間 / 自動回復 / 観測可能** であること。

### Sector Population Cap（入場制限 = 最終バックストップ）

各 Sector はエンティティ数の上限（`population_cap`）を持つ。
ADR-0018 以降、入場制限は主要手段ではなく **最終バックストップ** に位置づけが下がる
（単一密戦闘では LoD・局所 TiDi を先に試し、それでも遅延が許容域を超える極端時のみ入場を絞る）。

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

### Local Time Dilation（局所 TiDi = 単一密戦闘の安全網）

分割不能な単一ホットスポットがノード容量を超えたら、当該 Sector に限り TiDi を発動する。
INV-TiDi の 5 条件（局所 / 観測可能 / 非破壊 / 自動回復 / 分割・LoD の後）を満たすこと。

```
dilation の決定（実時間ペーシングのみ。論理 Tick の処理内容は不変）:
  if sector.tick_cost > sector.budget && !sector.splittable() {
      sector.dilation = (sector.budget / sector.tick_cost).max(MIN_DILATION);
      metrics.tidi_active.set(sector.id, sector.dilation);   // 観測可能性
  } else if sector.dilation < 1.0 && sector.tick_cost <= sector.budget {
      sector.dilation = 1.0;                                  // 自動回復
  }
```

TiDi は論理 Tick の決定性を壊さない（INV-005 と無関係）。
イベントの並べ替え・欠落・結果変化を起こしてはならない（純粋な実時間ペーシング）。

### Tick SLA の監視と対処ヒエラルキー

Tick 処理時間が目標を超えた場合、黙って遅らせず、記録したうえでヒエラルキーで対処する。

```
Tick 処理時間 ≤ 12ms : 正常
Tick 処理時間 ≤ 32ms : 警告（warn! ログ）
Tick 処理時間 > 32ms : 記録（error! ログ + メトリクス）し、順に対処:
                       1. 分割可能か？   → 可能なら動的分割（劣化ゼロ）
                       2. 単一密戦闘か？ → LoD → 局所 TiDi（観測可能に発動）
                       3. それでも遅延が許容域外の極端時 → 入場制限（最終バックストップ）
```

「黙って Tick を遅らせる」ことは禁止 — dilation は常に観測可能でなければならない。
これが EVE のグローバル TiDi と異なる点である（dawn は局所・観測可能・自動回復）。

### 設計上の不変条件（INV-TiDi 改訂・ADR-0018）

```
INV-TiDi: 論理 Tick 速度は通常一定。
          Time Dilation は分割不能な単一ホットスポット超過時に限り、
          (a) 局所 (b) 観測可能 (c) 非破壊 (d) 自動回復 (e) 分割/LoD の後
          を満たす境界つき最終手段としてのみ許可する。
          （正規定義は CLAUDE.md §2 INV-TiDi）
```
