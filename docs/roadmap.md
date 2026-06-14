---
scope    : 何を・どの順番で・なぜその順番で作るか。現在地と次のステップの明示
audience : AI Agent / Human Developer
update   : フェーズ完了時 / タスクが完了するたびに更新する
related  : architecture.md, CLAUDE.md §1
---

> **CLAUDE.md レビュータイミング**
> このファイルの各フェーズ完了マーク（✅）を更新するタイミングで
> `CLAUDE.md` のレビューも実施すること。
>
> | タイミング | レビュー内容 |
> |---|---|
> | Phase 4 完了時（Phase 5 移行前） | スコープ・Crate表・Tick順序・パターン集を全面見直し |
> | Phase 5 完了時 | ClientConnection 差し替え後の設計原則を更新。ADR-0007 実装チェックリストを消化してから着手すること |
> | Phase 7 完了時 | Raft 導入後の INV-003 / INV-005 の具体例を更新 |
>
> CLAUDE.md フッターの `次回レビュー予定` と必ず一致させること。

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
現在のフェーズ : Phase 7.5 完了 — ADR-0009（星系間ナビゲーション）実装完了
フェーズの状態 : サーバー側 + Godot クライアント配線完了（241テスト）
                — dawn-core 型定義、Jump Raft パイプライン
                （star_map / TransitOp gate_id / Step 7.5）、
                ClientCommand::Jump、ws_server の EventJson + JSON パーサー、
                connection.gd の send_jump_command、main.gd の
                JumpGateUsed / StarSystemChanged 処理（瞬間移動・HUD通知・J キー）。
                次のフェーズ: Phase 8（スケール基盤 / 持続性）着手。
                2026-06-14 の設計変更（ADR-0017 スナップショット圧縮 / ADR-0018 局所 TiDi /
                マルチ Raft 却下）を Phase 8 タスクに反映済み（§10）。
                Sector キャパシティの悪用対策は docs/game-design.md §8 を参照。
