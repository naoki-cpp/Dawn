# AI_DEVELOPMENT_GUIDE.md

This file provides guidance to AI coding agents when working with code in this repository.

---

# dawn プロジェクト AI開発ガイド

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
cargo test velocity_changed                   # テスト名フィルタ

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
cargo run -p dawn-simulation --bin simulate --release -- --serve --cluster   # 3ノード Raft クラスタ（Jump Gate 有効・ADR-0009/0014。--ships N も可）
cargo run -p dawn-simulation --bin simulate --release -- --serve --duel       # 1 human vs 1 Bot（NPC なし・デュエル計測）
cargo run -p dawn-simulation --bin simulate --release -- --serve --pop-cap 100 # Sector 人口バックストップを可変化（ADR-0018・8B-1。--cluster と併用可）
cargo run -p dawn-simulation --bin simulate -- --raft-demo            # Phase 7 Raft Transit デモ（ADR-0014）
cargo run -p dawn-simulation --bin simulate --release -- --aoi-bench  # AoI スケーリングベンチ（ADR-0019）
```

**WebSocket サーバー起動後の接続先**: `ws://127.0.0.1:7878`

# ゲームバランス調整（リビルド不要）
# data/ ディレクトリの TOML を編集してサーバーを再起動するだけでよい
# ファイルが見つからない場合は ship_types.rs / modules.rs のデフォルト値を使用
data/ship_types.toml   # 船種定義（HP・速度・スロット数など）
data/modules.toml      # モジュール定義（ダメージ・射程・StatDelta など）

# コミット（英語・Conventional Commits 準拠）
# → 規約と例は docs/process/commit-convention.md を参照

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

### このプロジェクトのゴール（ADR-0016）

> **EVE Online を超えるゲームを作る。**

数万エンティティを3ノードで遅延なく同期する分散基盤は、そのゴールの **実現手段** であり、
かつ EVE が Time Dilation で諦めた領域を突く **競争優位** である。
「ゲームではない」という従来の宣言は ADR-0016 で撤回した。

技術的な土台（＝競争優位の源泉）:

- Single Shardの分散シミュレーション
- イベントソーシングによる完全な因果追跡
- 追記ログのゴシップ配布とRaftの責務分離による高スループット同期（ADR-0021。Sector-local は単一所有のため競合解決 CRDT を要さず、追記ログのゴシップ配布で収束する。Raft は Sector 越え transit 専用）
- アンチ TiDi（INV-TiDi）— TiDi 閾値を EVE より桁違いに高く保ち、出ても局所・短時間・自動回復に抑える（ADR-0018）

**4本の柱（EVE を超える差別化 / ADR-0016）**:

1. **TiDi 閾値が桁違いに高い大規模リアルタイム戦闘** — 分散基盤を武器に「多数が遅延なく戦う」を実現する。TiDi が出ても局所・短時間・自動回復に抑える（ADR-0018）。
2. **グラインドゼロの深い戦闘** — 退屈なグラインドを排し、意図的判断だけで構成する。
3. **プレイヤー主導の世界** — プレイヤーが構造物・防衛・領域・経済を作る（非 Web3）。
4. **実損のある危険な宇宙** — 撃沈＝実損、Tackle で逃がさない非対称リスクを核に据える。

> 設計の中心的な問い（docs/design/game-design.md）は不変:
> 「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」

### 現在のスコープ（Phase 8A/8B/8C 完了・ADR-0025 実装済み）

