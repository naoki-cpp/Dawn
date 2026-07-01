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

## Phase 8 — スケール基盤 / 持続性（詳細記録・8A/8C/8D/8E ほぼ完了）

> 2026-07-02、roadmap.md 本体の分量削減のため詳細タスク表をここへ移設（Phase 0〜7 と同じ理由）。
> 8B のみ「一区切り」で保留項目を残すため、保留項目の要約は roadmap.md §10 に残す。
> 対応 ADR: ADR-0017（スナップショット圧縮・2層ログ）, ADR-0018（局所 TiDi）,
> ADR-0019（AoI 静的セルグリッド）, ADR-0020（Simulation LoD・deferred）, ADR-0021/0027（複製）。
> 関連: docs/architecture/tick-model.md §8, docs/design/game-design.md §8, docs/reference/eve-reference.md §8–§11。

**完了基準（ADR-0018 で更新。旧「5,000 ships 上限で常に SLA」は撤回）:**

- 通常負荷では論理 Tick が一定で SLA（≤32ms）を満たす。
- 空間的に分離可能な負荷は動的分割で**劣化ゼロ**に捌ける。
- 分割不能な単一密戦闘がノード容量を超えたら、当該 Sector **局所**の TiDi で graceful に劣化し、
  dilation 係数を SLA メトリクスに記録、負荷減で 1.0 に**自動回復**する（イベントの並べ替え・欠落なし）。
- 入場制限は**最終バックストップ**としてのみ発動する。
- **創世記 replay なし**で failover / 再起動できる（最新スナップショット + 末尾 replay）。

### 8A. イベントログの持続性（ADR-0017）✅ 完了

| # | タスク | クレート | 状態 |
|---|---|---|---|
| 1 | **スナップショット検証テスト**: ① round-trip（snapshot→restore→snapshot バイト一致）② snapshot + 末尾 Tick == live（cap/hull 含む） | dawn-simulation | ✅ take_snapshot 正準ソート + 2テスト |
| 2 | ホットログのセグメント化（base_index ヘッダ）+ `compact()` 機構 | dawn-event-store | ✅ FileEventStore.compact + 4テスト |
| 3 | コールドアーカイブ書き出し（append-only）+ 原子的 swap（write-new-then-swap） | dawn-event-store | ✅ compact() 内で実装（header に base を埋め rename 一発で原子的） |
| 4 | failover / 再起動が創世記 replay を要求しないテスト（ADR-0014 連携） | dawn-simulation | ✅ 圧縮後 reopen + restore テスト |
| 5 | snapshot.rs のドキュメントコメントを改訂後 INV-002 に更新 | dawn-simulation | ✅（228f244） |
| 6 | event-catalog.md / architecture.md に2層ログを反映 | docs | ✅ §5-C 復旧モデル + §2 永続化モデル追記 |
| 7 | 圧縮の自動トリガ（ノードのスナップショット周期 → `compact()` 呼び出しのオーケストレーション） | dawn-simulation | ✅ `checkpoint()` + `CheckpointScheduler`（checkpoint.rs）+ Phase 3 デモ配線 + 3テスト |

