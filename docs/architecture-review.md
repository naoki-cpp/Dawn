---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture.md
date     : 2026-06-19（Phase 6 完了後に更新）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B**

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector 新設でゲームロジックが分離された |
| ファイルサイズ | C+ | node/mod.rs 2,396行・serve.rs 899行が残存課題 |
| 型設計 | B+ | SectorMap・ShipRegistry 抽出で SimulationNode のフィールド数が適正化 |
| 重複 | B+ | `_owned` 4ペアは3行ラッパー（ロジック重複ゼロ）で許容。実質的な重複なし |
| Rust固有 | B+ | Box\<dyn\> ゼロ・Mutex 最小。clone は許容範囲 |
| AI開発由来 | B | 命名汚染なし。node/mod.rs・serve.rs の「残りもの置き場」化が懸念 |

---

## ファイルサイズ一覧（2026-06-19 時点）

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/mod.rs` | 2,396 | 🟡 改善中（2,868 → 2,396; P4-1/P4-2 完了）|
| `crates/dawn-sector/src/node/spawner_logic.rs` | 394 | 🟢 P4-2 で新設 |
| `crates/dawn-sector/src/node/navigation.rs` | 301 | 🟢 |
| `crates/dawn-sector/src/node/commands.rs` | 262 | 🟢 |
| `crates/dawn-sector/src/star_map.rs` | 262 | 🟢 |
| `crates/dawn-sector/src/aoi.rs` | 246 | 🟢 |
| `crates/dawn-sector/src/transit.rs` | 215 | 🟢 |
| `crates/dawn-sector/src/node/serialization.rs` | 158 | 🟢 |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 168 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 156 | 🟢 |
| `crates/dawn-sector/src/node/tick.rs` | 91 | 🟢 P4-1 で新設 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/serve.rs` | 899 | 🟡 2モード（single/cluster）混在 → P5-1 で分割予定 |
| `crates/dawn-simulation/src/cluster.rs` | 528 | 🟢 |
| `crates/dawn-simulation/src/data_loader.rs` | 479 | 🟡 3種ローダー混在 → P5-2 で分割予定 |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 421 | 🟢 |
| `crates/dawn-simulation/src/bench.rs` | 411 | 🟢 |
| `crates/dawn-simulation/src/protocol.rs` | 309 | 🟢 |
| `crates/dawn-simulation/src/ws_server.rs` | 199 | 🟢 |
| `crates/dawn-simulation/src/main.rs` | 63 | 🟢 |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 573 | 🟡 許容範囲（Raft 実装の核）|
| `crates/dawn-core/src/events.rs` | 495 | 🟢 |
| `crates/dawn-event-store/src/file.rs` | 431 | 🟢 |
| `crates/dawn-ecs/src/systems/combat.rs` | 431 | 🟢 |
| `crates/dawn-consensus/src/actor.rs` | 430 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 414 | 🟢 |

---

## 問題一覧

### Critical

現在 Critical な問題はない。
前回 C-1 だった god object 問題はクレート分離（ADR-0026）で根本対処済み。

---

### High

#### H-1: `node/mod.rs` 2,396行（残存 god file、改善中）

P4-1/P4-2 で tick.rs・spawner_logic.rs を分離し 472行削減（2,868→2,396）。
ただし以下の責務がまだ混在している:

```
現在 node/mod.rs が抱えているもの:
  - SimulationNode struct 定義と定数（適切）
  - export_transit / import_transit / propose_transit（~100行）
  - process_tackle（~80行）
  - Lock-on ロジック（~100行）
  - take_snapshot / restore_from_snapshot（~100行）
  - AoI 関連ヘルパー（~50行）
  - apply_event（~80行）
  - テストコード（~1,400行 ≒ ファイルの過半）
```

テストの分離（L-1）と、残存責務の tackle.rs / snapshot.rs 等への分割（将来 P4-4 以降）が候補。

---

### Medium

#### M-1: `serve.rs` 899行（2サーバーモードの混在）

```
run_phase4_server()   — シングルノード WebSocket ループ（~220行）
run_cluster_server()  — Raft クラスター WebSocket ループ（~370行）
apply_common_command()— 共通コマンド処理（~36行）
deliver_aoi_frame()   — AoI 配信（~55行）
build_serve_node()    — ノード初期化（~16行）
spawn_npc_frigates()  — NPC スポーン（~14行）
```

