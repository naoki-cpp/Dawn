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

> 設計の中心的な問い（docs/game-design.md）は不変:
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

Phase 4 以降で追加承認済み（全て実装済み）:
  Fitting システム（EVE Online 準拠・Active/Passive モジュール）
  Combat システム（武器 / ダメージ / HP 3層 / 破壊）
  Lock-on システム（2フェーズ戦闘）
  ShipType システム（船種・船クラス・スロットレイアウト）
  Capacitor システム（サイクルベース cap 管理・強制 OFF）

Phase 7 で追加承認済み（ADR-0014・実装済み）:
  Raft コンセンサス（dawn-consensus: Leader 選出 / Log Replication / RaftActor / 障害注入）
  Sector Transit（TransitCommand → Raft Log コミット → Step 7.5 適用の全経路配線済み）

Phase 7.5 で追加承認済み（ADR-0009・実装済み）:
  星系間ナビゲーション（StarSystemId / JumpGateId / StarSystemDef / JumpGateDef,
  JumpCommand, JumpGateUsed / StarSystemChanged イベント,
  star_map.rs 静的トポロジー, Godot クライアント配線（J キー）まで完了）

Phase 7.5 で追加承認済み（ADR-0015・実装済み）:
  アプローチ（半自動操船・ApproachComp / ApproachCommand / ApproachTarget,
  対象は Ship または Jump Gate（ゲートは activation_radius 内で停止）,
  Tick Step 2.5 process_approach, Move/Stop で解除, 新イベントなし,
  Godot クライアント配線（クリックで船 / ゲート選択 + A キー）まで完了）

戦闘の深み（ADR-0016 §5）で追加承認済み（ADR-0022・実装済み）:
  intra-Sector ワープ（短距離 Fold = 「逃がさない」の前提・WarpCommand / WarpComp /
  WarpPhase::Aligning|Warping, Tick Step 2.6 process_warp, can_propose_warp 検証,
  Move/Stop は align 中のみ解除（warping は committed）, 新イベントなし（VelocityChanged で記録）,
  Godot クライアント配線（ゲート選択 + W キー）まで完了。lore: 短距離 Fold = ワープ）

戦闘の深み（ADR-0016 §5）で追加承認済み（ADR-0023・実装済み）:
  Propulsion Physics — 慣性モデル（EVE 式指数接近。ShipBaseStats の thrust_magnitude 廃止→
  mass / inertia_modifier 導入。MovementSystem を exponential approach モデルに置換。
  Afterburner 対応の StatDelta 拡張: speed_multiplier / mass_add。
  WarpComp に auto_jump: bool フラグ追加。JumpCommand が gate 射程外の場合は
  apply_warp_command(auto_jump=true) でワープ開始 → ワープ完了時に pending_auto_jumps へ push →
  drain_pending_auto_jumps() でサーバーが Raft へ Transit 提案（auto-warp-then-jump）。
  Godot クライアント配線（J キー優先順位修正・ワープ到着スナップ）まで完了）

戦闘の深み（ADR-0016 §5）で追加承認済み（ADR-0024・実装済み）:
  Tackle — Fold Disruptor モジュール（TackledComp / TackleApplied / TackleReleased イベント,
  ModuleKind::Tackle, Tick Step 4.5 process_tackle, can_propose_warp / can_propose_jump 拒否,
  スナップショット永続化（INV-002）, data/modules.toml Fold Disruptor I（id=12）配線済み）

追加承認済み（ADR-0025・実装済み）:
  天体（恒星・惑星）— CelestialBodyId / CelestialBodyKind / CelestialBodyDef,
  WarpTarget::Body 対応（BODY_WARP_ARRIVAL_FACTOR = 1.5）, celestial_bodies_in_sector(),
  静的マップデータ（3星系 × 恒星1 + 惑星1, 1 unit = 10,000 km → 1 AU ≈ 15,000 units）,
  space_sky.gdshader 太陽ディスク・sun_direction uniform,
  Godot クライアント配線（天体クリック選択 + W キー天体ワープ / sun_direction 毎フレーム更新）まで完了）

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
→ 詳細設計は docs/tick-model.md §8 を参照。

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
            └── dawn-actor          ← Actor基盤（ReplicationBus, ClientConnection）
                    ↑
                    └── dawn-simulation  ← 実行バイナリ・負荷生成
                        （dawn-consensus にも依存する）