### 8B. 負荷制御 / Anti-TiDi（ADR-0018 + 既存方針）🔶 一区切り（詳細は roadmap.md §10 参照）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | Sector Population Cap（**最終バックストップに格下げ**） | game-design.md §8 | ✅ 生 `ship_count()` ベース: `at_population_cap()` = `ship_count() >= population_cap`。TiDi 予算と同じ単位（生カウント）。`--pop-cap N` で Sector 毎に可変、両 serve ループで新規入場を拒否・3 テスト。当初の「アクティブ船除外（実効人口）」案は撤回 — INV-MOVE により等速船はイベントを出さず「無イベント = idle」が不成立（放置船を安くするのは数の除外でなく LoD=8B-3 の忠実度低下で表現する） |
| 2 | Dynamic Sector Fission（分離可能負荷の第1手） | tick-model.md §8 | ⬜ |
| 3 | Simulation LoD（忠実度の階層化・更新間引き） | game-design.md §8 層1 / ADR-0020 | ⏸️ **deferred**（ADR-0020）。設計は完了（近似ゼロの 2 段階・交差閉包）だが、着手前のコストモデルで計算メリットが未実証と判明。サーバ計算は O(n²) でなく小定数の O(n)（ADR-0019）で、LoD が削るのは c·(n−k) のみ。再開は go/no-go スパイク（idle 反復が Tick 予算の有意割合か）次第 |
| 4 | 局所 TiDi: dilation = 実時間ペーシングのみ・論理 Tick の処理内容は不変（テスト） | INV-005 と無関係 | ✅ `dilation.rs::DilationController`（判定は論理コスト=ship_count、物理時刻不使用・決定的）。単一 `--serve` ループに実配線（sleep のみ伸ばす） |
| 5 | dilation が当該 Sector 局所であること（隣接へ伝播しない）のテスト | INV-TiDi (a) | ✅ コントローラは状態共有なし・per-Sector（`dilation_in_one_sector_does_not_affect_another`）。クラスタ（多 Sector lockstep）への per-Sector ペーシングは独立ループ化（8B-2 連動）が必要・未 |
| 6 | SLA イベント / メトリクス（dilation 係数・継続時間の記録） | INV-TiDi (b) 観測可能 | 🔶 `active_ticks`（継続 Tick）+ 係数・engage/recover ログ。構造化 SLA イベント化は未 |
| 7 | 負荷減での自動回復（係数 → 1.0）のテスト | INV-TiDi (d) | ✅ `auto_recovers_to_real_time_when_load_drops` |
| 8 | 差分 TiDi の越境因果ルールを実装 ADR で詰める | ADR-0018 未解決論点 | ⬜ |

> **Phase 8B 一区切り（2026-06-15）**
>
> **達成**: 過負荷対応ヒエラルキー（ADR-0018）の中核が機能する状態になった。
> - **局所 TiDi コア（8B-4/5/7）** ✅ — 決定論的に発動（論理コスト基準・非破壊・自動回復）。単一密戦闘の安全網。
> - **入場バックストップ（8B-1）** ✅ — 生カウントの最終手段。
> - **容量レバー（8C / AoI）** ✅ — 真の O(n²)（配信側）を解消し TiDi 閾値を押し上げ。
>
> これで**柱①（TiDi 閾値が桁違いに高い大規模リアルタイム戦闘 / ADR-0016）の主要レバーは単一 Sector 内で出揃った**。
> 単一密戦闘＝クライマックスは「AoI で容量↑ → それでも超えたら局所 TiDi で全員が少し遅い → 極端時のみ入場制限」で一貫して捌ける。
>
> **意図的に open のまま残す項目（柱①をブロックしない）**:
> - **8B-3 Simulation LoD** ⏸️ deferred（ADR-0020）— 計算メリット未実証。再開は go/no-go スパイク次第。
> - **8B-2 Dynamic Sector Fission** ⬜ — 要 ADR。密戦闘には効かず**空間分離可能な負荷**（複数戦線・広域経済）向け。
>   物理ノード分散（**8D**）と本質的に対であり、8D 着手時にまとめて設計するのが自然。クラスタ per-Sector ペーシング（8B-5 残り）の前提でもある。
> - **8B-8 差分 TiDi 越境** ⬜ — 別 ADR・8B-2 に依存。多 Sector の差分 dilation が前提。
> - **8B-6 構造化 SLA イベント** 🔶 — 係数・継続 Tick・engage/recover ログは実装済み。イベント化は小さな磨き込みで後回し可。
>
> **結論**: 密戦闘（柱①）の主要レバーが揃ったので Phase 8B を一区切りとする。残り（Fission / 越境 TiDi / SLA イベント化）は
> それぞれ独立 ADR・または 8D（分散インフラ）と連動して着手する。次の自然な前進先は **8D（物理ノード分散）** か
> **戦闘の深み（ADR-0016 §5: Tackle → Signature → Orbit/Keep at Range → Logistics）**。

### 8C. AoI 静的セルグリッド（ADR-0019）✅ ほぼ完了（NPCオートロック連携のみ保留）

