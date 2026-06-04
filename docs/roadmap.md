---
scope    : 何を・どの順番で・なぜその順番で作るか。現在地と次のステップの明示
audience : AI Agent / Human Developer
update   : フェーズ完了時 / タスクが完了するたびに更新する
related  : architecture.md, CLAUDE.md §1
---

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
現在のフェーズ : Phase 2 — In-Memory Multi-Node
フェーズの状態 : 未着手
```

### 完了済みフェーズ

- ✅ Phase 0 — 基盤確立（`cargo test --workspace` 49テスト全パス）
- ✅ Phase 1 — Single Node シミュレーション検証（max 11,847 µs ≤ 16,000 µs 目標達成）

### 次に着手すべきタスク

**`dawn-actor` クレートの作成（tokio mpsc ベースの Actor 基盤）**

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

## 5. Phase 2 — In-Memory Multi-Node

**完了基準:** 3 つの論理ノードが In-Memory Channel 経由でイベントを同期し、
全ノードのイベントログが一致することをテストで確認する

| タスク | 状態 | 依存 |
|---|---|---|
| `dawn-actor` クレート作成（Actor 基盤） | ⬜ 未着手 | Phase 1 完了後 |
| `SectorSimulatorActor` 実装 | ⬜ 未着手 | |
| `EventStoreActor` 実装 | ⬜ 未着手 | |
| ノード間 In-Memory Channel 接続 | ⬜ 未着手 | |
| 3 ノード整合性テスト | ⬜ 未着手 | |

### Phase 2 の制約（変えない）

```
通信: In-Memory Channel（tokio::mpsc）のみ
ネットワーク: 引き続き不使用
ノード数: 3 固定
```

---

## 6. Phase 3 — Event 永続化

**完了基準:** ノードを再起動した後、Snapshot + Event Replay によって
シャットダウン直前の Ship 状態が完全に復元される

| タスク | 状態 | 依存 |
|---|---|---|
| ファイルベース EventStore 実装 | ⬜ 未着手 | Phase 2 完了後 |
| Snapshot 取得ロジック | ⬜ 未着手 | |
| Snapshot からの State 復元 | ⬜ 未着手 | |
| 再起動後の整合性テスト | ⬜ 未着手 | |

---

## 7. Phase 4 以降（方向性のみ）

詳細設計は Phase 3 完了後に行う。現時点では方向性のみ記録する。

### インフラ拡張フェーズ

```
Phase 4: ネットワーク層（gRPC / QUIC）
          In-Memory Channel を gRPC に置き換える
          trait による抽象化で dawn-actor への変更を最小化する
          完了基準: 別プロセスの3ノードが通信できる

Phase 5: 分散コンセンサス（Raft）
          Sector Transit の整合性保証
          Leader 選出 / Log Replication
          完了基準: ノード障害後にSector Transitが正しく完了する

Phase 6: スケール基盤
          Sector Population Cap の実装（Anti-TiDi）
          Dynamic Sector Fission（負荷超過前の自動分割）
          Spatial Index（近傍クエリ）
          Interest Management（Bubble配信）
          完了基準: 1 Sector 5,000 ships 上限でTick SLAを常に満たす
```

### ゲーム機能フェーズ（Bounded Context 拡張）

クライアントはゲーム機能と**並行して開発する**。
機能を実装するたびにクライアントで動作確認できる状態を維持する。

```
Phase 7: Navigation Context + Minimal Client（同時進行）

  Phase 7-Server: Navigation Context（サーバー側）
    Warp（高速移動）/ Dock（停泊）/ Jump Gate
    Ship Template の導入（データ駆動 / TOML定義）
    完了基準: Warpコマンドで目的地まで自律移動できる

  Phase 7-Client: Godot 最小クライアント（クライアント側）
    技術: Godot 4 + GDScript
    gRPC でサーバーからイベントを受信
    Ship を 3D 空間に表示・移動を反映
    スカイボックス（宇宙背景）
    完了基準: Godot 上で Ship が 3D 宇宙空間を動いているのが見える

Phase 8: Combat Context + Combat View

  Phase 8-Server: Combat Context
    武器 / ダメージ / Shield / Armor / Hull
    ターゲティング / 射程管理
    完了基準: Ship同士が戦闘し、どちらかがDestroyedになる

  Phase 8-Client: 戦闘エフェクト
    武器発射パーティクル / 爆発エフェクト
    HUD（Shield/Armor/Hull ゲージ）
    ターゲット表示
    完了基準: 戦闘が 3D で視覚的に確認できる

Phase 9: Resource Context + Mining View
  採掘ビーム・資源オブジェクト表示
  完了基準: 採掘動作が 3D で確認できる

Phase 10: Economy Context + Market UI
  市場画面 / 取引 UI
  完了基準: ゲーム内でアイテムを売買できる

Phase 11: Client 本格化（GDExtension 導入）
  godot-rust (GDExtension) で Client-Side Prediction を Rust 実装
  dawn-core の型を Godot へ直接公開
  本格的な宇宙エフェクト（ネビュラ・レンズフレア・ワープトンネル）
  完了基準: レイテンシを隠した滑らかな操作感が実現できる
```

### クライアント技術スタック（決定済み）

```
エンジン    : Godot 4
ゲームロジック: GDScript（AI が主に書く）
高性能処理  : godot-rust / GDExtension（Phase 11 以降）
サーバー通信: gRPC（Phase 4 完了後） / In-Memory（Phase 7 開発時）
型共有      : Phase 7-11: proto 変換 → Phase 11: GDExtension で直接共有

→ 技術選択の根拠は ADR-0004 を参照
```

### リポジトリ構成（Phase 7-Client 追加時）

```
dawn/                       ← 既存 Cargo Workspace（サーバー）
client/                     ← Godot プロジェクト（新規追加）
  project.godot
  scenes/
    main.tscn
    ship.tscn
  scripts/
    server_connection.gd    ← gRPC 受信
    ship_controller.gd      ← Ship 表示・移動
    skybox.gd
  assets/
    models/                 ← Ship 3D モデル（glTF）
    shaders/                ← 宇宙エフェクト
  gdextension/              ← Phase 11 以降
    Cargo.toml              ← godot-rust
    src/
      lib.rs                ← dawn-core を import
```

### フェーズ横断の設計原則

```
各 Server Context は独立した Crate として追加する
上位 Context は下位 Context に依存しない（Spatial ← Navigation ← Combat …）
各 Server フェーズに対応する Client フェーズを必ず用意する
Anti-TiDi の制約（INV-TIDI）は全フェーズで維持する
Event Sourcing の原則（INV-001〜006）は全フェーズで維持する
```

---

## 8. 廃止・変更された計画の記録

変更があった場合のみ追記する。

現時点での変更履歴: **なし**