# 将来追加予定（まだ存在しない）:
#   dawn-actor ← dawn-replication（追記ログのゴシップ配布・ADR-0021）
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
現在の実装（Phase 7 完了・ADR-0014）:
  1. Tick カウンタをインクリメント
  2. コマンドキューを処理する
       MoveCommand              → ThrustComp.direction を更新（is_braking = false）
       StopCommand              → ThrustComp.is_braking = true（逆推力で減速停止）
       LockOnCommand            → LockSystem に渡す
       ActivateModuleCommand    → FittedSlot.is_active = true / apply_fitting()
       DeactivateModuleCommand  → FittedSlot.is_active = false / apply_fitting()
       JumpCommand              → can_propose_jump() 検証後、TransitOp::Request
                                  （gate_id 付き）を Raft に提案（ADR-0009）
       ApproachCommand          → ApproachComp を付与（半自動操船 / ADR-0015）
                                  対象は Ship または Jump Gate。Move / Stop で解除
       WarpCommand              → can_propose_warp() 検証後 WarpComp を付与
                                  （intra-Sector 短距離 Fold = ワープ / ADR-0022）
                                  Move / Stop は align 中なら解除・warping 中は無視
       JumpCommand（射程外）    → apply_warp_command(auto_jump=true) で WarpComp 付与
                                  （射程内なら即 Raft 提案。auto-warp-then-jump / ADR-0023）
       ※ Transit 中（TransitState::InTransit）の Ship への Move / Stop /
         二重 Transit / Jump / Approach / Warp は拒否する（ADR-0014 / §5）
  2.5 Approach System を実行する            ← Movement の前（ADR-0015）
       ApproachComp を持つ Ship のみ対象。対象（Ship / Jump Gate）の位置へ
       thrust を向け直す。到着半径まで詰めたら is_braking = true で停止保持。
       Ship 対象が消失したら ApproachComp を除去し is_braking = true でブレーキ。
  2.6 Warp System を実行する                ← Approach の後・Movement の前（ADR-0022）
       WarpComp を持つ Ship のみ対象。Aligning は gate へ加速し、ゲート方向の速度が
       max_speed×75% に達したら Warping へ遷移（EVE 準拠・整列時間は機動性次第。中断可・Tackle 窓）。
       Warping は warp 速度で gate へ直進、残距離比例で減速し activation_radius×0.8 内で停止。
       warping 中の船は Movement がスキップ（warp 速度をクランプしない）。VelocityChanged を発行。
       auto_jump=true の場合、到着時に (ship_id, gate_id) を pending_auto_jumps へ push。
       呼び出し元は drain_pending_auto_jumps() でキューを取り出し Raft へ提案する（ADR-0023）。
  3. Movement System を実行する（ECS バッチ処理・warping 中の船はスキップ）
  4. Capacitor System を実行する           ← Movement の後
       毎 Tick: cap を recharge_per_tick 分回復
       cycle_remaining == 0 → 新サイクル: cap 消費 / cap 不足 → 強制 OFF
       cycle_remaining > 0  → デクリメント
       武器モジュールのサイクル開始 → weapon_cycles_started に ship_id を追加
       → 生成: Vec<ModuleDeactivated>（cap 枯渇時のみ）
  4.5 Tackle System を実行する            ← Capacitor の後・Lock の前（ADR-0024）
       アクティブな Tackle モジュール（ModuleKind::Tackle、cap ON）を持つ Ship のみ tackler。
       ロック済みターゲットが tackle_range 以内にいれば TackledComp に tackler を追加。
       射程外・ロック消失・tackler 破壊の場合は tackler を除去して TackleReleased を発行。
       TackledComp を持つ Ship は can_propose_warp / can_propose_jump が false を返す。
       → 生成: Vec<TackleApplied | TackleReleased>（process_tackle() — HashMap desired-state diff）
  5. Lock System を実行する                ← Capacitor の後（位置確定後）
  6. Combat System を実行する              ← Lock の後（Locked 状態を参照）
       weapon_cycles_started に含まれる Ship のみ発射判定（ADR-0012）
       EVE 命中率式: 0.5^((angular/(tracking×sig))² + (max(0,d−opt)/falloff)²)
  7. Bot System を実行する                 ← Combat の後（破壊判定済み後）
       IsBotComp を持つ Ship のみ対象
       apply_*_owned() でプレイヤーと同一パイプラインを使用
  8. 生成されたイベントを EventStore に Append する
  9. ReplicationBus に差分を転送する       ← 必ず 8 の後
  10. RaftActor に TickElapsed を送る       ← election/heartbeat タイマーを
       1 Tick 進める（ADR-0014・INV-005/FBD-003。SectorSimulatorActor 内）
  11. 呼び出し元へ TickResult を返す

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

