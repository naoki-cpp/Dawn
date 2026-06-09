# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

# CLAUDE.md — dawn プロジェクト AI開発ガイド

このファイルはAIエージェントが本プロジェクトを安全に継続開発するための
**唯一の権威ある運用規約**である。

コードを書く前に必ずこのファイルを読むこと。
設計判断の根拠は `docs/adr/` を参照すること。

---

## 0. 開発コマンド早見表

```bash
# ビルド
cargo build --workspace
cargo build --workspace --release

# テスト
cargo test --workspace                        # 全テスト
cargo test -p dawn-core                       # 特定クレートのみ
cargo test ship_moved_event                   # テスト名フィルタ

# カバレッジ（要: cargo install cargo-llvm-cov）
cargo llvm-cov --workspace --html

# ベンチマーク
cargo bench -p dawn-simulation

# 依存チェック（循環・禁止依存の検出）
cargo tree --duplicates
# cargo deny check bans  # 要: cargo install cargo-deny

# シミュレーション実行
cargo run -p dawn-simulation --bin simulate                          # Phase 1-3 benchmark
cargo run -p dawn-simulation --bin simulate --release -- --serve     # Phase 5 WebSocket server (Godot用)
cargo run -p dawn-simulation --bin simulate --release -- --serve --ships 50  # 船数指定
```

**WebSocket サーバー起動後の接続先**: `ws://127.0.0.1:7878`

# ゲームバランス調整（リビルド不要）
# data/ ディレクトリの TOML を編集してサーバーを再起動するだけでよい
# ファイルが見つからない場合は ship_types.rs / modules.rs のデフォルト値を使用
data/ship_types.toml   # 船種定義（HP・速度・スロット数など）
data/modules.toml      # モジュール定義（ダメージ・射程・StatDelta など）

# コミット
# → 規約: docs/commit-convention.md を参照すること（英語・Conventional Commits 準拠）
# 例:
#   feat(dawn-ecs): add CapacitorSystem with cycle-based cap drain
#   fix(godot): correct cap bar percentage calculation
#   docs(adr): update ADR-0006 checklist to reflect Phase 6 completion

---

---

## 目次