```
実装対象:
  エンティティ  : Ship のみ
  コンポーネント: Position(x, y, z), Velocity, ThrustComp, ShipStatsComp,
                  HullComp（Shield/Armor/Hull 3層）, FittingComp（装備スロット）,
                  CapacitorComp（現在 cap 量）, TransitComp（Transit 状態）,
                  ApproachComp（接近対象 Ship/Gate・半自動操船 / ADR-0015）,
                  WarpComp（intra-Sector ワープ = 短距離 Fold・align/warping / ADR-0022）
  船種          : ShipTypeDefinition（id, name, class, base_stats, slot_layout）
  イベント      : ShipSpawned（ship_type_id 含む）, VelocityChanged, SectorTransit系,
                  ShipFitted, WeaponFired, DamageTaken（3層 HP）, ShipDestroyed,
                  ModuleActivated, ModuleDeactivated
  ノード構成    : 3ノード固定

追加承認済み機能（全て実装済み・詳細は各 ADR / docs を参照）:
  ADR-0006  Fitting / Combat / Lock-on（EVE 準拠 Active/Passive・2フェーズ戦闘・HP 3層）
  ADR-0011  Capacitor（サイクルベース cap 管理・cap 枯渇で強制 OFF）
  ADR-0014  Raft コンセンサス + Sector Transit（dawn-consensus・Step 7.5 適用）
  ADR-0009  星系間ナビゲーション（JumpCommand / JumpGateUsed / StarSystemChanged・J キー）
  ADR-0015  Approach（半自動操船・対象 Ship/Gate・Step 2.5・Move/Stop で解除・A キー）
  ADR-0022  intra-Sector Warp（短距離 Fold・align/warping 2フェーズ・Step 2.6・W キー）
  ADR-0023  Propulsion 慣性モデル（mass/inertia_modifier・指数接近・auto-warp-then-jump）
  ADR-0024  Tackle（Fold Disruptor・TackledComp・Step 4.5・warp/jump 拒否・snapshot 永続化）
  ADR-0025  天体（恒星・惑星・WarpTarget::Body・sun_direction シェーダー・天体ワープ）

  ※ 各機能が触る型・イベント・Tick ステップの正確な仕様は対応 ADR と
    docs/architecture/event-catalog.md / docs/architecture/tick-model.md を一次情報とする（ここでは重複させない）。

実装しない（提案も拒否する / 反グラインドの核 — FBD-009）:
  スキルポイント制 / 時間経過・課金による受動成長（= キャラクター育成）
  AFK 採掘（放置で進む採取・意図的判断を伴わない作業）
  Pay-to-Win（性能を金で買う）
  物理エンジンへの外部依存

実装してよい（ADR 承認で解禁 — FBD-008 撤廃 / ADR-0016）:
  市場・経済 / キャラクター（エンティティとして・育成は除く） / インベントリ /
  UI / グラフィクス。独立クレート化する場合は個別 ADR と Dependency DAG 上の位置確定が必要。
```

ゴール（ゲーム化）に沿う機能拡張は ADR を起票して人間の承認を得てから実装する（§9）。
反グラインドの核（スキル育成・AFK 採掘・P2W）に該当する提案は、ゴール再定義後も拒否する。
拡張は段階的に行う（まず Ship 中心の戦闘の深み: Tackle → Signature Resolution →
Orbit/Keep at Range → Logistics → 資源シンク。受動採取は採らない / ADR-0016 §5）。

### 絶対に変えてはならない設計原則

```
原則1: Event が唯一の真実。State は派生物に過ぎない。
原則2: Event は追記のみ。既存のEventを変更・削除しない。
原則3: 因果順序は論理Tick + NodeIdで決定する。物理時刻を使わない。
原則4: Crate依存は一方向のみ。循環依存は設計の失敗を意味する。
原則5: Actor間の通信はMailbox経由のみ。直接メソッド呼び出し禁止。
原則6: Tickの論理速度は通常一定。過負荷は 分割 → LoD → 局所 TiDi → 入場制限 の順で対処する（ADR-0018）。単一密戦闘では締め出すより全員が少し遅い方を優先する。
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

### INV-002: スナップショットが権威ある永続チェックポイント。状態はスナップショット + 末尾 catch-up で再構築する（ADR-0017 改訂）

```
検証方法（運用復旧経路）:
  1. ノードをシャットダウンする
  2. In-Memory状態を破棄する
  3. (最新の検証済みスナップショット) + (それ以降のホットログのイベントの catch-up)
     から State を再構築する
  4. シャットダウン直前の状態と一致することを確認する