```

### 完了済みフェーズ

- ✅ Phase 0 — 基盤確立（`cargo test --workspace` 73テスト全パス）
- ✅ Phase 1 — Single Node シミュレーション検証（max 11,847 µs ≤ 16,000 µs 目標達成）
- ✅ Phase 2 — In-Memory Multi-Node（3ノード 63,000イベント整合性 ✓）
- ✅ Phase 3 — Event 永続化（Snapshot + Replay 再起動後の状態完全復元 ✓）
- ✅ Phase 4 — ゲーム開発ループ（Cycle 1〜3 完了 / 卒業基準 5/5 達成）
- ✅ Phase 5 — マルチプレイヤー基盤（ADR-0007 チェックリスト全完了 / 138テスト全パス）
- ✅ Phase 6 — ゲームループ改善（Capacitor / EVE命中率式 / タクティカルオーバーレイ / ボットAI / 154テスト全パス）
- ✅ Phase 7 — 分散コンセンサス（Raft / ADR-0014 / リーダー障害中の Transit 完遂を検証 / 223テスト全パス）
- ✅ Phase 7.5 — 星系間ナビゲーション（ADR-0009 / Jump Gate Raft パイプライン + Godot クライアント配線 / 241テスト全パス）

### Phase 4 卒業記録（ADR-0007 §6）

```
✅ 2クライアントが同時に接続できる
✅ 両クライアントの世界状態が同期している
✅ プレイヤーのロックオン操作が機能する
✅ 再接続後に InitialState で状態が復元される
✅ 基本的なゲームループでクラッシュしない
```

### Phase 5 完了記録（ADR-0007 実装チェックリスト）

```
✅ dawn-core: PlayerId(u64) 型追加
✅ dawn-core: DawnError::NotOwner 追加
✅ dawn-simulation/node.rs: player_ships HashMap / spawn_player_ship / 全コマンド所有権チェック
✅ dawn-simulation/ws_server.rs: Hello/Welcome/InitialState ハンドシェイク
✅ dawn-simulation/ws_server.rs: PlayerSession 構造体 / 複数クライアント同時接続
✅ dawn-simulation/ws_server.rs: AttackCommand JSON パーサー追加
✅ dawn-simulation/main.rs: ORIGIN シグナル処理を削除
✅ connection.gd: Hello 送信 / Welcome 受信 / InitialState 受信
✅ main.gd: ORIGIN シグナル送信削除 / Welcome シグナル処理
✅ 138テスト全パス
```

### 次に着手すべきタスク

**次の単一タスク: Phase 8A-6 — event-catalog.md / architecture.md に 2 層ログを反映（docs）。**
（8A-1〜8A-5 完了: スナップショット検証 2 テスト + take_snapshot 正準ソート、FileEventStore の
2 層ログ（base_index ヘッダ）+ `compact()`（コールドアーカイブ + 原子的 swap）+ 4 テスト、
圧縮後 reopen + restore で「創世記 replay 不要」を実証する failover テスト。全 workspace グリーン。
残り 8A: 6=docs 反映、7=圧縮の自動トリガ・オーケストレーション。）
理由: 機構とテストが揃ったので docs を実態に合わせ、その後 8A-7（実運用で圧縮を走らせる）へ。
設計トラックとして §8C-1（空間索引 + AoI の新規 ADR 起票）は並行可。
Phase 8 全体のタスク内訳は §10 を参照。
CLAUDE.md フッターの「次回レビュー予定」は 2026-06-14 に
「空間索引 + AoI ADR 起票時」へ更新済み（ADR-0017/0018 適用に伴う）。

#### Phase 6 完了タスク一覧

| 優先度 | タスク | 状況 | 理由 |
|---|---|---|---|
| ✅ 完了 | Capacitor 実装 | サイクルベース cap 管理まで完了（ADR-0011） | 「常時 ON で勝ち」問題の解消 |
| ✅ 完了 | セッションメトリクス出力 | --duel モード限定で実装済み（勝敗・経過Tick・cap枯渇回数をstdout出力） | 数値でバランスを判断できるようにする |
| ✅ 完了 | Godot: cap バー表示 | ProgressBar ウィジェット実装済み（青色バー + GJ表示） | cap 状態の視覚フィードバック |
| ✅ 完了 | EVE 命中率式（ADR-0012） | tracking/falloff/sig_radius 追加。hit_chance = 0.5^(追跡項²+射程項²) | ポジション管理が実質的な意味を持つ |
| ✅ 完了 | タクティカルオーバーレイ（ADR-0013） | Tab キーで射程リング（緑:最適/橙:フォールオフ）を表示 | 距離と射程の視覚的フィードバック |
| ✅ 完了 | StopCommand（S キー） | 逆推力で減速停止。ボット AI にも使用 | 精密なポジション制御を可能にする |
| ✅ 完了 | ボット AI 改善 | 射程内停止・ロックキュー・スポーン位置修正 | デュエルが成立するようにする |

---

## 3. Phase 0 — 基盤確立 ✅

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

## 4. Phase 1 — Single Node シミュレーション検証 ✅

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

## 5. Phase 2 — In-Memory Multi-Node ✅

**完了基準:** 3 つの論理ノードが In-Memory Channel 経由でイベントを同期し、
全ノードのイベントログが一致することをテストで確認する → **達成**

| タスク | 状態 | 備考 |
|---|---|---|
| `dawn-actor` クレート作成（Actor 基盤） | ✅ 完了 | EventStoreActor, ReplicationBus |
| `SectorSimulatorActor` 実装 | ✅ 完了 | dawn-simulation 内 |
| `EventStoreActor` 実装 | ✅ 完了 | |
| ノード間 In-Memory Channel 接続 | ✅ 完了 | 単一チャンネル設計で決定論的 |
| 3 ノード整合性テスト | ✅ 完了 | 65 テスト全パス |

### 整合性テスト結果（記録）

```
3 nodes × 1,000 ships × 20 ticks
replicated : 63,000 events
expected   : 63,000 events  ✓ PASS（sleep・flush・バリアなし）
```

---

## 6. Phase 3 — Event 永続化 ✅

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

## 7. Phase 4 — ゲーム開発ループ（反復開発）

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

---

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
ReplicationBus                   Godot シーン
    ↓                                ↑
ClientConnection trait  ─────────────
    ├── InProcessConnection  ← Phase 4（チャンネル直結）
    └── GrpcConnection       ← Phase 5（本物のネットワーク）

Godot は trait に向かって書く。
差し替え時に Godot 側のコードは変更しない。
```

---

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

## 8. Phase 5 — マルチプレイヤー基盤 ✅

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

## 9. Phase 7 — 分散コンセンサス（Raft）✅

設計: [ADR-0014](./adr/ADR-0014-raft-consensus.md)（accepted）
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

## 10. Phase 8 — スケール基盤 / 持続性（ADR-0017 / ADR-0018）