### フェーズによる適用範囲

このセクションのルールには **プレリリース（現在）** と **リリース以降** の 2 段階がある。

```
プレリリース（Phase 1〜リリース前）:
  永続化されたイベントログを持つ外部ユーザーが存在しない。
  → 破壊的変更（フィールド削除・型変更・イベント削除）を直接行ってよい。
  → Upcaster・V2 命名・Deprecated マークは不要。
  → ただし docs/event-catalog.md と CLAUDE.md は常に実態と合わせること。

リリース以降（本番ログが存在する段階）:
  外部ユーザーのイベントログが存在する。
  → 既存フィールドの変更・削除は Upcaster なしに行ってはならない。
  → 以下「リリース以降の制約」が完全に適用される。
```

**現在は Phase 6（プレリリース）。破壊的変更は許可されている。**

---

### リリース以降の基本原則

**既存の Event フィールドを変更・削除してはならない。**
**新しいフィールドの追加のみが許可される。**

### リリース以降に許可される変更

```rust
// 変更前
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
    pub damage   : f32,
    pub tick     : Tick,
}

// 変更後: 新フィールドの追加は許可（必ず Option にする）
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
    pub damage   : f32,
    pub tick     : Tick,
    pub hit_chance: Option<f32>,  // ← 新フィールドは Option<T> で追加
}
```

### リリース以降に禁止される変更

```rust
// 禁止1: フィールドの削除
pub struct WeaponFired {
    pub ship_id  : ShipId,
    // target_id を削除 ← 禁止。過去のEventのReplayでデシリアライズが失敗する
    pub damage   : f32,
    pub tick     : Tick,
}

// 禁止2: フィールドの型変更
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: u64,   // ShipId → u64 に変更 ← 禁止
    pub damage   : f32,
    pub tick     : Tick,
}

// 禁止3: フィールド名の変更（シリアライゼーションのキーが変わる）
pub struct WeaponFired {
    pub attacker_id: ShipId,  // ship_id → attacker_id に変更 ← 禁止
    pub target_id  : ShipId,
    pub damage     : f32,
    pub tick       : Tick,
}
```

### リリース以降に破壊的変更が必要な場合の手順

```
1. 新しい Event を別名で定義する
   例: WeaponFired → WeaponFiredV2

2. 古い Event を Deprecated としてマークする（削除しない）
   /// @deprecated WeaponFiredV2 を使用すること
   pub struct WeaponFired { ... }

3. Upcaster を実装する
   impl Upcaster for WeaponFired {
       fn upcast(self) -> WeaponFiredV2 { ... }
   }

4. Replay 時に Upcaster を通して新形式に変換する

5. docs/event-catalog.md を更新する

6. 対応する ADR を作成する（既存 ADR の更新ではなく新規作成）
```

### Event Catalog との同期

`docs/event-catalog.md` が Event の唯一の仕様書である。
フェーズにかかわらず、コードの変更と同時に更新すること。

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
□ 既存 Event を変更する場合: リリース済みか確認した
  - プレリリース（現在）→ 破壊的変更を直接行ってよい（Upcaster 不要）
  - リリース以降       → §7「リリース以降に破壊的変更が必要な場合の手順」に従う
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

> 注記（ADR-0017）: ログの圧縮はこれらの禁止メソッドでは**行わない**。
> 圧縮は trait の外側の運用プロセス（検証済みスナップショット背後のセグメントを
> コールドアーカイブへ移送し、ホットログを write-new-then-swap で原子的に切り替える）として
> 実装する。セグメント内のイベントは決して書き換えない。`EventStore` trait は append-only のまま。

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

### FBD-009: スキルポイント育成 / 受動成長 / AFK 採掘の実装

> ゲーム化（ADR-0016）後も **維持** する。反グラインドは "EVE を超える" ための核であり、
> §6 の観測（18k 文書・フォーラム傾向）でも最も嫌われた要素群として現れた
> （フォーラム声は実証ではない — 選択バイアスに留意・eve-reference §11.5）。

