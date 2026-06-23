---
scope    : 完了済みフェーズ（Phase 0〜7）の詳細記録、および廃止・変更された計画の記録
audience : AI Agent / Human Developer
update   : 通常は更新しない（完了済みフェーズの確定記録）。新たにフェーズが完了したら
           docs/process/roadmap.md の §2「完了済みフェーズ」要約に追記後、本ファイルへ詳細を移すこと
related  : docs/process/roadmap.md（現在地・進行中フェーズはこちらが正典）
---

# Roadmap History

`docs/process/roadmap.md` から分離した完了済みフェーズの詳細記録（ADR-0030 と同じ理由 — 当時の
判断根拠・計測値は時々参照する程度で、常時ロードする roadmap.md 本体には不要）。

現在地・進行中フェーズ（Phase 8 以降）は `docs/process/roadmap.md` を参照すること。
このファイルは確定済みの過去の記録であり、通常は更新しない。

---

## Phase 0 — 基盤確立 ✅

**完了基準:** `cargo test --workspace` がゼロエラーで通過する → **達成**

| タスク | 状態 | 備考 |
|---|---|---|
| Cargo Workspace 初期化 | ✅ 完了 | |
| `dawn-core` 全型定義 + テスト | ✅ 完了 | 17 テスト |
| `dawn-event-store` InMemoryEventStore | ✅ 完了 | 8 テスト |
| `dawn-ecs` SimWorld + MovementSystem | ✅ 完了 | 11 テスト |
| `dawn-simulation` SimulationNode + Spawner | ✅ 完了 | 13 テスト |
| CLAUDE.md 初版 | ✅ 完了 | |
| docs/ 設計ドキュメント群 | ✅ 完了 | |
| Rust インストール + ビルド確認 | ✅ 完了 | rustc 1.96.0 |
| `cargo test --workspace` 通過 | ✅ 完了 | 49 テスト全パス |

---

## Phase 1 — Single Node シミュレーション検証 ✅

**完了基準:** 10,000 ships が 1 Tick を 16ms 以内に処理できることを計測で確認する → **達成**

| タスク | 状態 | 備考 |
|---|---|---|
| `cargo run --release` でベンチマーク実行 | ✅ 完了 | |
| Tick 処理時間の計測と目標達成確認 | ✅ 完了 | max 11,847 µs ≤ 16,000 µs |
| P95 計測値の記録 | ✅ 完了 | 4,313 µs |
| Event Log の増加ペース確認 | ✅ 完了 | 10,000 events / tick |

### 計測結果（記録）

```
環境     : Windows / rustc 1.96.0 / --release
ships    : 10,000
ticks    : 100

min      :    734 µs
mean     :  1,687 µs
p95      :  4,313 µs
max      : 11,847 µs  ✓ ≤ 16,000 µs

throughput        : 約 592万 events/sec
move events/tick  : 10,000
total events      : 1,010,000（spawn 10,000 + move 1,000,000）
```

---

## Phase 2 — In-Memory Multi-Node ✅

**完了基準:** 3 つの論理ノードが In-Memory Channel 経由でイベントを同期し、
全ノードのイベントログが一致することをテストで確認する → **達成**

| タスク | 状態 | 備考 |
|---|---|---|
| `dawn-actor` クレート作成（ClientConnection 境界） | ✅ 完了 | ReplicationBus は 8D-2a で dawn-replication へ移動 |
| `SectorSimulatorActor` 実装 | ✅ 完了 | dawn-simulation 内 |
| `EventStoreActor` 実装 | 🗑️ 削除 | wire されないまま残っていたため削除（`SimulationNode` が EventStore を直接所有） |
| ノード間 In-Memory Channel 接続 | ✅ 完了 | 単一チャンネル設計で決定論的 |
| 3 ノード整合性テスト | ✅ 完了 | 65 テスト全パス |

### 整合性テスト結果（記録）

```
3 nodes × 1,000 ships × 20 ticks
replicated : 63,000 events
expected   : 63,000 events  ✓ PASS（sleep・flush・バリアなし）
```

---

## Phase 3 — Event 永続化 ✅

**完了基準:** ノードを再起動した後、Snapshot + Event Replay によって
シャットダウン直前の Ship 状態が完全に復元される → **達成**