> 本フェーズは 2026-06-14 の設計変更を反映して詳細化した（旧版は「方向性のみ」だった）。
> 対応 ADR は **方針確定済み・コード実装は本フェーズで行う**。
> 関連: ADR-0017（スナップショット圧縮・2層ログ）, ADR-0018（局所 TiDi）, docs/tick-model.md §8,
> docs/game-design.md §8, docs/reference/eve-reference.md §8–§11。

**完了基準（ADR-0018 で更新。旧「5,000 ships 上限で常に SLA」は撤回）:**

- 通常負荷では論理 Tick が一定で SLA（≤32ms）を満たす。
- 空間的に分離可能な負荷は動的分割で**劣化ゼロ**に捌ける。
- 分割不能な単一密戦闘がノード容量を超えたら、当該 Sector **局所**の TiDi で graceful に劣化し、
  dilation 係数を SLA メトリクスに記録、負荷減で 1.0 に**自動回復**する（イベントの並べ替え・欠落なし）。
- 入場制限は**最終バックストップ**としてのみ発動する。
- **創世記 replay なし**で failover / 再起動できる（最新スナップショット + 末尾 replay）。

### 8A. イベントログの持続性（ADR-0017）— 最優先（正しさ / 運用性）

| # | タスク | クレート | 状態 |
|---|---|---|---|
| 1 | **スナップショット検証テスト**: ① round-trip（snapshot→restore→snapshot バイト一致）② snapshot + 末尾 Tick == live（cap/hull 含む） | dawn-simulation | ✅ take_snapshot 正準ソート + 2テスト |
| 2 | ホットログのセグメント化（base_index ヘッダ）+ `compact()` 機構 | dawn-event-store | ✅ FileEventStore.compact + 4テスト |
| 3 | コールドアーカイブ書き出し（append-only）+ 原子的 swap（write-new-then-swap） | dawn-event-store | ✅ compact() 内で実装（header に base を埋め rename 一発で原子的） |
| 4 | failover / 再起動が創世記 replay を要求しないテスト（ADR-0014 連携） | dawn-simulation | ✅ 圧縮後 reopen + restore テスト |
| 5 | snapshot.rs のドキュメントコメントを改訂後 INV-002 に更新 | dawn-simulation | ✅（228f244） |
| 6 | event-catalog.md / architecture.md に2層ログを反映 | docs | ⬜ |
| 7 | 圧縮の自動トリガ（ノードのスナップショット周期 → `compact()` 呼び出しのオーケストレーション） | dawn-simulation | ⬜ |

### 8B. 負荷制御 / Anti-TiDi（ADR-0018 + 既存方針）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | Sector Population Cap（**最終バックストップに格下げ**） | game-design.md §8 | ⬜ |
| 2 | Dynamic Sector Fission（分離可能負荷の第1手） | tick-model.md §8 | ⬜ |
| 3 | Simulation LoD（忠実度の階層化・更新間引き） | game-design.md §8 層1 | ⬜ |
| 4 | 局所 TiDi: dilation = 実時間ペーシングのみ・論理 Tick の処理内容は不変（テスト） | INV-005 と無関係 | ⬜ |
| 5 | dilation が当該 Sector 局所であること（隣接へ伝播しない）のテスト | INV-TiDi (a) | ⬜ |
| 6 | SLA イベント / メトリクス（dilation 係数・継続時間の記録） | INV-TiDi (b) 観測可能 | ⬜ |
| 7 | 負荷減での自動回復（係数 → 1.0）のテスト | INV-TiDi (d) | ⬜ |
| 8 | 差分 TiDi の越境因果ルールを実装 ADR で詰める | ADR-0018 未解決論点 | ⬜ |

### 8C. 空間索引 + AoI（新規 ADR が必要）— TiDi 閾値を上げる本体

> 8C が効くほど 8B-4〜7（TiDi 発動）が稀になる。両者は連動する。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | **新規 ADR 起票**（空間索引 grid/quadtree + Area-of-Interest） | CLAUDE.md「次回レビュー予定」 | ⬜ |
| 2 | Sector 内 空間索引（近傍クエリ O(n²) → O(n·k)） | ターゲティング / AoE / 衝突 | ⬜ |
| 3 | ロックグラフ + 空間索引で全ペア走査を排除 | 密戦闘の天井↑ | ⬜ |
| 4 | AoI: クライアントは relevant 半径内のみ受信（Sector 単位 → グリッド単位へ細分） | 帯域レバー（fb2a484 の発展） | ⬜ |