これが成立しない実装は INV-002 違反である。
```

不変条件（ADR-0017）:
- スナップショットは**権威ある永続状態**である（単なる最適化ではない）。クラッシュ復旧・
  failover（ADR-0014）はスナップショット + 末尾 catch-up で行う。運用ホットパスで要る replay は
  「末尾の catch-up」のみ。創世記からの完全 replay に依存する運用経路は無い。
- 派生・transient 状態（位置・capacitor・lock カウントダウン等）はスナップショットに永続化する。
  これらは毎 Tick の純粋関数（位置 = velocity 積分、cap = recharge）でありイベントには記録しない。
  ゆえに「イベントのみから」再構築できる必要はない（位置と同じ扱い）。復旧後は live の Tick で再計算される。
- スナップショットは検証可能でなければならない:
    ① snapshot → restore → snapshot がバイト一致（round-trip）
    ② snapshot + 末尾 Tick の再実行 == その時点の live 状態
- 創世記（log index 0）からの再構築は**経路外**（監査・災害復旧のみ）。イベント適用で権威ある状態を
  組み直し、transient 派生状態は sim を前進させて再計算する。通常運用・failover の依存先ではない。
- ホットログは検証済みスナップショットの背後を圧縮して有界に保つ。圧縮はセグメント移送であり、
  イベントの破壊ではない（FBD-001 維持）。

具体的な違反:
- 権威ある状態変化（破壊・装備・所有権など）をイベントなしに行う（→ ノード間伝播・監査が壊れる）
- スナップショットが round-trip（①）でバイト一致しない（検証不能なスナップショット）
- Event の Payload に後から追加されたフィールドが復元時にデフォルト値になる

### INV-003: Sector-local操作はSector境界を越えない

```
違反例:
  // SectorAのActorがSectorBのEntityを直接操作する
  sector_b_actor.move_ship(ship_id, new_pos)

正しい実装（Phase 7 / ADR-0014 で実装済み）:
  // バリデーション後、Raft Log に TransitOp を提案する
  if node.can_propose_transit(ship_id) {
      raft.propose(TransitOp::Request { ship_id, to }.encode());
  }
  // コミット済みエントリは Tick Step 7.5（apply_committed_raft_entries）で
  // 適用され、SectorTransitRequested / Completed が EventStore に Append される
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

### INV-TiDi: 論理 Tick 速度は通常一定。TiDi は境界つき局所的最終手段（ADR-0018 改訂）

```
方針:
  論理 Tick 速度は通常負荷下では一定に保つ。
  過負荷は「分割 → LoD → 局所 TiDi → 入場制限」の順で対処する。
  単一密戦闘では入場制限が最後（締め出すより全員が少し遅い方を優先）。

Time Dilation（論理 Tick の単調性・決定性を保ったまま、実時間ペースのみを落とす）は、
分割不能な単一ホットスポットがノード容量を超えた場合に限り、次をすべて満たすときのみ許可:
  (a) 局所性   : dilation は当該 Sector に限定。隣接へ伝播させない。
  (b) 観測可能 : dilation 係数・継続時間を SLA イベント / メトリクスに記録する。
  (c) 非破壊   : イベントの並べ替え・欠落・結果変化を起こさない（純粋な実時間ペーシング）。
  (d) 自動回復 : 負荷が引いたら係数を 1.0 へ戻す。
  (e) 発動順序 : 分割・LoD の後。入場制限は TiDi でも保ちきれない極端時のみの最終バックストップ。

違反例:
  // 全 Sector を一律に dilate する（EVE 型・局所性 (a) 違反）
  for sector in all_sectors { sector.dilation = 0.1; }

  // dilation でイベントを間引く / 並べ替える（決定性破壊・(c) 違反）
  if dilated { drop_events(non_critical); }

  // 物理時刻で dilation を判定する（INV-005 違反。判定は論理 Tick の処理予算超過で行う）
  if SystemTime::now() - tick_start > budget { ... }   // ← 物理時刻は使わない

正しい実装:
  // 局所・観測可能・非破壊・自動回復
  if sector.tick_cost > sector.budget && !sector.splittable() {
      sector.dilation = (sector.budget / sector.tick_cost).max(MIN_DILATION);
      metrics.tidi_active.set(sector.id, sector.dilation);   // (b)
  } else if sector.dilation < 1.0 && sector.tick_cost <= sector.budget {
      sector.dilation = 1.0;                                  // (d) 自動回復
  }
```

理由: 単一密戦闘は分割不能。旧「絶対アンチ TiDi」は手段が入場制限のみになり、
クライマックスから締め出す＝EVE の TiDi より悪い体験になりうる（eve-reference §11.1）。
TiDi は論理 Tick の決定性を壊さない（INV-005 と無関係・純粋な実時間ペーシング）ため、
局所・観測可能・非破壊・自動回復・後置の条件下でのみ最終手段として許可する。
差別化は「TiDi が無い」ではなく「閾値が EVE より桁違いに高く、出ても局所・短時間・自動回復」。
→ 詳細設計は docs/architecture/tick-model.md §8 を参照。