```
【スキルポイント / 受動成長】
以下のいかなる形式のスキルポイント制・受動成長も実装してはならない:
  - 時間経過でアンロックされる能力
  - プレイ時間に比例して強くなるパッシブ成長
  - 課金で加速できる育成要素（Pay-to-Win）

理由:
  ゲームの上手さに関係なく、ゲーム時間・課金額で性能が変わる。
  公平感（Perceived Fairness）を根本から損なう時代遅れの設計。

  ※ 「キャラクター」を*エンティティ*として持つことは可（ADR-0016 で解禁）。
    禁止するのは「キャラクターが時間/課金で強くなる育成」であって、存在そのものではない。

【AFK 採掘】
採掘レーザーを起動して放置するコンテンツを実装してはならない。

理由:
  採掘は「放置するだけ」であり、プレイヤーが意図的な判断を下す機会がない。
  EVE では採掘者は「無力な標的」として海賊側のコンテンツとして機能する。
  採掘している人自身はゲームをしていない。

  設計の中心的な問い「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」
  に対して AFK 採掘は No である。

  ※ 「能動的判断を伴う資源獲得」や「資源を消費シンクにして希少性で判断を強制する」設計は
    検討可（ADR-0016 §5・eve-reference §7.4.3）。禁止するのは "放置で進む採取動作" のみ。

  → docs/game-design.md §5 参照
```

### FBD-008: ~~MVP範囲外の実装~~ → 撤廃（ADR-0016）

```
【撤廃】ゲーム化（ADR-0016）に伴い、本禁則は撤廃した。
以下のクレートは ADR 承認のうえ作成してよい:
  crates/dawn-economy/   ← 経済システム
  crates/dawn-character/ ← キャラクター（エンティティ。育成は FBD-009 で引き続き禁止）
  crates/dawn-inventory/ ← インベントリ
  crates/dawn-ui/        ← UI 専用クレート
  crates/dawn-graphics/  ← グラフィックス専用クレート

ただし新規クレートは従来どおりの手続きを踏むこと:
  - 個別 ADR を起票し、人間の承認を得る（§9）
  - Dependency DAG（§3）上の位置を確定し、循環依存を作らない
  - §11 Crate別責務早見表を更新する

Combat / Fitting ロジックは引き続き dawn-ecs / dawn-core 内に実装する
（独立クレートに切り出すなら ADR が必要）。
```

---

## 11. Crate別責務早見表

### 現在存在するクレート

| Crate | 責務 | 依存してよいもの | 禁止 |
|---|---|---|---|
| `dawn-core` | ドメインモデル定義のみ。EntityId, Position, Fitting型, 全Event型, 全Command型 | serde, thiserror のみ | ネットワーク、ファイルI/O、非同期 |
| `dawn-ecs` | ECS World の薄いラッパー。Component定義（Movement/Fitting/Combat）, System定義 | dawn-core, hecs | ネットワーク、EventStore |
| `dawn-event-store` | Event Log の永続化。Append, Read, Snapshot（InMemory + File） | dawn-core, serde | ネットワーク、ECS |
| `dawn-consensus` | Raft実装（ADR-0014）。Leader選出, RaftActor, RaftTransport（In-Process）, PartitionableTransport | dawn-core, serde, rand, tokio | ネットワーク、ECS、EventStore |
| `dawn-actor` | Actor基盤。ReplicationBus（マルチノード収束テスト用スタンドイン）, ClientConnection trait（+ InProcessConnection / WsClientConnection 実装） | dawn-core, dawn-event-store, tokio | dawn-ecs, dawn-simulation |
| `dawn-simulation` | 実行バイナリ。SimulationNode, MultiNodeCluster（RaftActor 配線含む）, WsServer（Godot WebSocket接続）, 負荷生成, DataLoader（TOML読み込み） | 上記全て + rand + tokio-tungstenite + toml | — |

### 将来追加予定のクレート（まだ存在しない・実装しないこと）

| Crate | 予定フェーズ | 責務（予定） |
|---|---|---|
| `dawn-replication` | Phase 8D | 追記ログのゴシップ配布（ADR-0021）。差分伝播 + アンチエントロピー + スナップショット転送（競合解決 CRDT/LWW は単一所有のため不要） |
| `dawn-sector-node` | Phase 8D | 本番実行バイナリ。Actorの配線と起動、ネットワーク RaftTransport + ゴシップの配線。ワイヤ = postcard 再利用（protobuf/`dawn-proto` は不採用・§3 参照） |

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
  ※ Raft は Phase 7（ADR-0014）で実装済み。Transit は Raft Log 経由で動作する。
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

*最終更新: 2026-06-18（ADR-0024 Tackle / ADR-0025 天体 実装反映 — §1 スコープに ADR-0024/0025 追記・フェーズ表記更新。人間承認済み）*
*対応ADR: ADR-0001 〜 ADR-0025（ADR-0020 Simulation LoD は deferred）*
*次回レビュー予定: Phase 8D（分散インフラ）設計時 / Signature Resolution 着手時*