2つのサーバーモードが同一ファイルに同居。どちらかを変更するたびに全体を把握する必要がある。

#### M-2: `data_loader.rs` 479行（3種ローダー混在）

`load_ship_types()` / `load_modules()` / `load_star_map()` と各中間型が同一ファイルに混在。
今後データ種が増えるほど肥大する。

#### M-3: `sector_simulator_actor.rs` と `SimulationNode` の密結合

`SectorSimulatorActor` は `SimulationNode` の公開メソッドをほぼ全て呼ぶ薄いラッパー。
`SimulationNode` の変更が即 Actor に波及する。
将来の `dawn-replication`（Phase 8D）配線を入れる際に複雑化する。

---

### Low

#### L-1: `node/mod.rs` のテストコード（~1,400行）が実装と混在

テストが実装ファイルに直接書かれているため、テストだけ読みたいときに実装が邪魔になる。
`tests/` ディレクトリへの分離は Rust の `#[cfg(test)]` モデル上任意だが、
このファイルサイズでは分離した方が可読性が上がる。

#### L-2: コンポーネント snapshot ループの重複（combat / capacitor / lock）

```rust
// dawn-ecs の combat.rs, capacitor.rs, lock.rs でほぼ同じパターン
let ships: Vec<_> = world.inner()
    .query::<(&ShipIdComp, &ShipStatsComp, &PositionComp, ...)>()
    .iter()
    .map(|(e, (id, stats, pos, ...))| ...)
    .collect();
```

`SimWorld` に `query_ships()` ヘルパーがないため各 system が手書き。

#### L-3: `star_system.rs`（dawn-core）と `star_map.rs`（dawn-sector）の命名が紛らわしい

```
dawn-core/src/star_system.rs    — 型定義（StarSystemDef, JumpGateDef 等）
dawn-sector/src/star_map.rs     — インスタンスデータ（StarMap struct, builtin()）
```

型とデータの区別が名前から読み取りにくい。

---

## 改善ロードマップ

### 完了済み

| 作業 | 完了日 | 内容 |
|---|---|---|
| P2-1 node.rs サブモジュール化 | 2026-06-19 | commands / navigation / serialization / sector_map / ship_registry に分割 |
| P2-2 main.rs 分割 | 2026-06-19 | 63行に縮小。実装は serve.rs / bench.rs / cluster.rs へ |
| P2-3 ws JSON 分離 | 2026-06-19 | serialization.rs + protocol.rs に分離 |
| P3-1 SectorMap / ShipRegistry 抽出 | 2026-06-19 | SimulationNode フィールド 17→12 |
| P3-2 persistence/ サブモジュール | 2026-06-19 | snapshot.rs + checkpoint.rs を persistence/ 下に統合 |
| ADR-0026 dawn-sector 新設 | 2026-06-19 | ゲームロジックを dawn-simulation から完全分離 |
| P4-1 tick.rs 抽出 | 2026-06-19 | tick() / tick_with_lock_commands() を node/tick.rs へ（91行）|
| P4-2 spawner_logic.rs 抽出 | 2026-06-19 | spawn/bot メソッド群を node/spawner_logic.rs へ（394行）。node/mod.rs 2,868→2,396行 |
| P4-3 `_owned` 統合 | — | スキップ: `_owned` は3行ラッパーでロジック重複ゼロ。統合コストが効果を上回る |
| P5-1 serve.rs 分割 | 2026-06-19 | serve/mod.rs・single.rs・cluster.rs の3ファイルに分割 |
| P5-2 data_loader.rs 分割 | 2026-06-19 | data_loader/{mod,ship_types,modules,star_map}.rs に分割 |
| P6-1 `SimWorld` クエリヘルパー追加 | 2026-06-19 | `find_entity` / `query` / `get` / `get_mut` を追加。combat/capacitor/lock/fitting の `inner()` 脱出を削減 |

---

### Phase 6 — dawn-ecs のヘルパー整備（完了）

`SimWorld` に `find_entity` / `query` / `get` / `get_mut` を追加し、
combat / capacitor / lock / fitting から `inner()` / `inner_mut()` 直接呼び出しを削減した。
`fitting.rs` の entity 検索ブロック（4行×4箇所）が `find_entity(id)?` 1行に整理された。
日本語コメントをタッチしたファイル内で英語に変換した。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（Actor 基盤）— 将来の `dawn-replication` と接続予定
