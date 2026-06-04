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
現在のフェーズ : Phase 3 — Event 永続化
フェーズの状態 : 未着手
```

### 完了済みフェーズ

- ✅ Phase 0 — 基盤確立（`cargo test --workspace` 49テスト全パス）
- ✅ Phase 1 — Single Node シミュレーション検証（max 11,847 µs ≤ 16,000 µs 目標達成）
- ✅ Phase 2 — In-Memory Multi-Node（3ノード 63,000イベント整合性 ✓、65テスト全パス）

### 次に着手すべきタスク

**ファイルベース EventStore の実装（Append-only Log）**

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

## 7. Phase 4 — Dummy Network + Godot 最小クライアント

**優先順位の根拠:** ゲーム体験を先に確立してからネットワークを実装する。
本物のネットワーク（gRPC/QUIC）は高コストのため後回しにし、
ダミー実装で Godot クライアントを動かしてゲームループをブラッシュアップする。
ネットワーク化は `ClientConnection` trait の差し替えだけで完結するよう設計する。
→ 詳細設計は ADR-0005 を参照

**完了基準:** Godot 上で Ship が 3D 宇宙空間を動いており、
サーバーのイベントがリアルタイムに反映される

| タスク | 状態 | 備考 |
|---|---|---|
| `ClientConnection` trait 定義 | ⬜ 未着手 | Event 配信 + Command 受付の2方向のみ |
| `InProcessConnection` 実装 | ⬜ 未着手 | In-Memory Channel で直結（ダミー） |
| Godot 4 プロジェクト初期化 | ⬜ 未着手 | `client/` ディレクトリ |
| Ship を 3D 空間に表示 | ⬜ 未着手 | GDScript |
| サーバーイベントを受信して位置反映 | ⬜ 未着手 | GDScript |
| スカイボックス（宇宙背景） | ⬜ 未着手 | |

### ClientConnection 抽象化の設計

```
サーバー側                         クライアント側
──────────────────────────────     ─────────────────
ReplicationBus                     Godot シーン
    ↓                                  ↑
ClientConnection trait  ←←←←←←←←←←←←←←←
    ├── InProcessConnection（Phase 4） ← チャンネル直結
    └── GrpcConnection（Phase 6）      ← 本物のネットワーク

Godot クライアントは trait に向かって書く。
実装の差し替えで Godot 側のコードは変更不要。
```

**trait の責務はこの2方向のみ:**

```
サーバー → クライアント : DomainEvent のストリーム
クライアント → サーバー : Command の送信
```

---

## 8. Phase 5 — ゲーム体験

**完了基準:** 「遊べる」と感じられる最小のゲームループが成立する

| タスク | 状態 | 備考 |
|---|---|---|
| Navigation Context（Warp / Dock） | ⬜ 未着手 | Ship Template 導入も含む |
| Godot 航行エフェクト（ワープ演出等） | ⬜ 未着手 | |
| Combat Context 基礎（武器 / HP / 破壊） | ⬜ 未着手 | |
| Godot 戦闘エフェクト / HUD | ⬜ 未着手 | |

---

## 9. Phase 6 — 本物のネットワーク

**完了基準:** `InProcessConnection` を `GrpcConnection` に差し替え、
別プロセスの Godot クライアントが接続できる。
**Godot 側のコードは変更しない。**

| タスク | 状態 | 備考 |
|---|---|---|
| `dawn-proto` クレート追加（protobuf定義） | ⬜ 未着手 | |
| gRPC / QUIC サーバー実装 | ⬜ 未着手 | tonic |
| `GrpcConnection` 実装 | ⬜ 未着手 | trait 差し替え |
| Godot GDScript の接続先を切り替え | ⬜ 未着手 | 変更最小化 |
| 別プロセス接続テスト | ⬜ 未着手 | |

---

## 10. Phase 7 以降（方向性のみ）

詳細設計は Phase 6 完了後に行う。

```
Phase 7: 分散コンセンサス（Raft）
          Sector Transit の整合性保証
          完了基準: ノード障害後に Sector Transit が正しく完了する

Phase 8: スケール基盤（Anti-TiDi）
          Sector Population Cap / Dynamic Fission
          Spatial Index / Interest Management
          完了基準: 1 Sector 5,000 ships 上限で Tick SLA を常に満たす

Phase 9: Resource + Economy Context
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
サーバー通信  : InProcessConnection（Phase 4〜5）
               → GrpcConnection（Phase 6〜）
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
Anti-TiDi の制約（INV-TIDI）は全フェーズで維持する
Event Sourcing の原則（INV-001〜006）は全フェーズで維持する
```

---

## 11. 廃止・変更された計画の記録

### 2026-06-04: Phase 4〜11 の優先順位を変更

**旧計画:**
```
Phase 4: ネットワーク層（gRPC/QUIC）
Phase 5: Raft
Phase 6: スケール基盤
Phase 7: クライアント（Navigation + Godot最小クライアント）
…
```

**新計画:**
```
Phase 4: Dummy Network + Godot 最小クライアント  ← 前倒し
Phase 5: ゲーム体験（Navigation + Combat）
Phase 6: 本物のネットワーク（gRPC/QUIC）        ← 後ろ倒し
Phase 7: Raft
…
```

**変更理由:**
ゲーム体験の確立を優先する。本物のネットワーク実装は高コストのため、
`ClientConnection` trait でダミー実装と差し替え可能な設計にしたうえで後回しにする。
Godot クライアントは trait に向かって書くため、ネットワーク化時に変更不要。