> 8C が効くほど 8B-4〜7（TiDi 発動）が稀になる。両者は連動する。
> ADR-0019 で確定: 真に O(n²) なのは **AoI（配信側・O(p·n)）** のみ。サーバ計算側は戦闘が
> 既知ターゲットに作用するため近傍探索負荷が実在せず、専用の exact 半径加速グリッドは**撤回**。
> 解は **静的セルグリッド + 3×3×3 隣接可視**（EVE 流バケツ化 + 不連続を 1.5 セル先へ。3D ゆえ 27 セル）。
> 単一密戦闘は空間分割では救えず TiDi（ADR-0018）に落ちる。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | **新規 ADR 起票**（AoI 静的セルグリッド 3×3×3） | ADR-0019・人間承認済み 2026-06-15 | ✅ |
| 2 | `dawn-simulation` に静的セルグリッド（床除算 + セル→ShipId バケツ、近傍列挙・ShipId 順） | 派生・非永続（スナップショットに含めない）。`aoi.rs` + 6 テスト。3D ゆえ近傍は 3×3×3=27 セル | ✅ |
| 3 | `ws_server` `InitialState` を 3×3×3 スコープ化 + セル跨ぎで外周殻のみ Enter/Leave（churn 有界） | 帯域レバー（fb2a484 の発展） | ✅ 接続時スコープ化 + `aoi_delta` で毎 Tick Enter/Leave 配信（`AoiEnter`/`AoiLeave` 新メッセージ・両 serve ループ + client main.gd）|
| 4 | `DomainEvent` 配信フィルタ（関与 Ship が観測者の 27 セル近傍のときのみ送る） | 配信側の関心事・権威状態に触れない | ✅ `event_visible_to`（主船+副次船）で per-session フィルタ・両 serve ループ + 4 aoi テスト |
| 5 | （副次）NPC オートロック / 将来 AoE の半径内探索を同じ静的セルの 27 セル候補 + 厳密距離に載せ替え | 全走査版と同一結果テスト | ⬜ |
| 6 | p を増やしつつ AoI 有無の 1 Tick 時間・配信量を比較し閾値上昇を記録 | 容量↑の実証 | ✅ `--aoi-bench`（バイナリ内・benches 基盤未整備のため慣習に合わせた）。n=1k→20k で naive scan 770µs→315ms に対し AoI query ~16ms・speedup 3→19.5x・配信量 ~45x 削減 |

### 8D. 分散インフラ（物理ノード）✅ 完了（Raspberry Pi 実機検証 2026-07-01 PASS）

> **第1次 8D マイルストンは意図的に最小化する**（8D レビュー 2026-06-15 の結論）。
> 「巨大基盤の一括建設」ではなく「実機で検証できる薄いスライス」を先に通す:
> **静的 3 ノード config + postcard ワイヤ + ネットワーク RaftTransport + ログ配布ゴシップ（ADR-0021）+ LAN 平文
> → Pi 実機で Raft/Gossip を検証**。下記の defer 項目はトリガー付きで後続。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | ~~dawn-proto（protobuf）~~ → **不採用**。ワイヤ = postcard+serde 再利用 + 最小の版付きフレーミング（長さ前置・種別タグ・版ハンドシェイク）を transport 層に置く | AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」参照。理由: Rust↔Rust・多言語不要・スキーマ進化は event-schema-evolution.md で規律化済み。protobuf は型の二重定義のみ生む | ✅ 方針確定（不採用） |
| 2 | dawn-replication（追記ログのゴシップ配布 + アンチエントロピー + スナップショット転送） | 新規クレート・ADR-0021/0027（単一所有のため競合解決 CRDT/LWW は不要）。8D-2a: `ReplicationBus` を dawn-replication の `InMemoryReplicationBus` へ移動し、`dawn-actor` は純粋なクライアント転送境界（`ClientConnection`）に縮小済み。送信側: `OutboundLogPublisher` が append-log cursor と `LogBatch` suffix 構築を保持し、production Node は `publish_new_events` を呼ぶだけに縮小済み。8D-2b: `AntiEntropy`（gap 検出・重複/overlap 判定・`iter_from` suffix 応答）実装済み。8D-2c: `TcpReplicationTransport`（4-byte length prefix + postcard / LAN plaintext）実装済み。8D-2d: `SnapshotTransfer`（`Serialize+DeserializeOwned` ジェネリック・u32 LE length prefix / 256 MiB cap）実装済み（2 テスト）。消費側: `ReplicaSet`（peer セクターごとに gap 検出・冪等・順序保持で複製ログを保持。ライブ world 適用と failover は別機能）実装済み（M-5・6 テスト） | ✅ |
| 3 | ネットワーク `RaftTransport` 実装（`InProcessTransport` の差し替え。静的 config のピア表） | trait は既存（transport.rs）。TLS 可能な選択（TCP+rustls / QUIC）にし後付けを塞がない。`TcpRaftTransport`（4-byte LE + postcard / LAN plaintext / per-peer 自動再接続 / accept ループ）実装済み（dawn-consensus/src/tcp_transport.rs・8D-3） | ✅ |
| 4 | dawn-sector-node（本番実行バイナリ・上記 transport + ゴシップの配線・静的 config 起動） | 新規クレート。`TcpRaftTransport` + `TcpReplicationTransport` を TOML 静的 config で配線。3 プロセスで 3 セクタクラスタ（ws/:787{8,9,80} raft/:790{0,1,2} repl/:791{0,1,2}）。プレイヤー Jump 時は `Redirect` JSON でクライアントを宛先 WS へ誘導し、`player_id` / `ship_id` 付き Hello で同じ player ship を resume（2026-06-29） | ✅ |
| 5 | （任意・推奨）Raspberry Pi クラスタ実機検証 | 下記 ★ 参照 | ✅ 2026-07-01・3項目とも PASS（[8d5-hardware-notes.md](./8d5-hardware-notes.md) 実行ログ参照） |
| 6 | `dawn-sector-node` への永続化配線（FileEventStore + checkpoint + 起動時リカバリ） | Phase 3 で `FileEventStore`/`checkpoint()`/`CheckpointScheduler`/`restore_from` は実装・テスト済みだったが、8D-4 で新設した本番バイナリには配線されておらず、本番は `InMemoryEventStore`（再起動で全消失）のまま稼働していたことが判明。`NodeConfig` に `event_log_path`/`snapshot_path`/`cold_path`/`checkpoint_interval_ticks` を追加し、起動時にスナップショットの有無で新規/復元を分岐、tickループに `CheckpointScheduler::maybe_checkpoint` を配線。実機起動→kill→再起動で tick・log_index が継続することを確認済み | ✅ 2026-07-01 |