---

## 3. Dependency DAG

### 許可された依存方向

```
dawn-core
    ↑ (依存してよい)
    ├── dawn-ecs
    ├── dawn-consensus      ← Raft（ADR-0014: state machine, RaftActor, RaftTransport）
    └── dawn-event-store
            ↑
            ├── dawn-actor          ← ClientConnection 境界（InProcess / WebSocket）
            ├── dawn-replication    ← 追記ログのゴシップ配布・InMemoryReplicationBus（ADR-0021/0027）
            └── dawn-sector         ← ゲームロジック（ADR-0026: SimulationNode・Warp・Transit・AoI）
                    ↑                  （dawn-ecs, dawn-consensus にも依存する）
                    ├── dawn-simulation  ← 実行バイナリ・配線・負荷生成
                    │   （dawn-actor, dawn-consensus にも依存する）
                    └── dawn-sector-node ← 本番実行バイナリ（8D-4）
                        （dawn-consensus, dawn-replication にも依存する）
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

### ワイヤ形式は postcard + serde を再利用する（`dawn-proto` / protobuf は採らない）

旧計画の `dawn-proto`（protobuf 定義クレート）は**採用しない**。理由:
ノード間は Rust↔Rust（多言語不要）、クライアント境界は既に JSON/WebSocket（ADR-0007）、
スキーマ進化は §7 のドメイン規律（Option 追加 / V2 / upcaster + event-catalog）で形式非依存に扱える。
既に postcard + serde が全イベント型・Raft メッセージ・スナップショットに効いており、ネットワーク層も
これを再利用する。protobuf は全型の二重定義 + ビルドステップを生むだけで見返りが乏しい。

唯一の小さな実需＝ワイヤのフレーミング/バージョニング（長さ前置・メッセージ種別タグ・プロトコル版
ハンドシェイク）は transport 層の小モジュールで賄う。混在バージョンのローリングアップグレードを
将来要件にする場合のみ、タグベースの進化可能形式を個別 ADR で再検討する。

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
    │  - 現在: dawn-replication::InMemoryReplicationBus（単一プロセス）
    │  - 将来: Sector-local → TCP Gossip / Sector Transit → Raft
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

**規範的定義（各ステップの詳細処理・生成イベント）は docs/architecture/tick-model.md §3 を一次情報とする。**
ここでは順序と「越えてはならない境界」だけを示す。

```
  1.   Tick カウンタをインクリメント
  2.   コマンドキューを処理（Move/Stop/Lock/Activate/Deactivate/Jump/Approach/Warp）
       ※ Transit 中（InTransit）の Ship への Move/Stop/二重 Transit/Jump/Approach/Warp は拒否（§5）
       ※ Jump/Warp は can_propose_jump()/can_propose_warp() 検証を通すこと
  2.5  Approach System  ← Movement の前（ADR-0015）
  2.6  Warp System      ← Approach の後・Movement の前（ADR-0022/0023/0025）
  3.   Movement System  （warping 中の船はスキップ）
  4.   Capacitor System ← Movement の後・Lock の前
  4.5  Tackle System    ← Capacitor の後・Lock の前（ADR-0024）
  5.   Lock System      ← 位置確定後（Movement の後）
  6.   Combat System    ← Lock の後（Locked 状態を参照）
  7.   Bot System       ← Combat の後（破壊判定済み後）
  8.   生成イベントを EventStore に Append
  9.   dawn-replication transport に差分を転送  ← 必ず 8 の後
  10.  RaftActor に TickElapsed（ADR-0014・INV-005/FBD-003）
  11.  TickResult を返す
```

この順序を変えてはならない（変更には ADR が必要）。
特に「8 の前に 9」を行うことは禁止する（未コミットの状態を伝播させない）。

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

**現在は Phase 6（プレリリース）。永続化された外部ユーザーのイベントログが存在しない
ため、破壊的変更（フィールド削除・型変更・イベント削除）を直接行ってよい
（Upcaster・V2 命名・Deprecated マークは不要）。ただし docs/architecture/event-catalog.md と本ガイドは
常に実態と合わせること。**

リリース以降の制約（既存フィールドの変更・削除禁止、Upcaster 手順、許可/禁止の
コード例）と Event Catalog 同期の詳細は **docs/architecture/event-schema-evolution.md** を参照。

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
  対象: Pure Function, ドメインロジック, ゴシップ適用の冪等性（ADR-0021）

統合テスト: tests/integration/ 以下
  対象: EventStore の永続化・復元, Snapshot のラウンドトリップ

シナリオテスト: tests/simulation/ 以下
  対象: 3ノード構成での同期, ネットワーク分断からの復帰

ベンチマーク: benches/ 以下
  対象: 10,000エンティティの1Tick処理時間
```

