---
id      : ADR-0026
title   : dawn-sector クレート新設 — ゲームロジックの分離
status  : accepted
date    : 2026-06-19
deciders: [human, ai-agent]
related : CLAUDE.md §3（Dependency DAG）, CLAUDE.md §11（Crate別責務早見表）,
          ADR-0016（Game Vision）, docs/architecture/architecture-review-server.md（P3 完了後の課題）
---

# ADR-0026 — dawn-sector クレート新設

## 背景

CLAUDE.md §11 の設計意図では `dawn-simulation` は「実行バイナリ・配線専門」である。
しかし現状、`SimulationNode`（ゲームの中核ロジック）が `dawn-simulation` 内に直接
実装されており、クレートの責務が破綻している:

```
建前:  dawn-simulation = 配線・起動のみ
実態:  Warp / Transit / Combat / Bot AI / Spawn / AoI が全部 dawn-simulation にある
```

これにより次の問題が生じている:

1. **責務の混在**: ゲームロジックを変更するたびに実行バイナリクレートが巻き込まれる。
2. **テスト境界の曖昧さ**: ゲームロジックのテストが `--bin simulate` のビルドに依存する。
3. **将来の拡張障壁**: Signature Resolution・Logistics・Economy など次の機能追加の
   たびに `dawn-simulation` が肥大し続ける。
4. **`dawn-sector-node`（Phase 8D）への道が閉じる**: 本番バイナリを別クレートにする
   計画（CLAUDE.md §11）は、ゲームロジックが `dawn-simulation` に縛られている限り
   実現できない。

## 決定

`dawn-sector` クレートを新設し、ゲームロジックを `dawn-simulation` から移管する。

### 新しい Dependency DAG

```
dawn-core
    ↑
    ├── dawn-ecs
    ├── dawn-consensus
    └── dawn-event-store
            ↑
            ├── dawn-actor
            ├── dawn-replication
            └── dawn-sector          ← NEW: ゲームロジック専用
                    ↑
                    └── dawn-simulation  ← 配線・起動のみ
```

### dawn-sector の責務

Sector 単位のゲームシミュレーションロジック。

**移管対象（dawn-simulation → dawn-sector）:**

| 移管元 | 内容 |
|---|---|
| `src/node/` | `SimulationNode` struct・tick 実装・commands / navigation / serialization / sector_map / ship_registry |
| `src/transit.rs` | Raft Transit ロジック（Step 7.5）|
| `src/spawner.rs` | Ship 生成・despawn |
| `src/aoi.rs` | Area of Interest（CellGrid）|
| `src/dilation.rs` | TiDi 計算ロジック |
| `src/galaxy.rs` | 星系トポロジーデータ（Galaxy）と TOML schema parser |
| `src/persistence/` | StateSnapshot・CheckpointScheduler |

**dawn-simulation に残すもの（配線・起動）:**

| ファイル | 内容 |
|---|---|
| `src/main.rs` | エントリポイント・引数パース |
| `src/serve.rs` | WebSocket サーバーループ |
| `src/cluster.rs` | Raft クラスター配線 |
| `src/bench.rs` | ベンチマーク・デモ |
| `src/ws_server.rs` | WebSocket フレーム送受信 |
| `src/protocol.rs` | JSON ⇔ コマンド変換 |
| `src/sector_simulator_actor.rs` | Actor ラッパー |
| `src/data_loader.rs` | TOML 読み込み |
| `src/modules.rs` | モジュール定義デフォルト値 |
| `src/ship_types.rs` | 船種定義デフォルト値 |

### dawn-sector の依存関係

```toml
# crates/dawn-sector/Cargo.toml [dependencies]
dawn-core      = { path = "../dawn-core" }
dawn-ecs       = { path = "../dawn-ecs" }
dawn-event-store = { path = "../dawn-event-store" }
dawn-consensus = { path = "../dawn-consensus" }   # TransitOp が RaftActorHandle を参照
serde          = { version = "1", features = ["derive"] }
postcard       = "1"
hecs           = "0.10"
tokio          = { version = "1", features = ["sync"] }
serde_json     = "1"
```

ネットワーク I/O・WebSocket・ファイルI/O は持たない。

## 却下した選択肢

### A: dawn-simulation を分割せず node/ サブモジュールで凌ぐ

Phase 3（P3-1/P3-2）で実施済みの方向。サブモジュール化はできたが
クレート境界が引けないため、テスト独立性・将来の拡張性に限界がある。

### B: dawn-ecs にゲームロジックを移す

`dawn-ecs` の責務（CLAUDE.md §11）は「ECS World の薄いラッパー・Component定義・System定義」。
`SimulationNode`（Tick 全体の実行・イベント管理・Transit）は `dawn-ecs` より上位の概念であり
依存方向が逆転する。不採用。

### C: dawn-actor にゲームロジックを移す

`dawn-actor` はクライアント転送境界（ClientConnection）。ゲームロジックを入れると
転送境界とドメインロジックが混在し、`dawn-replication` との責務分離が崩れる。不採用。

## 実装チェックリスト

- [x] `crates/dawn-sector/` ディレクトリ・`Cargo.toml` 作成
- [x] workspace `Cargo.toml` に `dawn-sector` を追加
- [x] `dawn-simulation/src/node/` → `dawn-sector/src/node/` へ移管
- [x] `transit.rs` / `spawner.rs` / `aoi.rs` / `dilation.rs` / `galaxy.rs` / `persistence/` / `modules.rs` / `ship_types.rs` を移管
- [x] `dawn-simulation/Cargo.toml` に `dawn-sector` 依存を追加
- [x] `dawn-simulation` 内の `use crate::` を `use dawn_sector::` に変換
- [x] `cargo build --workspace` 成功確認
- [x] `cargo test --workspace` 全通過確認
- [x] CLAUDE.md §3（Dependency DAG）・§11（Crate別責務早見表）を更新
- [x] `docs/architecture/architecture-review-server.md` を更新（C-1 の根本対処として記録）

## 期待効果

- `dawn-simulation` がゲームロジックを持たない純粋な配線クレートになる
- `dawn-sector` 単体でゲームロジックのテストが可能になる（WebSocket ビルドが不要）
- Phase 8D の `dawn-sector-node`（本番バイナリ）は `dawn-sector` に依存するだけでよい
- 新機能（Signature Resolution・Logistics）の追加先が `dawn-sector` に明確化される