**defer（トリガー付き・第1次マイルストン外。2026-07-01 時点で4項目とも未発火、着手不要）:**

| 項目 | トリガー（いつ着手するか） | 現状 |
|---|---|---|
| Raft ログ圧縮 + **InstallSnapshot RPC** | Raft ログ（transit 専用で小・成長は遅い）の無限成長が問題化、または圧縮導入で base_index 前を捨て遅延 follower が AppendEntries で追えなくなったら（ADR-0017 圧縮と対の completeness 項目） | 未発火。`dawn-consensus/src/lib.rs` のスコープ注記どおり未実装のまま。8D-5 実機検証（数百隻規模・短時間）でもログ成長は問題化せず |
| メンバーシップ変更（Raft ConfChange） | ノード入替・スケール・**8B-2 Fission（動的トポロジ）** が要るとき | 未発火。8B-2 Fission は roadmap 上も `⬜`・未着手のまま（要 ADR） |
| 動的ノード発見 | 弾力クラスタにするとき（固定 3 ノードは静的 config で足りる） | 未発火。8D-4/8D-5 とも 3 ノード静的 config のまま運用・検証済み |
| TLS / 認証 | インターネット公開時（LAN の Pi 検証は平文で可）。transport を TLS 可能にしておけば後付け可 | 未発火。8D-5 の実機検証も意図的に LAN 平文のまま実施（[8d5-hardware-notes.md](./8d5-hardware-notes.md) Out of scope 参照）。インターネット公開の計画はまだない |

★ 実機検証（任意・推奨）: ネットワークトランスポート実装後、Raspberry Pi クラスタ
（Pi 4/5 推奨。Zero 2 W は aarch64 ビルド可だが 512MB RAM が制約のため数百隻規模に縮小）で
3 ノードを物理的に分離して動作確認する。目的: 実ネットワーク遅延・分断条件下での Raft / Gossip
挙動を実機で検証する（dawn の競争優位＝分散基盤の本番妥当性を確かめる / ADR-0016）。
検証項目・合否基準・自動検証スクリプトは [8d5-hardware-notes.md](./8d5-hardware-notes.md) 参照。

### 8E. Transit consensus（ADR-0017 §5 で方針決定済み）✅ 方針確定（バッチ提案は保留）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 単一 Raft グループを維持（実装変更なし） | マルチ Raft はメンテ不能として却下 | ✅ 方針確定 |
| 2 | バッチ提案（fleet jump = N 隻を 1 エントリに束ねる） | fleet-jump レイテンシが実測で問題化したら着手 | ⬜ 保留 |

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