| タスク | 状態 | 備考 |
|---|---|---|
| ファイルベース EventStore 実装 | ✅ 完了 | `FileEventStore`（length-prefix + postcard） |
| Snapshot 取得ロジック | ✅ 完了 | `SimulationNode::take_snapshot()` |
| Snapshot からの State 復元 | ✅ 完了 | `SimulationNode::restore_from()` |
| 再起動後の整合性テスト | ✅ 完了 | tick / ship count / positions 全一致 |

### 計測結果（記録）

```
100 ships × 10 ticks
Session 1: Tick 5 でスナップショット（log_index=600）→ Tick 10 まで継続（1100 events）
Session 2: restore_from() で復元 → tick / ship count / positions ✓ PASS
テスト総数: 73/73
```

---

## Phase 4 — ゲーム開発ループ（反復開発） ✅

**構造:** ウォーターフォール的な「完了」を定めず、
サーバー機能追加 → クライアントで確認 → フィードバック → 次の機能
という短いサイクルを繰り返し、ゲームとして「満足できる」状態になったら Phase 5 へ進む。

**ネットワークはダミーのまま維持する。**
本物のネットワーク（gRPC/QUIC）は Phase 5 で一括対応する。
→ `ClientConnection` trait の差し替えだけで完結するよう設計する（ADR-0005）

**Phase 4 卒業基準（ADR-0007 §6 より）:**

```
□ 2クライアントが同時に接続できる
□ 両クライアントの世界状態が同期している（VelocityChanged が両方に届く）
□ プレイヤーのロックオン操作が機能する
□ 再接続後に InitialState で状態が復元される
□ 基本的なゲームループ（移動・ロック・戦闘）でクラッシュしない
```

### Phase 4 前提作業（初回のみ・サイクル開始前）

| タスク | 状態 | 備考 |
|---|---|---|
| `ClientConnection` trait 定義 | ✅ 完了 | Event ストリーム + Command の2方向のみ（ADR-0005） |
| `InProcessConnection` 実装 | ✅ 完了 | In-Memory Channel 直結（6テスト）|
| Godot 4 プロジェクト初期化 | ✅ 完了 | `client/` ディレクトリ + WebSocket サーバー |

### ClientConnection 抽象化

```
サーバー側                       クライアント側
────────────────────────         ─────────────────
dawn-replication bus             Godot シーン
    ↓                                ↑
ClientConnection trait  ─────────────
    ├── InProcessConnection  ← テスト用（チャンネル直結）
    └── WsClientConnection   ← 本番（WebSocket・ADR-0007）

Godot は trait に向かって書く。
差し替え時に Godot 側のコードは変更しない。
```

> 注: 当初計画の `GrpcConnection`（Phase 5）は ADR-0007 で不採用。本番転送は
> WebSocket（`WsClientConnection`）。gRPC は Phase 9 以降に再検討。

### Cycle 1 — 宇宙に船を浮かべる ✅

```
目標 : 宇宙空間で Ship が動いているのが見える
Server: WsServer（WebSocket）/ 200 ships / 10 tick/sec
Client: Godot 4 初期化 / Ship を 3D 空間に表示（六角柱 + OmniLight）
確認  : 「宇宙に船がいる」という感覚 → 達成
```

### Cycle 2 — 航行する ✅

```
目標 : 宇宙空間を飛び回れる
Server: 加速度ベースの物理（ThrustComp + ShipStatsComp）
        MoveCommand で ThrustComp を設定
        速度上限（max_speed）・加速度（thrust_magnitude）をコンポーネント化
        壁の削除（宇宙は無限）
Client: 左ダブルクリック → カメラレイ方向に推力ベクトルを指定
        クォータニオンオービットカメラ（上下左右全方向・ジンバルロックなし）
        速度インジケーター（緑矢印）/ 推力インジケーター（橙矢印）
        HUD（速度 / Tick / 接続状態）
確認  : 「宇宙の広さ」「3D 方向への加速」が感じられる → 達成
```

### Cycle 3 — 戦う ✅