1. [プロジェクト本質の理解](#1-プロジェクト本質の理解)
2. [Architecture Invariants](#2-architecture-invariants)
3. [Dependency DAG](#3-dependency-dag)
4. [Event Workflow](#4-event-workflow)
5. [Entity Ownership Rules](#5-entity-ownership-rules)
6. [Tick Model](#6-tick-model)
7. [Event Schema Evolution Rules](#7-event-schema-evolution-rules)
8. [Testing Rules](#8-testing-rules)
9. [AI Change Checklist](#9-ai-change-checklist)
10. [Forbidden Changes](#10-forbidden-changes)
11. [Crate別責務早見表](#11-crate別責務早見表)
12. [よくある設計違反パターン](#12-よくある設計違反パターン)

---

## 1. プロジェクト本質の理解

### このシステムが解く問題

> **数万エンティティを3ノードの分散構成でリアルタイム同期する技術基盤**

ゲームを作っているのではない。以下を実証する研究基盤である。

- Single Shardの分散シミュレーション
- イベントソーシングによる完全な因果追跡
- CRDTとRaftの責務分離による高スループット同期

### 現在のスコープ（Phase 6 完了時点）

```
実装対象:
  エンティティ  : Ship のみ
  コンポーネント: Position(x, y, z), Velocity, ThrustComp, ShipStatsComp,
                  HullComp（Shield/Armor/Hull 3層）, FittingComp（装備スロット）,
                  CapacitorComp（現在 cap 量）
  船種          : ShipTypeDefinition（id, name, class, base_stats, slot_layout）
  イベント      : ShipSpawned（ship_type_id 含む）, VelocityChanged, SectorTransit系,
                  ShipFitted, WeaponFired, DamageTaken（3層 HP）, ShipDestroyed,
                  ModuleActivated, ModuleDeactivated
  ノード構成    : 3ノード固定

Phase 4 以降で追加承認済み（全て実装済み）:
  Fitting システム（EVE Online 準拠・Active/Passive モジュール）
  Combat システム（武器 / ダメージ / HP 3層 / 破壊）
  Lock-on システム（2フェーズ戦闘）
  ShipType システム（船種・船クラス・スロットレイアウト）
  Capacitor システム（サイクルベース cap 管理・強制 OFF）

実装しない（提案も拒否する）:
  課金 / キャラクター育成 / 市場 / チャット
  グラフィックス専用エンジン / 物理エンジン外部依存
```

スコープ外の機能追加を求められた場合、実装せずにその旨を伝えること。
スコープの拡張が必要な場合は ADR を作成して人間の承認を得ること。

### 絶対に変えてはならない設計原則

```
原則1: Event が唯一の真実。State は派生物に過ぎない。
原則2: Event は追記のみ。既存のEventを変更・削除しない。
原則3: 因果順序は論理Tick + NodeIdで決定する。物理時刻を使わない。
原則4: Crate依存は一方向のみ。循環依存は設計の失敗を意味する。
原則5: Actor間の通信はMailbox経由のみ。直接メソッド呼び出し禁止。
原則6: Tickの論理速度は一定である。負荷超過はSector入場制限で対処し、Tickを遅らせない。
```

---

## 2. Architecture Invariants

以下はコードレビューで必ず検証する不変条件である。
**これらを破るコードは、動作していても必ずリジェクトする。**

### INV-001: Event Log は Append-only

```
違反例:
  event_store.update(event_id, new_payload)  // 既存Eventの上書き
  event_store.delete(event_id)               // Eventの削除
  log.truncate(index)                        // コミット済みLogの切り捨て

許容される唯一の操作:
  event_store.append(event)
```

理由: 過去のEventを変更できると世界の再現性が破壊される。
バグ修正は新しいEventを追加することで表現する。

### INV-002: StateはEventのReplayで完全に再現できなければならない

```
検証方法:
  1. ノードをシャットダウンする
  2. In-Memory状態を破棄する
  3. Event LogのみからStateを再構築する
  4. シャットダウン直前の状態と一致することを確認する

これが成立しない実装は INV-002 違反である。
```

具体的な違反:
- Stateのみに存在する情報（Event Logに対応するEventがない）
- EventのPayloadに後から追加されたフィールドがReplay時にデフォルト値になる

### INV-003: Sector-local操作はSector境界を越えない

```
違反例:
  // SectorAのActorがSectorBのEntityを直接操作する
  sector_b_actor.move_ship(ship_id, new_pos)

正しい実装:
  // SectorTransitイベントを経由する
  event_store.append(SectorTransitRequested { ship_id, from: A, to: B })
```

理由: Sector境界を越える操作がRaftを経由しないと整合性が壊れる。

### INV-004: EntityIdは世界全体で一意かつ再利用不可

```
違反例:
  // ShipDespawn後に同じIDを再割り当てする
  let id = recycled_ids.pop().unwrap()

正しい実装:
  // 単調増加するカウンタ + NodeIdの組み合わせ
  let id = EntityId::new(node_id, global_counter.fetch_add(1))
```

理由: 再利用されたIDはEvent Logのリプレイで「Despawn済みのShipが再びSpawnする」
という矛盾を引き起こす。

### INV-005: Tickは単調増加する論理カウンタである

```
違反例:
  use std::time::SystemTime;
  let tick = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();

正しい実装:
  let tick = self.logical_tick.fetch_add(1, Ordering::SeqCst);
```

理由: 物理時刻はノード間でずれる。NTPのステップ補正により時刻が逆行する
可能性もある。因果順序の判定に物理時刻を使うと非決定論的な結果になる。

### INV-006: CommandとEventを混在させない

```
違反例:
  // CommandとEventが同じ型で表現されている
  enum Message {
      MoveShip { ship_id, target },    // これはCommand
      ShipMoved { ship_id, from, to }, // これはEvent
  }

正しい実装:
  // commands.rs と events.rs を完全に分離する
  mod commands { pub struct MoveCommand { pub ship_id: ShipId, pub target: Position } }
  mod events   { pub struct ShipMoved   { pub ship_id: ShipId, pub from: Position, pub to: Position } }
```

理由: Commandは拒否できる。Eventは既に起きた事実で拒否できない。
同じ型で表現すると「まだ起きていないこと」と「起きたこと」の区別が失われる。

### INV-MOVE: 移動イベントは速度の変化のみを記録する（ADR-0008）

```
違反例:
  // 毎 Tick、位置を記録する
  event_store.append(ShipMoved { from, to, tick });

  // 物理入力を記録する
  event_store.append(ThrustApplied { direction, tick });

正しい実装:
  // 速度が変化したときのみ記録する
  if new_velocity != old_velocity {
      event_store.append(VelocityChanged { ship_id, velocity: new_velocity, tick });
  }
```

理由:
  - 位置（Position）は派生状態である。イベントに含めない。
  - 物理入力（推力）はコマンドに相当する。イベントに含めない。
  - Replay は物理シミュレーションを必要としてはならない。
  - 物理ロジックが将来変わっても、過去の VelocityChanged は正確に Replay できる。
  - `position += velocity` は純粋な算術であり、物理ロジックではない。

### INV-TIDI: Tickの速度は常に一定である

```
違反例:
  // 負荷が高いのでTickを遅らせる
  if tick_elapsed > TARGET_MS {
      self.time_dilation_factor = 0.1; // 10倍スロー
  }

  // Tickを遅延させて「待つ」
  tokio::time::sleep(extra_delay).await;

正しい実装:
  // 負荷が高い → Sector入場を制限する（SpawnRejected）
  if sector.population >= sector.population_cap * 0.95 {
      return Err(CommandError::SectorAtCapacity);
  }

  // Tick SLA超過は「許容された動作」ではなく「異常」として記録する
  if elapsed > TICK_BUDGET {
      error!("Tick SLA violated: {}µs > {}µs", elapsed, TICK_BUDGET);
      metrics.tick_sla_violation.inc();
  }
```

理由: EVE OnlineのTime Dilation（TiDi）はプレイヤー体験を著しく悪化させる。
負荷超過はTickを遅らせるのではなく、Sectorへの入場制限と動的分割で事前に対処する。
→ 詳細設計は docs/tick-model.md §8 を参照。

---

## 3. Dependency DAG

### 許可された依存方向

```
dawn-core
    ↑ (依存してよい)
    ├── dawn-ecs
    └── dawn-event-store
            ↑
            └── dawn-actor          ← Actor基盤（EventStoreActor, ReplicationBus）
                    ↑
                    └── dawn-simulation  ← 実行バイナリ・負荷生成

# 将来追加予定（まだ存在しない）:
#   dawn-actor ← dawn-consensus  （Raft）
#   dawn-actor ← dawn-replication（Gossip + CRDT）
#   上記 ← dawn-sector-node      （本番実行バイナリ）
```

### 依存の絶対ルール

**上位層から下位層への依存は禁止する（矢印の逆方向）**

```toml
# 禁止: dawn-core が dawn-ecs に依存する
# Cargo.toml (dawn-core)
[dependencies]
dawn-ecs = { path = "../dawn-ecs" }  # ← これは絶対に書いてはならない
```

**`dawn-core` が依存してよいクレートの完全なリスト**

```toml
# dawn-core/Cargo.toml の [dependencies] に書いてよいもの
serde       = { version = "1", features = ["derive"] }
uuid        = { version = "1", features = ["v4"] }
thiserror   = "1"
# 以上。ネットワーク・ファイルI/O・非同期ランタイムは禁止。
```

### 依存違反の検出

```bash
# CIで実行する。失敗したら依存を修正すること。
cargo deny check bans

# 循環依存の検出
cargo tree --duplicates
```

### Proto クレートの特別ルール（将来・未実装）

`dawn-proto` は将来 Phase 5 で追加される予定。
全クレートから依存されてよい（シリアライゼーション定義のみを含む）。
ただし `dawn-proto` からドメインロジックへの依存は禁止する。

---

## 4. Event Workflow

### Commandからイベント発行までの正規フロー

```
外部入力 (または Simulation)
    │
    ▼
[1] Command 受信
    │  例: MoveCommand { ship_id, target_position }
    │
    ▼
[2] Command Validation（バリデーション）
    │  - ship_id が存在するか
    │  - target_position が Sector 境界内か
    │  - 速度制限を超えていないか
    │  失敗 → CommandRejected を返す（Eventは発行しない）
    │
    ▼
[3] Domain Logic（ドメインロジック実行）
    │  - 新しい Position を計算する
    │  - ECS World を更新する（メモリ内のみ）
    │
    ▼
[4] Event 生成
    │  例: ShipMoved { ship_id, from, to, tick }
    │
    ▼
[5] EventStore への Append（永続化）
    │  - ここで失敗した場合 ECS の変更をロールバックする
    │  - 現在: FileEventStore（Phase 3 で実装済み・length-prefix + postcard）
    │  - 将来: fsync で durability を保証する
    │
    ▼
[6] Replication（ノード間伝播）
    │  - 現在: ReplicationBus（In-Memory Channel）で伝播
    │  - 将来: Sector-local → Gossip / Sector Transit → Raft
    │
    ▼
[7] Projection 更新（Readモデル）
    　 - 必要な場合のみ（将来実装）
```

### このフローから逸脱してはならない

```
禁止パターン1: バリデーション前にEventを発行する
  event_store.append(ShipMoved { ... });  // バリデーション前
  if !is_valid() { return Err(...) }      // 遅すぎる

禁止パターン2: EventStore Appendを省略してStateだけ更新する
  ecs_world.update_position(ship_id, new_pos);  // ← Eventなしで更新
  // → ノード再起動でこの更新が消える

禁止パターン3: Eventの発行とReplicationを同期的に待機する
  event_store.append(event).await?;
  replication.sync_all_nodes().await?;  // ← ここでブロックしない
  // Replication は非同期で行う。EventAppend の完了が Commit を意味する。
```

---

## 5. Entity Ownership Rules

### Shipエンティティの所有権

```
Ship は必ず 1つの Sector に所有される。
複数の Sector が同一の Ship を同時に所有してはならない。
```

**所有権の状態遷移**

```
[存在しない]
     │ ShipSpawned (sector_id 付き)
     ▼
[Sector A が所有]
     │ SectorTransitRequested
     ▼
[Transit 中 - 所有権は Sector A のまま]
     │ SectorTransitCompleted
     ▼
[Sector B が所有]
     │ ShipDespawned
     ▼
[存在しない]
```

**Transit 中の操作制限**

```rust
// Transit中のShipに対してこれらの操作は禁止:
// - MoveCommand の受理
// - 別の SectorTransit の開始
// - ShipDespawn

// TransitState を確認してから操作する
match ship.transit_state {
    TransitState::None => { /* 通常操作可 */ }
    TransitState::InTransit { .. } => {
        return Err(CommandError::ShipInTransit);
    }
}
```

**所有権の確認責務**

```
誰が確認するか:
  - Sector-local操作  → Sector Node 自身が所有を確認してから処理
  - Sector Transit    → Consensus Layer (Raft) が排他を保証
  - Read操作          → 所有権確認不要（どのノードからでも読める）
```

### NodeId による所有権

```
各 Sector は必ず 1つの Node が管理する。
同一 Sector を複数 Node が同時に管理してはならない。

Sector → Node のマッピングは Consensus Layer が管理する。
Node 障害時のフェイルオーバーは Raft のリーダー選出で処理する。
```

---

## 6. Tick Model

### Tick の定義

```rust
/// Tick は世界の論理的な時間単位である。
/// 物理時刻とは無関係。単調増加する符号なし整数。
pub type Tick = u64;

/// Tick の生成規則:
/// - 各 Sector Node が独立した Tick カウンタを持つ
/// - Tick は同一 Sector 内でのみ比較可能
/// - Sector 間の因果順序は VectorClock で表現する
```

### Tick 内の処理順序

```
現在の実装（Phase 6 時点）:
  1. Tick カウンタをインクリメント
  2. コマンドキューを処理する
       MoveCommand              → ThrustComp.direction を更新（is_braking = false）
       StopCommand              → ThrustComp.is_braking = true（逆推力で減速停止）
       LockOnCommand            → LockSystem に渡す
       ActivateModuleCommand    → FittedSlot.is_active = true / apply_fitting()
       DeactivateModuleCommand  → FittedSlot.is_active = false / apply_fitting()
  3. Movement System を実行する（ECS バッチ処理）
  4. Capacitor System を実行する           ← Movement の後
       毎 Tick: cap を recharge_per_tick 分回復
       cycle_remaining == 0 → 新サイクル: cap 消費 / cap 不足 → 強制 OFF
       cycle_remaining > 0  → デクリメント
       武器モジュールのサイクル開始 → weapon_cycles_started に ship_id を追加
       → 生成: Vec<ModuleDeactivated>（cap 枯渇時のみ）
  5. Lock System を実行する                ← Capacitor の後（位置確定後）
  6. Combat System を実行する              ← Lock の後（Locked 状態を参照）
       weapon_cycles_started に含まれる Ship のみ発射判定（ADR-0012）
       EVE 命中率式: 0.5^((angular/(tracking×sig))² + (max(0,d−opt)/falloff)²)
  7. Bot System を実行する                 ← Combat の後（破壊判定済み後）
       IsBotComp を持つ Ship のみ対象
       apply_*_owned() でプレイヤーと同一パイプラインを使用
  8. 生成されたイベントを EventStore に Append する
  9. ReplicationBus に差分を転送する       ← 必ず 8 の後
  10. 呼び出し元へ TickResult を返す

この順序を変えてはならない。
特に「8 の前に 9」を行うことは禁止する（未コミットの状態を伝播させない）。
```

### Tick の実時間目標

```
目標Tick処理時間: 16ms 以内（10,000エンティティ）
計測対象        : TickStarted → TickCompleted の経過時間
警告閾値        : 12ms を超えたら warn! ログを出力する
致命的閾値      : 32ms を超えたら Tick 遅延を記録し metrics に報告する
```

### Tick とEventの対応

```
移動・戦闘など世界の変化を表す全 Event は必ず発行時の Tick を含む:

ShipMoved     { ship_id, from, to,           tick }  // ← 必須
WeaponFired   { attacker_id, target_id, damage, tick }
DamageTaken   { ship_id, amount, current_hp, tick }
ShipDestroyed { ship_id, killer_id,          tick }

Tick なしのイベントは INV-005 違反として拒否する。
```

---

## 7. Event Schema Evolution Rules

### 基本原則

**既存の Event フィールドを変更・削除してはならない。**
**新しいフィールドの追加のみが許可される。**

### 許可される変更

```rust
// 変更前
pub struct ShipMoved {
    pub ship_id: ShipId,
    pub from   : Position,
    pub to     : Position,
    pub tick   : Tick,
}

// 変更後: 新フィールドの追加は許可（必ず Option にする）
pub struct ShipMoved {
    pub ship_id : ShipId,
    pub from    : Position,
    pub to      : Position,
    pub tick    : Tick,
    pub velocity: Option<Velocity>,  // ← 新フィールドは Option<T> で追加
}
```

### 禁止される変更

```rust
// 禁止1: フィールドの削除
pub struct ShipMoved {
    pub ship_id: ShipId,
    // from を削除 ← 禁止。過去のEventのReplayでデシリアライズが失敗する
    pub to  : Position,
    pub tick: Tick,
}

// 禁止2: フィールドの型変更
pub struct ShipMoved {
    pub ship_id: ShipId,
    pub from   : (f32, f32, f32), // Position → tuple に変更 ← 禁止
    pub to     : Position,
    pub tick   : Tick,
}

// 禁止3: フィールド名の変更（シリアライゼーションのキーが変わる）
pub struct ShipMoved {
    pub ship_id    : ShipId,
    pub origin     : Position, // from → origin に変更 ← 禁止
    pub destination: Position, // to → destination に変更 ← 禁止
    pub tick       : Tick,
}
```

### Event に破壊的変更が必要な場合の手順

```
1. 新しい Event を別名で定義する
   例: ShipMoved → ShipMovedV2

2. 古い Event を Deprecated としてマークする（削除しない）
   /// @deprecated ShipMovedV2 を使用すること
   pub struct ShipMoved { ... }

3. Upcaster を実装する
   impl Upcaster for ShipMoved {
       fn upcast(self) -> ShipMovedV2 { ... }
   }

4. Replay 時に Upcaster を通して新形式に変換する

5. docs/event-catalog.md を更新する

6. 対応する ADR を作成する（既存 ADR の更新ではなく新規作成）
```

### Event Catalog との同期

`docs/event-catalog.md` が Event の唯一の仕様書である。

```bash
# Event定義とカタログの整合をCIで検証する
cargo run --bin check-event-catalog

# このコマンドが失敗する場合、以下のいずれかが発生している:
# - コードにあってカタログにないEvent
# - カタログにあってコードにないEvent
# - フィールド定義の不一致
```

---

## 8. Testing Rules

### テストファーストの強制

**実装の前にテストを書くこと。**
テストなしの実装 PR は CI によって自動拒否される。

```
カバレッジ要件: 80% 以上（llvm-cov で計測）
例外なし。ただし以下は計測対象外:
  - main.rs のエントリポイント
  - 自動生成コード（build.rs が生成するもの）
  - ベンチマークコード（benches/ 以下）
```

### テストの種類と配置

```
単体テスト: 各 .rs ファイル末尾の #[cfg(test)] ブロック
  対象: Pure Function, ドメインロジック, CRDT のマージ操作

統合テスト: tests/integration/ 以下
  対象: EventStore の永続化・復元, Snapshot のラウンドトリップ

シナリオテスト: tests/simulation/ 以下
  対象: 3ノード構成での同期, ネットワーク分断からの復帰

ベンチマーク: benches/ 以下
  対象: 10,000エンティティの1Tick処理時間
```

### コメントとコミットメッセージは英語で書く

**すべてのコードコメントおよびコミットメッセージは英語で記述すること。**

コミットメッセージの詳細規約: `docs/commit-convention.md` を参照。

```rust
// Good
// Apply thrust vector to velocity each tick.

// Bad — Japanese causes encoding issues with some tools
// 毎 Tick、推力ベクトルを速度に加算する。
```

```
# Good commit message
feat(dawn-ecs): add CapacitorSystem with cycle-based cap drain

# Bad — Japanese subject
feat: キャパシタシステムを追加する
```

理由:
- PowerShell など一部のツールが日本語ファイルを UTF-16 で上書きしてソースを破壊するリスクがある
- ASCII のみのコメント・メッセージはあらゆるツールチェーンで安全
- 国際的な可読性
- `git log --oneline` やコードレビューツールでの文字化けを防ぐ

**移行方針（段階的）:**
- 新しく書くコードはすべて英語コメント
- 新しいコミットはすべて英語メッセージ
- 既存のファイルを変更するタイミングで、そのファイル内のコメントを英語に変換する
- 一括変換は行わない

### テストが仕様書である

テストの説明文（`#[test]` の関数名）は「何をテストするか」ではなく
「何が保証されるか」を日本語または英語で記述すること。

```rust
// 悪い例: 何をするかを書いている
#[test]
fn test_move_ship() { ... }

// 良い例: 何が保証されるかを書いている
#[test]
fn ship_moved_event_is_appended_to_log_when_move_command_is_valid() { ... }

#[test]
fn move_command_is_rejected_when_target_is_outside_sector_boundary() { ... }

#[test]
fn ecs_state_is_fully_restored_from_event_log_after_node_restart() { ... }
```

### INV 検証テストの必須化

各 Architecture Invariant（INV-001 〜 INV-006）に対して
**それが破られた場合にテストが失敗することを確認するテストを用意する。**

```rust
// INV-001 の検証テスト例
#[test]
fn event_store_rejects_update_operation() {
    let store = InMemoryEventStore::new();
    let result = store.update(EventId::new(), new_payload); // 存在しない操作
    // update メソッド自体が存在しないことをコンパイルで保証
    // または存在する場合は常に Err を返すことをテストする
}
```

### Actor のテスト方針

Actor はメッセージのやり取りをテストする。内部状態を直接参照しない。

```rust
// 悪い例: 内部状態を直接参照している
let actor = SectorSimulatorActor::new();
actor.ecs_world.get_position(ship_id); // ← 内部状態への直接アクセス

// 良い例: メッセージ経由でテストする
let (tx, rx) = mpsc::channel(10);
tx.send(QueryPosition { ship_id, reply: reply_tx }).await?;
let pos = reply_rx.await?;
assert_eq!(pos, expected_position);
```

---

## 9. AI Change Checklist

コードを変更する前に以下を確認すること。
**全項目に「問題なし」と判断できない場合は変更を止め、確認を求めること。**

### 変更前の確認

```
□ 変更するCrateを特定した
□ そのCrateの責務を Crate別責務早見表（セクション11）で確認した
□ 変更によって影響を受けるCrateを Dependency DAG（セクション3）で特定した
□ 変更が現在のスコープ内であることを確認した（セクション1）
□ 変更が Architecture Invariants（セクション2）のいずれかを破らないことを確認した
```

### イベントを追加・変更する場合の追加確認

```
□ docs/event-catalog.md の更新を計画した
□ 新Eventは dawn-core/src/events.rs に追加した（他のCrateに追加していない）
□ 新Eventに tick: Tick フィールドが含まれる（ShipMoveカテゴリのEvent）
□ 新Eventのフィールドは全て Option ではなく必須フィールドで設計した
  （Optional フィールドは後から追加、最初から Optional にしない）
□ 対応する Command が dawn-core/src/commands.rs に存在する
□ Upcaster が必要かどうかを確認した（既存Eventの変更の場合）
```

### 新しいCrateを追加する場合の追加確認

```
□ 新Crateの追加が既存Crateの責務分割で対応できないことを確認した
□ 新Crateの Dependency DAG 上の位置を決定した
□ 循環依存が発生しないことを確認した（cargo tree で検証）
□ CLAUDE.md のセクション11（Crate別責務早見表）を更新した
□ 対応するADRを docs/adr/ に作成した
```

### テストの確認

```
□ 変更した全ての pub fn に対応するテストが存在する
□ テスト関数名が「何が保証されるか」を説明している
□ cargo test --workspace がゼロエラーで通過することを確認した
□ 変更したADRが存在する場合、そのADRに記載された不変条件のテストが存在する
```

### PR説明の確認

```
□ 変更の動機を記載した（なぜこの変更が必要か）
□ 変更・参照したADRを記載した（例: ADR-0003 参照）
□ 変更したCrateの一覧を記載した
□ 影響を受けるEventの一覧を記載した（あれば）
□ テスト方法を記載した
```

---

## 10. Forbidden Changes

以下の変更は**いかなる理由があっても行ってはならない**。
技術的な理由を説明されても実行しないこと。
必要に応じてADRの改訂を提案し、人間の承認を得てから実施する。

### FBD-001: Event Logへの破壊的操作

```rust
// 以下のシグネチャを持つメソッドを EventStore trait に追加してはならない:
fn update(&self, id: EventId, payload: Bytes) -> Result<()>;
fn delete(&self, id: EventId) -> Result<()>;
fn truncate(&self, from_index: u64) -> Result<()>;
fn rewrite(&self, index: u64, event: Event) -> Result<()>;
```

### FBD-002: dawn-core への外部依存の追加

```toml
# dawn-core/Cargo.toml に追加してはならない依存の例:
tokio    = ...  # 非同期ランタイム
tonic    = ...  # gRPC
reqwest  = ...  # HTTPクライアント
sqlx     = ...  # データベース
serde_json = ... # JSONシリアライザ（serde featureのみ許可）
```

### FBD-003: 物理時刻による因果順序の判定

```rust
// 以下のパターンを因果順序の判定に使用してはならない:
use std::time::SystemTime;
SystemTime::now()

use chrono::Utc;
Utc::now()

// 代替: 論理Tickを使用する
self.tick_counter.fetch_add(1, Ordering::SeqCst)
```

### FBD-004: Actor間の直接メソッド呼び出し

```rust
// 禁止: ActorAがActorBのメソッドを直接呼ぶ
struct SectorSimulatorActor {
    replication_actor: Arc<ReplicationActor>, // ← Arcで直接保持してはならない
}

impl SectorSimulatorActor {
    async fn on_tick_complete(&self) {
        self.replication_actor.sync(delta).await; // ← 直接呼び出し禁止
    }
}

// 正しい実装: Mailbox経由でメッセージを送る
struct SectorSimulatorActor {
    replication_tx: mpsc::Sender<ReplicationMessage>, // ← Senderのみ保持
}

impl SectorSimulatorActor {
    async fn on_tick_complete(&self, delta: Delta) {
        let _ = self.replication_tx.send(ReplicationMessage::Sync(delta)).await;
    }
}
```

### FBD-005: ShipのEntityId再利用

```rust
// 禁止: Despawn済みIDのプール管理と再割り当て
struct IdPool {
    recycled: VecDeque<ShipId>,
}

impl IdPool {
    fn next_id(&mut self) -> ShipId {
        self.recycled.pop_front().unwrap_or_else(|| self.generate_new())
        // ↑ recycled からの取り出しが禁止
    }
}
```

### FBD-006: Raftを経由しないSector Transit

```rust
// 禁止: RaftをバイパスしたSector間の直接状態移転
async fn teleport_ship_between_sectors(
    &self,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
) {
    self.sector_nodes[from].remove_ship(ship_id).await; // Raftなし
    self.sector_nodes[to].add_ship(ship_id).await;     // Raftなし
}
```

### FBD-007: テストなしでのpub fnの追加

```
CIが以下を検出した場合、PRを自動拒否する:
  - pub fn が追加されているが対応するテストがない
  - カバレッジが 80% を下回る

例外はない。テストを書けない場合は pub(crate) または pub(super) にする。
```

### FBD-009: スキルポイント / キャラクター育成 / 採掘コンテンツの実装

```
【スキルポイント】
以下のいかなる形式のスキルポイント制も実装してはならない:
  - 時間経過でアンロックされる能力
  - プレイ時間に比例して強くなるパッシブ成長
  - 課金で加速できる育成要素

理由:
  ゲームの上手さに関係なく、ゲーム時間・課金額で性能が変わる。
  公平感（Perceived Fairness）を根本から損なう時代遅れの設計。

【採掘コンテンツ】
採掘レーザーを起動して放置するコンテンツを実装してはならない。

理由:
  採掘は「放置するだけ」であり、プレイヤーが意図的な判断を下す機会がない。
  EVE では採掘者は「無力な標的」として海賊側のコンテンツとして機能する。
  採掘している人自身はゲームをしていない。

  設計の中心的な問い「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」
  に対して採掘は No である。

  → docs/game-design.md §5 参照
```

### FBD-008: MVP範囲外の実装

```
以下のクレート・モジュールを作成してはならない:
  crates/dawn-economy/   ← 経済システム（Phase 4 スコープ外）
  crates/dawn-character/ ← キャラクター育成
  crates/dawn-inventory/ ← インベントリ
  crates/dawn-ui/        ← UI 専用クレート
  crates/dawn-graphics/  ← グラフィックス専用クレート

Phase 4 Cycle 3 で承認済み（作成してよい）:
  Combat / Fitting ロジックは dawn-ecs / dawn-core 内に実装する。
  独立クレートに切り出す場合は ADR を作成して承認を得ること。

これらのディレクトリが存在する場合、削除の提案を行うこと。
```

---

## 11. Crate別責務早見表

### 現在存在するクレート

| Crate | 責務 | 依存してよいもの | 禁止 |
|---|---|---|---|
| `dawn-core` | ドメインモデル定義のみ。EntityId, Position, Fitting型, 全Event型, 全Command型 | serde, thiserror のみ | ネットワーク、ファイルI/O、非同期 |
| `dawn-ecs` | ECS World の薄いラッパー。Component定義（Movement/Fitting/Combat）, System定義 | dawn-core, hecs | ネットワーク、EventStore |
| `dawn-event-store` | Event Log の永続化。Append, Read, Snapshot（InMemory + File） | dawn-core, serde | ネットワーク、ECS |
| `dawn-actor` | Actor基盤。EventStoreActor, ReplicationBus, ClientConnection trait | dawn-core, dawn-event-store, tokio | dawn-ecs, dawn-simulation |
| `dawn-simulation` | 実行バイナリ。SimulationNode, MultiNodeCluster, WsServer（Godot WebSocket接続）, 負荷生成, DataLoader（TOML読み込み） | 上記全て + rand + tokio-tungstenite + toml | — |

### 将来追加予定のクレート（まだ存在しない・実装しないこと）

| Crate | 予定フェーズ | 責務（予定） |
|---|---|---|
| `dawn-consensus` | Phase 7 | Raft実装。Leader選出, Log Replication |
| `dawn-replication` | Phase 8 | Gossip + CRDT。差分伝播, LWW-Register |
| `dawn-proto` | Phase 5 | protobuf定義と生成コード |
| `dawn-sector-node` | Phase 5 | 本番実行バイナリ。Actorの配線と起動 |

---

## 12. よくある設計違反パターン

AIが陥りやすいアンチパターンとその修正方法を示す。

### パターン1: 「便利だから」とState同期を使う

```
状況: ノード間でPosition差分が発生した時、Stateを直接上書きで同期しようとする

違反コード:
  // "Eventより直接同期の方が速い" という誤った判断
  node_b.update_position(ship_id, node_a.get_position(ship_id))

正しい判断:
  EventをGossipで伝播させる。StateはEventから自動的に収束する。
  State直接同期は INV-001 と INV-002 を同時に破る。
```

### パターン2: テストをスキップして「後で書く」

```
状況: 実装が複雑でテストを後回しにしようとする

なぜ危険か:
  AIは次のセッションでコンテキストを持ち越さない。
  「後で書く」は「永遠に書かない」と等しい。
  テストなしのコードは次のAIセッションで意図せず破壊される。

対処:
  実装が複雑ならテストを先に書き、テストを通す最小実装を先に行う。
  テストが仕様書になる。
```

### パターン3: 新機能のためにdawn-coreを肥大化させる

```
状況: 新しい機能を追加するとき、dawn-coreに実装ロジックを追加しようとする

違反コード（dawn-core/src/position.rs）:
  impl Position {
      pub async fn broadcast_to_nodes(&self, nodes: &[NodeAddr]) { // ← ネットワーク処理
          ...
      }
  }

正しい判断:
  dawn-core はデータ定義のみ。
  ネットワーク処理は dawn-replication または dawn-sector-node に配置する。
```

### パターン4: Tickを物理時刻に「合わせる」最適化

```
状況: "Tickと実時間を合わせると分かりやすい" という理由で物理時刻を使おうとする

危険性:
  物理時刻に依存した瞬間、3ノード間で Tick の順序が非決定論的になる。
  テスト環境と本番環境でTick順序が変わる可能性がある。
  NTPのステップ補正で時刻が逆行した瞬間、システムが破綻する。

対処:
  Tick は論理カウンタのまま維持する。
  "人間が読みやすい時刻" は Observation Layer（ログ・メトリクス）でのみ使う。
  INV-005 を参照すること。
```

### パターン5: Sector Transitを「最適化」してRaftをスキップする

```
状況: "レイテンシ削減のため" Sector Transit を Raft なしで実装しようとする

違反の結果:
  2つのノードが同一Shipの所有権を同時に主張する状態（スプリットブレイン）
  → 両方のSectorが独立したShipMoveを処理し始める
  → 世界が分岐する（Single Shardの破壊）

対処:
  Sector Transit は必ず Raft を経由する。INV-003 を参照すること。
  レイテンシが問題なら Transit の頻度を下げる設計を検討する。
  ※ Raft は Phase 7 で実装予定。現在（Phase 4）は単一Sectorで運用。
```

### パターン6: FittingSnapshot をイベントに含めず ID だけ記録する

```
状況: "モジュールIDだけ保存してレジストリで引けば十分" という判断で
      ShipFitted イベントに ModuleId のリストだけを含めようとする

違反の結果:
  レジストリの内容が変わった場合（モジュールの stat が更新されるなど）、
  過去の Event を Replay すると当時と異なる stat が再現される。
  → INV-002 違反（Event Replay で世界が完全に再現されない）

正しい実装:
  ShipFitted イベントには FittingSnapshot（モジュール定義全体）を含める。
  Replay はレジストリに依存せず、イベントの内容だけで完結しなければならない。
  → ADR-0006 §1 参照
```

### パターン8: 状態変化をイベントとして表現する

```
状況: モジュールのオン/オフを表すイベントに is_active フラグを持たせようとする

違反コード:
  ModuleToggled { ship_id, module_id, is_active: bool, tick }
  // → is_active を見ないと何が起きたかわからない
  // → 状態の記述であって「事実」ではない

正しい実装:
  ModuleActivated   { ship_id, module_id, slot, tick }  // オンにした
  ModuleDeactivated { ship_id, module_id, slot, tick }  // オフにした
  // → イベント名自体が「何が起きたか」を表す

原則:
  Event は既に起きた事実（INV-006）。
  「状態がこうなった」ではなく「この動作が起きた」と命名する。
  過去形・動詞（Activated, Fired, Destroyed）を使う。
  is_*/has_* フラグをイベントのキーフィールドにしない。
```

### パターン7: 特定座標を「シグナル」として流用する

```
状況（Phase 4 で採用した暫定実装）:
  MoveCommand の target_position に Position::ORIGIN (0,0,0) を送ると
  サーバーが「プレイヤー船に指定せよ」と解釈するという特殊ルールが存在する。

なぜ危険か:
  座標とシグナルが同一チャンネルで混在しており、
  次の AI セッションが「MoveCommand は常に移動先」と誤読するリスクがある。
  将来、原点付近でのプレイヤー操作と誤判定する可能性がある。

現状:
  Phase 4 の暫定措置として許容している。
  Phase 5 で Hello/Welcome ハンドシェイクに置き換える（ADR-0007 §2）。
  → この暫定ルールを拡張・踏襲しないこと。
```

---

## 付録: 参照すべきドキュメント

```
設計の根拠   : docs/adr/ 以下の各ADRファイル
Eventの仕様  : docs/event-catalog.md
Crate一覧    : Cargo.toml (workspace)
型の定義     : dawn-core/src/ 以下
```

## 付録: このファイル自体の更新ルール

CLAUDE.md の変更は以下の条件を全て満たす場合のみ許可する。

```
1. 対応するADRが存在する（新規作成または更新）
2. 変更内容が既存のセクションと矛盾しない
3. 人間のレビューと承認を得ている

AIは CLAUDE.md を自律的に変更してはならない。
変更が必要と判断した場合は、変更提案を出して人間の判断を求めること。
```

---

*最終更新: 2026-06-10（Phase 6 完了・EVE命中率式 / タクティカルオーバーレイ / StopCommand / ボットAI反映）*
*対応ADR: ADR-0001 〜 ADR-0013*
*次回レビュー予定: Phase 7 着手前（ADR-0009 実装開始前）*