### 8D. 分散インフラ（物理ノード）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | dawn-proto（シリアライゼーション定義） | 新規クレート（個別 ADR + DAG 位置確定） | ⬜ |
| 2 | dawn-replication（Gossip + CRDT / LWW-Register） | 新規クレート | ⬜ |
| 3 | dawn-sector-node（本番実行バイナリ・ノード間ネットワーク通信） | 新規クレート | ⬜ |
| 4 | （任意・推奨）Raspberry Pi クラスタ実機検証 | 下記 ★ 参照 | ⬜ |

★ 実機検証（任意・推奨）: ネットワークトランスポート実装後、Raspberry Pi クラスタ
（Pi 4/5 推奨。Zero 2 W は aarch64 ビルド可だが 512MB RAM が制約のため数百隻規模に縮小）で
3 ノードを物理的に分離して動作確認する。目的: 実ネットワーク遅延・分断条件下での Raft / Gossip
挙動を実機で検証する（dawn の競争優位＝分散基盤の本番妥当性を確かめる / ADR-0016）。
検証項目: ノード間通信の到達性、ネットワーク分断時の Raft フェイルオーバー、低スペック環境での Tick SLA。

### 8E. Transit consensus（ADR-0017 §5 で方針決定済み）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 単一 Raft グループを維持（実装変更なし） | マルチ Raft はメンテ不能として却下 | ✅ 方針確定 |
| 2 | バッチ提案（fleet jump = N 隻を 1 エントリに束ねる） | fleet-jump レイテンシが実測で問題化したら着手 | ⬜ 保留 |

### Phase 9 以降（方向性のみ）

```
Phase 9 : Resource + Economy Context（dawn-economy / FBD-008 撤廃により ADR で解禁）
Phase 10: Client 本格化（GDExtension 導入）
           godot-rust で Client-Side Prediction を Rust 実装
           dawn-core 型を Godot へ直接公開
           完了基準: レイテンシを隠した滑らかな操作感
```

### クライアント技術スタック（決定済み）

```
エンジン      : Godot 4
ゲームロジック: GDScript（AI が主に書く）
高性能処理    : godot-rust / GDExtension（Phase 10 以降）
サーバー通信  : WebSocket + JSON（Phase 4〜6 で継続使用）
               → gRPC への移行は Phase 9 以降で再検討（ADR-0007）
型共有        : Phase 4〜9: チャンネル / proto 変換
               → Phase 10: GDExtension で dawn-core を直接 import

→ 技術選択の根拠は ADR-0004 を参照
```

### リポジトリ構成（Phase 4 で追加）

```
dawn/                        ← 既存 Cargo Workspace（サーバー）
client/                      ← Godot 4 プロジェクト（Phase 4 で追加）
  project.godot
  scenes/
    main.tscn
    ship.tscn
  scripts/
    connection.gd            ← ClientConnection の GDScript ラッパー
    ship_controller.gd       ← Ship 表示・移動
    skybox.gd
  assets/
    models/                  ← Ship 3D モデル（glTF）
    shaders/                 ← 宇宙エフェクト
  gdextension/               ← Phase 10 以降
    Cargo.toml               ← godot-rust
    src/
      lib.rs                 ← dawn-core を import
```

### フェーズ横断の設計原則

```
ClientConnection trait を正しく定義することがネットワーク差し替えの鍵
各 Server Context は独立した Crate として追加する
上位 Context は下位 Context に依存しない（Spatial ← Navigation ← Combat …）
Anti-TiDi 優先の方針（INV-TiDi 改訂・ADR-0018: TiDi は局所的最終手段）は全フェーズで維持する
Event Sourcing の原則（INV-001〜006）は全フェーズで維持する
```

---

## 11. 廃止・変更された計画の記録

### 2026-06-14: Phase 8 の前提を 3 つの設計判断で変更（ADR-0016/0017/0018）

**変更 1 — 絶対アンチ TiDi を撤回（ADR-0018）:**

旧: Phase 8 完了基準「1 Sector 5,000 ships 上限で Tick SLA を常に満たす」＝
TiDi を一切出さず入場制限で事前規制する。
新: 単一密戦闘は分割不能なため入場制限だけだと「クライマックスから締め出す」＝ EVE より悪い体験に
なりうる。過負荷は **分割 → LoD → 局所 TiDi → 入場制限** の順で対処し、TiDi は局所・観測可能・
非破壊・自動回復・後置の境界つき最終手段として採用する。完了基準も置換（§10）。

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
唯一の単純な備えはバッチ提案で、fleet-jump レイテンシが実測で問題化してから着手（§10 8E）。

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