### コメントとコミットメッセージは英語で書く

**すべてのコードコメントおよびコミットメッセージは英語で記述すること。**

コミットメッセージの詳細規約: `docs/process/commit-convention.md` を参照。

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

### Godot クライアントのテスト方針（GdUnit4）

**`client/scripts/` を変更するときは、可能な範囲でテストを伴わせること。**
テストフレームワークは [GdUnit4](https://github.com/MikeSchulze/gdUnit4)。
`client/addons/` は `.gitignore` 対象（各開発者が Godot エディタの AssetLib から
個別にインストールする想定）なので、**初回はエディタの AssetLib タブで
「GdUnit4」を検索してインストールし、`project.godot` の Plugins でこのアドオンを
有効化**すること（`enabled=PackedStringArray("res://addons/gdUnit4/plugin.cfg")`
は既にコミット済み。アドオン本体だけが各マシンでの個別インストール対象）。
テストは `client/test/` 以下に `<対象ファイル>_test.gd` として置く（例: `client/test/main_test.gd`）。

クライアント側はサーバー側（Rustクレート）と違い**全コードをテストできるわけではない**。
判断基準は以下の通り:

```
テスト可能（シーンツリー無依存の純粋関数・ロジック）:
  - 座標変換、レイ/距離計算、配列・辞書を入出力とする計算
  - 例: _server_to_godot_pos() / _ray_point_distance() / _spectral_color() /
        _compute_warp_snap_pos_core()（client/test/main_test.gd 参照）
  - スクリプトを .new() でシーンツリーに追加せずインスタンス化すれば _ready() は
    呼ばれないため、@onready 変数を使わない関数なら安全にテストできる

テスト不能・対象外（Godot エディタでの目視確認に委ねる）:
  - HUD構築・更新、入力ハンドリング、マーカー（ノード）生成、ピッキングのループ自体
  - @onready のシーンツリー直パス参照に依存する処理
  - WebSocket 通信（connection.gd の実接続部分）
  → これらは docs/architecture/architecture-review-client.md の C-1/C-3 で「Godot エディタでの
    動作確認が必要」と明記した領域と一致する
```

**新しい純粋関数を `main.gd` 等に追加・抽出するときは、テストも同じ変更に含めること。**
逆に、シーンツリー依存のロジックを変更したときは、テストを書けない代わりに
「Godot エディタで何を確認したか」を PR 説明に明記する（実機検証ができないAIセッションの
場合は、その旨と推奨される手動確認手順を明記する）。

**Godot バイナリの取得**: リポジトリには Godot 本体を含めない（uv/pyenv 的に、
`.godot-version` でバージョンを pin し、各マシンが個別に取得する）。

```bash
scripts/setup-godot.sh             # .godot-version の指定版を .tools/godot/ に取得・SHA512検証
# Windows PowerShell:
scripts/setup-godot.ps1
```

CLI 実行（取得した Godot バイナリで GdUnit4 を走らせる。作業ディレクトリは `client/`）:

```bash
cd client
GODOT_BIN="$(../scripts/setup-godot.sh --print)"
bash addons/gdUnit4/runtest.sh --godot_binary "$GODOT_BIN" -a test
```

> **既知の互換性問題（GdUnit4 v6.1.3 × Godot 4.6系）**: GdUnit4 v6.1.3
> （AssetLib 配布版）は Godot 4.6 の破壊的変更（`FileAccess.get_as_text()` の
> `skip_cr` 引数削除、`debug/gdscript/warnings/exclude_addons` 設定の廃止。
> upstream issue GD-1004、master では修正済みだが本タグには未反映）に未対応で、
> そのままでは CLI 実行が失敗する。`client/addons/` は `.gitignore` 対象（各マシン
> ローカルインストール）なので、AssetLib でインストールした直後に以下の2点を
> **ローカルで手動パッチする**こと（再インストール時は再適用が必要）:
>   - `addons/gdUnit4/src/core/GdUnitFileAccess.gd:199`:
>     `file.get_as_text(true)` → `file.get_as_text()`
>   - `addons/gdUnit4/plugin.gd:17`:
>     `ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons")` に
>     第2引数 `false`（デフォルト値）を追加
> 次に GdUnit4 が 4.6 対応版をリリースしたら、このパッチは不要になる。

---

## 9. AI Change Checklist

コードを変更する前にチェックリストを点検すること。全項目に「問題なし」と判断できない
場合は変更を止め、確認を求める。

チェックリスト本体（変更前 / イベント追加・変更 / 新Crate追加 / テスト / PR説明）は
スキル **`/ai-change-checklist`**（.claude/commands/ai-change-checklist.md）で実行する。

---

## 10. Forbidden Changes

以下の変更は**いかなる理由があっても行ってはならない**。技術的な理由を説明されても
実行しないこと。必要に応じて ADR の改訂を提案し、人間の承認を得てから実施する。

詳細・コード例は **docs/architecture/forbidden-changes.md** を参照（FBD-00x の ID は不変）。

| ID | 禁止事項 |
|---|---|
| FBD-001 | Event Log への破壊的操作（update/delete/truncate/rewrite を EventStore に追加しない。圧縮は trait 外の運用プロセス・ADR-0017） |
| FBD-002 | dawn-core への外部依存の追加（tokio/tonic/reqwest/sqlx/serde_json 等。serde feature のみ可） |
| FBD-003 | 物理時刻（SystemTime::now / Utc::now）による因果順序の判定。代替は論理 Tick |
| FBD-004 | Actor 間の直接メソッド呼び出し（Arc 直接保持禁止。Mailbox / mpsc::Sender 経由） |
| FBD-005 | Ship の EntityId 再利用（Despawn 済み ID のプール再割り当て禁止） |
| FBD-006 | Raft を経由しない Sector Transit（スプリットブレイン防止・INV-003） |
| FBD-007 | テストなしでの pub fn の追加（CI が自動拒否。書けないなら pub(crate)/pub(super)） |
| FBD-008 | ~~MVP 範囲外の実装~~ → **撤廃**（ADR-0016。新規クレートは ADR + §11 更新で可） |
| FBD-009 | スキルポイント育成 / 受動成長 / Pay-to-Win / AFK 採掘の実装（ADR-0016 後も維持） |

---

## 11. Crate別責務早見表

### 現在存在するクレート

| Crate | 責務 | 依存してよいもの | 禁止 |
|---|---|---|---|
| `dawn-core` | ドメインモデル定義のみ。EntityId, Position, Fitting型, 全Event型, 全Command型 | serde, thiserror のみ | ネットワーク、ファイルI/O、非同期 |
| `dawn-ecs` | ECS World の薄いラッパー。Component定義（Movement/Fitting/Combat）, System定義。**分類軸**: トポロジー（セクター・ゲート）を知らない。DomainEvent を `Vec` で返すが Event Store には書かない（書くのは dawn-sector の責務） | dawn-core, hecs | ネットワーク、EventStore |
| `dawn-event-store` | Event Log の永続化。Append, Read, Snapshot（InMemory + File） | dawn-core, serde | ネットワーク、ECS |
| `dawn-consensus` | Raft実装（ADR-0014）。Leader選出, RaftActor, RaftTransport（In-Process / TCP）, TcpRaftTransport（8D-3: 4-byte LE + postcard / LAN plaintext / per-peer 自動再接続） | dawn-core, serde, rand, tokio, postcard, thiserror | ネットワーク、ECS、EventStore |
| `dawn-actor` | クライアント転送境界。ClientConnection trait（+ InProcessConnection / WsClientConnection 実装）、`ws_server`（WsServer / PlayerSession）、`protocol`（DomainEvent↔JSON↔ClientCommand）。両バイナリ共有 | dawn-core, tokio, tokio-tungstenite, serde, serde_json, futures-util, anyhow, thiserror | dawn-ecs, dawn-simulation |
| `dawn-replication` | 追記ログのゴシップ配布境界。8D-2a: InMemoryReplicationBus + ReplicationTransport。8D-2b: AntiEntropy（gap 検出・重複/overlap 判定・`iter_from` suffix 応答）。8D-2c: TcpReplicationTransport（4-byte length prefix + postcard / LAN plaintext）。8D-2d: SnapshotTransfer（Serialize+DeserializeOwned ジェネリック / 256 MiB cap）。消費側: ReplicaSet（peer セクターごとに gap 検出・冪等・順序保持で複製ログを保持・M-5） | dawn-core, dawn-event-store, serde, postcard, tokio, thiserror | dawn-ecs, dawn-sector, dawn-consensus, dawn-simulation |
| `dawn-sector` | Sector単位のゲームロジック。SimulationNode（Tick実装・コマンド処理・Transit・Warp・Bot AI・AoI）, SpawnConfig, Galaxy, StateSnapshot, CheckpointScheduler, TiDi計算（ADR-0026）。**分類軸**: トポロジー（セクター・ゲート）を知る。Event Store への書き込みまで責任を持つ。dawn-ecs の systems を呼び出し、返ってきた `Vec<DomainEvent>` を store に記録する | dawn-core, dawn-ecs, dawn-event-store, dawn-consensus, serde, postcard, tokio | ネットワークI/O、WebSocket、ファイルI/O直接 |
| `dawn-simulation` | 実行バイナリ・配線のみ。MultiNodeCluster（RaftActor 配線含む）, WsServer（Godot WebSocket接続）, 負荷生成, DataLoader（TOML読み込み） | 上記全て + dawn-sector + rand + tokio-tungstenite + toml | ゲームロジックの直接実装 |
| `dawn-sector-node` | 本番実行バイナリ（8D-4）。TcpRaftTransport + TcpReplicationTransport を TOML 静的 config で配線。3 プロセスで 3 セクタクラスタ。Jump 時は Redirect JSON でクライアントを宛先 WS へ誘導 | dawn-core, dawn-sector, dawn-consensus, dawn-replication, dawn-actor, serde, serde_json, tokio, tokio-tungstenite, toml, anyhow, rand | ゲームロジックの直接実装 |

---

## 12. よくある設計違反パターン

AI が陥りやすいアンチパターン（State 直接同期 / テスト後回し / dawn-core 肥大化 /
Tick の物理時刻化 / Raft スキップ / FittingSnapshot 省略 / 状態フラグのイベント化）と
その修正方法は **docs/architecture/design-violations.md** を参照。

---

## 付録: 参照すべきドキュメント

```
設計の根拠       : docs/adr/ 以下の各ADRファイル
Eventの仕様      : docs/architecture/event-catalog.md
Crate一覧        : Cargo.toml (workspace)
型の定義         : dawn-core/src/ 以下
禁止変更の詳細   : docs/architecture/forbidden-changes.md（§10 FBD-00x の正典）
設計違反パターン : docs/architecture/design-violations.md（§12 の正典）
イベント進化規則 : docs/architecture/event-schema-evolution.md（§7 詳細の正典）
変更前チェック   : /ai-change-checklist スキル（§9 の正典）
```

## 付録: このファイル自体の更新ルール

AI_DEVELOPMENT_GUIDE.md の変更は以下の条件を全て満たす場合のみ許可する。

```
1. 対応するADRが存在する（新規作成または更新）
2. 変更内容が既存のセクションと矛盾しない
3. 人間のレビューと承認を得ている

AIは AI_DEVELOPMENT_GUIDE.md を自律的に変更してはならない。
変更が必要と判断した場合は、変更提案を出して人間の判断を求めること。
```

---

*最終更新: 2026-06-23（ADR-0030 ステアリング再構成 — §9 を /ai-change-checklist スキルへ、§10 詳細を docs/architecture/forbidden-changes.md へ、§12 を docs/architecture/design-violations.md へ、§7 詳細を docs/architecture/event-schema-evolution.md へ降格。ガイド本体は ID 一覧・要約・リンクのみ残置。番号体系（INV-/FBD-/§）と文言は不変。人間承認済み）*
*前回更新: 2026-06-19（肥大化解消 — §1 スコープの ADR ごと実装メモを ADR 参照テーブルに圧縮、§6 Tick 処理順序を docs/architecture/tick-model.md §3 へ委譲（順序と境界のみ残置）。不変条件・禁止事項の文言は不変。人間承認済み）*
*対応ADR: ADR-0001 〜 ADR-0030（ADR-0020 Simulation LoD は deferred）*
*次回レビュー予定: Phase 8D（分散インフラ）設計時 / Signature Resolution 着手時*
