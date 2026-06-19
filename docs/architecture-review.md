---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture.md
date     : 2026-06-19（Phase 5 / Phase 6 完了後に更新）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B+**

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector 新設でゲームロジックが分離された（ADR-0026） |
| ファイルサイズ | B− | serve.rs / data_loader.rs を分割済み。残る課題は node/mod.rs 2,396行のみ |
| 型設計 | B+ | SectorMap・ShipRegistry 抽出で SimulationNode のフィールド数が適正化 |
| 重複 | A− | `_owned` 4ペアは3行ラッパーで許容。P6-1 で system 間のクエリ手書きも解消 |
| Rust固有 | B+ | Box\<dyn\> ゼロ・Mutex 最小。clone は許容範囲 |
| AI開発由来 | B+ | 命名汚染なし。「残りもの置き場」は node/mod.rs を残すのみ |

---

## ファイルサイズ一覧（2026-06-19 時点）

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/mod.rs` | 2,396 | 🟡 残存課題（テスト ~1,400行 + 未抽出責務）|
| `crates/dawn-sector/src/node/spawner_logic.rs` | 394 | 🟢 P4-2 で新設 |
| `crates/dawn-sector/src/node/navigation.rs` | 301 | 🟢 |
| `crates/dawn-sector/src/node/commands.rs` | 262 | 🟢 |
| `crates/dawn-sector/src/star_map.rs` | 262 | 🟢 |
| `crates/dawn-sector/src/aoi.rs` | 246 | 🟢 |
| `crates/dawn-sector/src/transit.rs` | 215 | 🟢 |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 168 | 🟢 |
| `crates/dawn-sector/src/dilation.rs` | 160 | 🟢 |
| `crates/dawn-sector/src/node/serialization.rs` | 158 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 156 | 🟢 |
| `crates/dawn-sector/src/modules.rs` | 137 | 🟢 |
| `crates/dawn-sector/src/spawner.rs` | 127 | 🟢 |
| `crates/dawn-sector/src/node/tick.rs` | 91 | 🟢 P4-1 で新設 |
| `crates/dawn-sector/src/ship_types.rs` | 82 | 🟢 |
| `crates/dawn-sector/src/node/ship_registry.rs` | 33 | 🟢 P3-1 |
| `crates/dawn-sector/src/node/sector_map.rs` | 25 | 🟢 P3-1 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 528 | 🟢 Raft クラスター配線 |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 421 | 🟢 |
| `crates/dawn-simulation/src/bench.rs` | 411 | 🟢 |
| `crates/dawn-simulation/src/serve/mod.rs` | 382 | 🟢 P5-1 共通ヘルパー |
| `crates/dawn-simulation/src/protocol.rs` | 309 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 241 | 🟢 P5-1 |
| `crates/dawn-simulation/src/ws_server.rs` | 199 | 🟢 |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 190 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 177 | 🟢 P5-1 |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 174 | 🟢 P5-2 |
| `crates/dawn-simulation/src/data_loader/star_map.rs` | 98 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 63 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 12 | 🟢 P5-2 pub use |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 573 | 🟡 許容範囲（Raft 実装の核）|
| `crates/dawn-core/src/events.rs` | 495 | 🟢 |
| `crates/dawn-event-store/src/file.rs` | 431 | 🟢 |
| `crates/dawn-ecs/src/systems/combat.rs` | 431 | 🟢 |
| `crates/dawn-consensus/src/actor.rs` | 430 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 414 | 🟢 |
| `crates/dawn-ecs/src/world.rs` | 252 | 🟢 P6-1 クエリヘルパー追加 |

---

## 問題一覧

### Critical

現在 Critical な問題はない。
前回 C-1 だった god object 問題はクレート分離（ADR-0026）で根本対処済み。

---

### High

#### H-1: `node/mod.rs` 2,396行（唯一残存する god file）

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

最も効果が大きいのはテストの分離（L-1・~1,400行）。
次いで残存責務の `tackle.rs` / `snapshot_io.rs` 等への分割。

---

### Medium

#### M-3: `sector_simulator_actor.rs` と `SimulationNode` の密結合

`SectorSimulatorActor` は `SimulationNode` の公開メソッドをほぼ全て呼ぶ薄いラッパー。
`SimulationNode` の変更が即 Actor に波及する。
将来の `dawn-replication`（Phase 8D）配線を入れる際に複雑化する。

> M-1（serve.rs 分割）・M-2（data_loader.rs 分割）は P5-1 / P5-2 で解消済み。

---

### Low

#### L-1: `node/mod.rs` のテストコード（~1,400行）が実装と混在

テストが実装ファイルに直接書かれているため、テストだけ読みたいときに実装が邪魔になる。
`tests/` ディレクトリへの分離は Rust の `#[cfg(test)]` モデル上任意だが、
このファイルサイズでは分離した方が可読性が上がる。H-1 解消の主役。

#### L-3: `star_system.rs`（dawn-core）と `star_map.rs`（dawn-sector）の命名が紛らわしい

```
dawn-core/src/star_system.rs    — 型定義（StarSystemDef, JumpGateDef 等）
dawn-sector/src/star_map.rs     — インスタンスデータ（StarMap struct, builtin()）
```

型とデータの区別が名前から読み取りにくい。

> L-2（system 間の snapshot ループ重複）は P6-1（`SimWorld` クエリヘルパー）で解消済み。

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
| P5-1 serve.rs 分割 | 2026-06-19 | serve/{mod,single,cluster}.rs の3ファイルに分割（899行 → 382/177/241）|
| P5-2 data_loader.rs 分割 | 2026-06-19 | data_loader/{mod,ship_types,modules,star_map}.rs に分割（479行 → 12/174/190/98）|
| P6-1 `SimWorld` クエリヘルパー追加 | 2026-06-19 | `find_entity` / `query` / `get` / `get_mut` を追加。combat/capacitor/lock/fitting の `inner()` 脱出を削減（L-2 解消）|

---

### Phase 7 — node/mod.rs の最終分割（次の優先項目）

唯一残る god file（H-1）への対処。優先度順:

**P7-1: テストモジュールの分離（L-1・最大の効果）**

`node/mod.rs` の `#[cfg(test)]` ブロック（~1,400行）を `tests/` 統合テスト、または
`node/tests.rs` サブモジュールへ移す。これだけで実装本体は ~1,000行に縮む。

**P7-2: 残存責務の抽出**

```
node/
  tackle.rs    — process_tackle（~80行）
  snapshot_io.rs — take_snapshot / restore_from_snapshot（~100行）
  lock.rs      — Lock-on ロジック（~100行）
```

`export_transit` / `import_transit` / `propose_transit` は既存の `transit.rs`
（dawn-sector トップレベル）との責務整理が必要なため、分割前に置き場を確定する。

---

### Phase 8 — 配線層の整理（Phase 8D と連動）

**M-3: `SectorSimulatorActor` の依存縮小**

`dawn-replication`（ADR-0021・Phase 8D）着手時に `SimulationNode` との
インターフェースを見直す。単独で着手するより 8D 設計とまとめるのが自然。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（Actor 基盤）— 将来の `dawn-replication` と接続予定