```
目標 : 船同士が戦えて破壊される

サーバー側（完了）:
  Fitting システム（EVE Online 準拠）
    - モジュール装備スロット（High / Mid / Low / Rig）
    - StatDelta による stat 集計（base_stats + Σmodule.delta）
    - 武器能力はモジュール装備でのみ付与（ベース値ゼロ）
  Lock-on システム（2フェーズ戦闘）
    - LockOnCommand → LockSystem でカウントダウン → TargetLocked
    - NPC 自動ロック / プレイヤー右クリックロック
    - lock_time / max_locks を ShipStatsComp で管理（モジュールで変更可能）
  Combat システム
    - Locked 状態のターゲットにのみ発射（ロックなし = 攻撃不可）
    - WeaponFired / DamageTaken / ShipDestroyed イベント
  ClientCommand 一般化（MoveCommand → ClientCommand enum）
    - ws_server.rs で MoveCommand / LockOnCommand 両方をパース

サーバー側追加（Cycle 3 後半）:
  Active / Passive モジュール区別（ADR-0006）
    - FittedSlot { def, is_active } で活性化状態管理
    - NPC は Active モジュールを自動 ON、プレイヤーは手動
  VelocityChanged（ADR-0008）
    - 位置は派生状態、速度変化のみをイベント記録
    - 物理ロジック変更に対して Replay が堅牢
  Phase 5 マルチプレイヤー基盤
    - PlayerId / Hello-Welcome ハンドシェイク
    - InitialState / PlayerFitting 送信
    - 複数クライアント同時接続

Godot 側（実装済み）:
  HP ゲージ HUD / 破壊エフェクト / ロック枠線 / 被弾フラッシュ
  F1〜F8 キーで Active モジュールをオン/オフ
  宇宙背景（手続き生成スカイシェーダー・天の川帯）
  フレームごとの velocity 積分による滑らかな動き

確認  : 「戦闘が面白い」という感覚があるか → ✅ OK
```

### Cycle 4 — 船種拡張とバランス外部化 ✅

```
目標: 船種を増やし、リビルドなしでバランス調整できる仕組みを導入する

サーバー側（完了）:
  data_loader（TOML ローダー）
    - data/ship_types.toml: 船種バランスデータ（6種）
    - data/modules.toml: モジュールデータ（11種）
    - ファイル不在時は built-in デフォルトへフォールバック
  新船種: NPC Destroyer / NPC Cruiser / Player Destroyer / Player Cruiser

バランス調整サイクル:
  data/*.toml を編集 → サーバー再起動のみ（リビルド不要）
```

### Cycle N — フィードバック次第で追加

```
採掘 / 資源 / 市場 / 陣営 / ...
各サイクルの内容は直前の確認フィードバックに基づいて決める
```

---

## Phase 5 — マルチプレイヤー基盤 ✅

**設計変更（ADR-0007）:** gRPC への移行は行わず WebSocket + JSON を維持する。
Godot 側のコードは変更しない。gRPC は Phase 9 以降で再検討する。

**完了基準:** ADR-0007 実装チェックリスト全完了 → **達成**

| タスク | 状態 | 備考 |
|---|---|---|
| `PlayerId` 型 + `DawnError::NotOwner` | ✅ 完了 | |
| `spawn_player_ship` + 全コマンド所有権チェック | ✅ 完了 | |
| Hello / Welcome / InitialState ハンドシェイク | ✅ 完了 | ADR-0007 §2-4 |
| PlayerSession / 複数クライアント同時接続 | ✅ 完了 | |
| `AttackCommand` JSON パーサー | ✅ 完了 | 現在は自動戦闘が主体 |
| Godot 側: Hello 送信 / Welcome 受信 | ✅ 完了 | |
| 全テスト通過 | ✅ 完了 | 138テスト |

---

## Phase 7 — 分散コンセンサス（Raft）✅

設計: [ADR-0014](../adr/ADR-0014-raft-consensus.md)（accepted）
完了基準: ノード障害後に Sector Transit が正しく完了する → **達成**
（`transit_completes_after_a_new_leader_is_elected_during_node_failure` で検証）
★ ADR-0009（星系間ナビゲーション）はこのフェーズ完了後に実装する

実装順序（ADR-0014 実装チェックリストに基づく）:

| # | タスク | クレート | 状態 |
|---|---|---|---|
| 1 | `SectorTransitRequested` / `Completed` / `Aborted` イベント + `TransitCommand` | dawn-core | ✅ |
| 2 | 状態機械（Follower / Candidate / Leader）+ 単体テスト | dawn-consensus（新規） | ✅ |
| 3 | RequestVote / AppendEntries 処理 + Tick 駆動タイマー | dawn-consensus | ✅ |
| 4 | `RaftActor`（Mailbox 経由）+ `RaftTransport` / `PartitionableTransport` | dawn-consensus | ✅ |
| 5 | `TransitState` コンポーネント + Transit 中の操作拒否 | dawn-ecs | ✅ |
| 6 | `SimulationNode` の Transit 処理（Step 7.5 / Step 10 組み込み） | dawn-simulation | ✅ |
| 7 | `MultiNodeCluster` への RaftActor 配線 | dawn-simulation | ✅ |
| 8 | シナリオテスト（正常系 / リーダー障害 / スプリットブレイン不在 / INV-002 Replay） | dawn-simulation | ✅ |
| 9 | Transit レイテンシのベンチマーク（benchmark-baseline.md 追記） | dawn-simulation | ✅ |
| 10 | ドキュメント更新（event-catalog / tick-model / CLAUDE.md ※要人間承認） | docs | ✅ |

---

## 廃止・変更された計画の記録

### 2026-06-14: Phase 8 の前提を 3 つの設計判断で変更（ADR-0016/0017/0018）

**変更 1 — 絶対アンチ TiDi を撤回（ADR-0018）:**

旧: Phase 8 完了基準「1 Sector 5,000 ships 上限で Tick SLA を常に満たす」＝
TiDi を一切出さず入場制限で事前規制する。
新: 単一密戦闘は分割不能なため入場制限だけだと「クライマックスから締め出す」＝ EVE より悪い体験に
なりうる。過負荷は **分割 → LoD → 局所 TiDi → 入場制限** の順で対処し、TiDi は局所・観測可能・
非破壊・自動回復・後置の境界つき最終手段として採用する。完了基準も置換（roadmap.md §10）。

理由: eve-reference §11.1 の批判。TiDi は INV-005 を壊さない（論理 Tick は単調・決定的のまま）ため
採用はクリーン。差別化は「TiDi が無い」ではなく「閾値が桁違いに高く局所・短時間・自動回復」。

**変更 2 — スナップショット圧縮・2層ログを Phase 8 に追加（ADR-0017）:**

旧: スナップショットは最適化に過ぎず、ログは index 0 から常に replay 可能（無限成長を許容）。
新: FBD-001 + INV-001 + INV-002 は長寿命シャードで両立しないため、2層ログ（圧縮可能なホットログ +
永久 append-only のコールドアーカイブ）を導入。INV-002 を「検証済みスナップショット + 末尾 replay」に
改訂。failover が創世記 replay を要求しない前提に。Phase 8A として最優先で実装する。

**変更 3 — マルチ Raft でのコンセンサス・スケールを却下（ADR-0017 §5）:**

旧（検討案 D）: 境界ごとのマルチグループ Raft で transit をスケール。
新: メンテナンス不能な複雑さ（クロスグループ 2PC 等）のため却下。単一 Raft グループを意図的に維持。
唯一の単純な備えはバッチ提案で、fleet-jump レイテンシが実測で問題化してから着手（roadmap.md §10 8E）。

### 2026-06-04: Phase 4〜11 の開発戦略を変更（2段階）

**変更 1 — クライアント優先（ネットワーク後回し）:**

旧: Phase 4 = gRPC/QUIC → Phase 7 = クライアント
新: Phase 4 = Godot + ダミーネットワーク → Phase 5 = 本物のネットワーク

理由: ゲーム体験を先に確立してからネットワーク化する。
`ClientConnection` trait により差し替えコストを最小化。

**変更 2 — Phase 4 と「ゲーム体験フェーズ」を反復ループに統合:**

旧: Phase 4（クライアント起動）→ Phase 5（ゲーム体験）の順に完了
新: Phase 4 = 反復サイクル（Cycle 1〜N）をゲーム体験が満足できるまで繰り返す

理由: サーバー機能追加 → クライアント確認 → フィードバック のループで
品質を高める。フェーズの境界よりサイクルの反復を優先する。
