---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture.md
date     : 2026-06-19（ADR-0026 実装後に全面改訂）
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
| ファイルサイズ | C | node/mod.rs 2,868行が唯一の重大問題 |
| 型設計 | B+ | SectorMap・ShipRegistry 抽出で SimulationNode のフィールド数が適正化 |
| 重複 | C | `_owned` 系メソッドの二重化（4ペア）が残存 |
| Rust固有 | B+ | Box\<dyn\> ゼロ・Mutex 最小。clone は許容範囲 |
| AI開発由来 | B | 命名汚染なし。node/mod.rs の「残りもの置き場」化が唯一の懸念 |

---

## ファイルサイズ一覧（2026-06-19 時点）

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/mod.rs` | 2,868 | 🔴 要分割（唯一の残存 god file）|
| `crates/dawn-sector/src/node/navigation.rs` | 301 | 🟢 |
| `crates/dawn-sector/src/node/commands.rs` | 262 | 🟢 |
| `crates/dawn-sector/src/star_map.rs` | 262 | 🟢 |
| `crates/dawn-sector/src/aoi.rs` | 246 | 🟢 |
| `crates/dawn-sector/src/transit.rs` | 215 | 🟢 |
| `crates/dawn-sector/src/node/serialization.rs` | 158 | 🟢 |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 168 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 156 | 🟢 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/serve.rs` | 899 | 🟡 2モード（single/cluster）混在 |
| `crates/dawn-simulation/src/cluster.rs` | 528 | 🟢 |
| `crates/dawn-simulation/src/data_loader.rs` | 479 | 🟡 3種ローダー混在 |
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

#### H-1: `node/mod.rs` 2,868行（dawn-sector 内の残存 god file）

クレート境界は正しくなった（ADR-0026）が、ファイル内の責務分離が不完全。
`node/mod.rs` が「残りもの置き場」になっており、以下の責務が混在している:

```
現在 node/mod.rs が抱えているもの:
  - SimulationNode struct 定義と定数（適切）
  - tick() 実装・Tick Step 1〜10（~200行）
  - spawn_ship / despawn_ship（~80行）
  - export_transit / import_transit / propose_transit（~100行）
  - Bot AI（process_bots / bot steering）（~150行）
  - Lock-on ロジック（~100行）
  - take_snapshot / restore_from_snapshot（~100行）
  - AoI 関連ヘルパー（~50行）
  - テストコード（~1,400行 ≒ ファイルの半分）
```

次の機能追加（Signature Resolution・Logistics）のたびにここが肥大し続ける。

#### H-2: `_owned` / 直接呼び出しの二重化（4ペア）

```rust
// commands.rs
pub fn apply_move_command(&mut self, ship_id, target)
pub fn apply_move_command_owned(&mut self, player_id, ship_id, target)
pub fn apply_stop_command(&mut self, ship_id)
pub fn apply_stop_command_owned(&mut self, player_id, ship_id)

// navigation.rs
pub fn apply_approach_command(&mut self, ship_id, target)
pub fn apply_approach_command_owned(&mut self, player_id, cmd)
pub fn apply_warp_command(&mut self, ship_id, target, auto_jump)
pub fn apply_warp_command_owned(&mut self, player_id, cmd)
```

認証チェック 1行のみ違い、本体は同一。バグ修正時に片方を忘れるリスク。

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

---

### Phase 4 — node/mod.rs の責務分散（次の優先項目）

**P4-1: tick.rs 抽出**

```
dawn-sector/src/node/
  tick.rs   — tick() 実装・Tick Step 1〜10（~200行）
```

最も独立性が高く、テストが豊富。低リスクで ~200行削減。

**P4-2: spawner_logic.rs 抽出**

```
dawn-sector/src/node/
  spawner_logic.rs  — spawn_ship / despawn_ship / adopt_player_ship（~80行）
```

既存の `spawner.rs`（SpawnConfig）との名前混在に注意。

**P4-3: `_owned` メソッド統合**

```rust
// Before: 4 ペア × 2 = 8 メソッド
// After: auth: Option<PlayerId> を追加して統合
pub fn apply_move_command(&mut self, ship_id, target, auth: Option<PlayerId>) -> bool
```

影響範囲: commands.rs / navigation.rs + 呼び出し元（serve.rs / sector_simulator_actor.rs）。

---

### Phase 5 — dawn-simulation の整理

**P5-1: serve.rs をモード別に分割**

```
dawn-simulation/src/
  serve/
    mod.rs      — 共通ヘルパー（build_serve_node / spawn_npc_frigates / AOI_CELL_SIZE）
    single.rs   — run_phase4_server()
    cluster.rs  — run_cluster_server()
```

**P5-2: data_loader.rs をサブモジュール分割**

```
dawn-simulation/src/data_loader/
  mod.rs        — pub use
  ship_types.rs — ShipTypeEntry + load_ship_types()
  modules.rs    — ModuleEntry + load_modules()
  star_map.rs   — StarMapFile + load_star_map()
```

---

### Phase 6 — dawn-ecs のヘルパー整備

**P6-1: `SimWorld::query_ships()` ヘルパー追加**

combat / capacitor / lock 各 system の snapshot ループを共通化。
各 system で 20〜30行削減。`dawn-ecs` 単独で完結する変更。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（Actor 基盤）— 将来の `dawn-replication` と接続予定
