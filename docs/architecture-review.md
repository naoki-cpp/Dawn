---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture.md
date     : 2026-06-19
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B−**

| 観点 | 評価 | 理由 |
|---|---|---|
| モジュール構成 | B+ | Crate DAG は設計通り。クレート間は良好 |
| ファイルサイズ | D | node.rs 3,103行は明確な問題 |
| 型設計 | B | dawn-core は清潔。SimulationNode が肥大 |
| 重複 | C | `_owned` 系メソッドの二重化が目立つ |
| Rust固有 | B+ | Box\<dyn\> ゼロ・Mutex 最小。clone は許容範囲 |
| AI開発由来 | A− | 命名汚染なし。god object のみ |

---

## ファイルサイズ一覧（2026-06-19 時点）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/node.rs` | 3,103 | 🔴 God Object |
| `crates/dawn-simulation/src/main.rs` | 1,208 | 🔴 要分割 |
| `crates/dawn-simulation/src/ws_server.rs` | 507 | 🟡 混在あり |
| `crates/dawn-consensus/src/state.rs` | 502 | 🟡 許容範囲 |
| `crates/dawn-simulation/src/cluster.rs` | 429 | 🟢 |
| `crates/dawn-core/src/events.rs` | 423 | 🟢 |
| `crates/dawn-simulation/src/data_loader.rs` | 416 | 🟡 混在あり |
| `crates/dawn-consensus/src/actor.rs` | 381 | 🟢 |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 370 | 🟢 |
| `crates/dawn-event-store/src/file.rs` | 365 | 🟢 |
| `crates/dawn-ecs/src/systems/combat.rs` | 362 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 353 | 🟢 |
| `crates/dawn-ecs/src/systems/movement.rs` | 306 | 🟢 |

---

## 問題一覧

### Critical

#### C-1: `SimulationNode` god object（node.rs 3,103行）✅ 根本対処済み（ADR-0026）

構造体フィールド 17個、公開メソッド ~65個、概念ドメイン 7以上。

```
担っている責務:
  - ECS world のオーナーシップ
  - Tick Step 1〜10 の全実行
  - コマンド受理（Move/Stop/Warp/Approach/Lock/Fit/Jump）
  - イベント Append とリプレイ
  - JSON シリアライズ（WebSocket 向け）
  - Bot AI 呼び出し
  - Transit / Jump Gate ロジック
  - スナップショット取得
```

影響: 機能追加のたびに同一ファイルが肥大。テスト対象の絞り込みが困難。コンフリクト多発。

---

### High

#### H-1: `_owned` / 直接呼び出しの二重化（node.rs）

同一コマンドに対して 2 種類のメソッドが存在（合計 8+ ペア）:

```rust
// 直接（ShipId 指定）— Bot / NPC / 内部
pub fn apply_move_command(&mut self, ship_id, target)
pub fn apply_approach_command(&mut self, ship_id, target)
pub fn apply_warp_command(&mut self, ship_id, target, auto_jump)
pub fn apply_lock_on_command(&mut self, ship_id, target)

// 認証付き（PlayerId 検証）— プレイヤー
pub fn apply_move_command_owned(&mut self, player_id, ship_id, target)
pub fn apply_approach_command_owned(...)
...
```

認証チェックのみ違い、本体ロジックはほぼ同一。バグ修正時に片方を忘れるリスク。

#### H-2: コンポーネント snapshot ループの重複（combat / capacitor / lock）

```rust
// combat.rs, capacitor.rs, lock.rs でほぼ同じパターン
let ships: Vec<ShipSnapshot> = world.inner()
    .query::<(&ShipIdComp, &ShipStatsComp, &PositionComp, ...)>()
    .iter()
    .map(|(e, (id, stats, pos, ...))| ShipSnapshot { ... })
    .collect();
```

`SimWorld` に `query_ships()` ヘルパーがないため各 system が手書き。

---

### Medium

#### M-1: `main.rs` 1,208行（責務混在）

```
--benchmark        Phase 1-3 の bench ループ
--serve            シングルノードサーバー
--serve --cluster  Raft クラスターサーバー
--duel             1v1 モード
--raft-demo
--aoi-bench
build_serve_node() ヘルパー
spawn_npc_frigates() ヘルパー
シグナルハンドラ
WebSocket tick ループ
```

各 `run_*` 関数が 100〜200行。`main.rs` がモードごとの実装を丸ごと抱えている。

#### M-2: `ws_server.rs` 507行（JSON ビルドとプロトコルの混在）

WebSocket フレーム送受信ロジックと、`InitialState` / `ship_state_json` / `aoi_enter_json` などの JSON 構築が同居。JSON 構造の変更のたびに ws_server.rs が巻き込まれる。

#### M-3: `data_loader.rs` 416行（3種類のローダー混在）

`load_ship_types()` / `load_modules()` / `load_star_map()` と各中間型が同一ファイルに混在。今後データ種が増えるほど肥大する。

#### M-4: `snapshot.rs` と `checkpoint.rs` の責務が近い

```
snapshot.rs   — StateSnapshot の定義・直列化・ロード
checkpoint.rs — スナップショットの取得・保存タイミング管理
```

どちらもスナップショット関連で、どちらに何があるか探しにくい。

#### M-5: `sector_simulator_actor.rs` と `node.rs` の密結合

`SectorSimulatorActor` は `SimulationNode` の公開メソッドほぼ全てを呼ぶ薄いラッパー。`SimulationNode` の変更が即 Actor に波及する。

---

### Low

#### L-1: `clone()` 67箇所（tick ループ内を含む）

```rust
// node.rs tick ループ内で毎 Tick 実行
let lock_cmds = self.pending_bot_lock_commands.clone();
events.clone() // Actor 転送前
```

10,000 エンティティ規模では現状問題なし。将来の性能上限になりうる。

#### L-2: `star_system.rs`（dawn-core）と `star_map.rs`（dawn-simulation）の命名が紛らわしい

```
dawn-core/src/star_system.rs   — 型定義（StarSystemDef, JumpGateDef 等）
dawn-simulation/src/star_map.rs — インスタンスデータ（StarMap struct, builtin()）
```

型とデータの区別が名前から読み取りにくい。

#### L-3: `aoi.rs` が node.rs に内包されても良い規模

220行で AoI 判定のみ。今後拡張しないなら `node/aoi.rs` で十分。

---

## 改善ロードマップ

### Phase 1 — 低リスク・即効性あり

**P1-1: `_owned` メソッドの統合**

```rust
// Before: 8+ メソッドペア
pub fn apply_move_command(&mut self, ship_id, target)
pub fn apply_move_command_owned(&mut self, player_id, ship_id, target)

// After: 認証チェックを内包
pub fn apply_move_command(&mut self, ship_id, target, auth: Option<PlayerId>)
```

影響範囲: node.rs のみ。既存テスト流用可。

**P1-2: `SimWorld::query_ships()` ヘルパー追加**

combat / capacitor / lock 各 system の snapshot ループを `world.query_ships()` に置き換え。各 system で 20〜30行削減。

**P1-3: `data_loader.rs` をサブモジュール分割**

```
src/data_loader/
  mod.rs        — pub use
  ship_types.rs — ShipTypeEntry + load_ship_types()
  modules.rs    — ModuleEntry + load_modules()
  star_map.rs   — StarMapFile + load_star_map()
```

各ファイル 100〜140行に。振る舞い無変更。

---

### Phase 2 — 中リスク・効果大

**P2-1: `node.rs` をサブモジュール化（最重要）**

```
src/node/
  mod.rs            — SimulationNode struct + 最小 pub API
  tick.rs           — tick() 実装（Step 1-10）
  commands.rs       — apply_*_command 全種
  navigation.rs     — transit / jump / auto_jump / gate ロジック
  registry.rs       — module_registry / ship_type_registry / player lookups
  serialization.rs  — JSON 構築メソッド群
  spawner_logic.rs  — spawn_ship / despawn_ship
```

Rust では impl を複数ファイルに分けられないため、private モジュール関数として抽出し、public な impl メソッドから委譲する形で実装する。

**P2-2: `main.rs` のモード分割**

```
src/
  main.rs       — エントリポイント + 引数パース（50行以内）
  server/
    single.rs   — run_phase4_server()
    cluster.rs  — run_cluster_server()
    duel.rs     — run_duel_server()
  bench/
    benchmark.rs  — run_benchmark()
    aoi_bench.rs  — run_aoi_bench()
    raft_demo.rs  — run_raft_demo()
```

**P2-3: `ws_server.rs` から JSON 構築を分離**

```
src/
  ws_server.rs        — WebSocket フレーム送受信のみ（~200行目標）
  protocol/
    state_view.rs     — InitialState / AoiEnter / ShipStateJson 構築
```

---

### Phase 3 — 高リスク・将来への投資 ✅ 完了 (2026-06-19)

**P3-1: `SimulationNode` の責務分離（本格リファクタ）** ✅

Phase 2 のサブモジュール化後に型分離を実施:

```rust
pub struct SectorMap {
    star_map: Arc<StarMap>,
    jump_gates: HashMap<JumpGateId, JumpGateDef>,
    celestial_bodies: HashMap<CelestialBodyId, CelestialBodyDef>,
}

pub struct ShipRegistry {
    ship_index: HashMap<ShipId, Entity>,
    ship_type_ids: HashMap<ShipId, ShipTypeId>,
    player_ships: HashMap<PlayerId, ShipId>,
    ship_owners: HashMap<ShipId, PlayerId>,
}
```

`SimulationNode` はこれらを保持するオーナーとして残り、サイズは現在の 1/3 程度に。

**P3-2: snapshot / checkpoint 統合** ✅

```
src/persistence/
  snapshot.rs   — StateSnapshot 型定義・直列化
  checkpoint.rs — 保存・復元ライフサイクル管理
  recovery.rs   — restore_from_snapshot() + replay
```

---

## 期待効果

| 改善 | 削減できる複雑性 | 保守性向上 | テスト容易性 |
|---|---|---|---|
| P1-1 `_owned` 統合 | メソッド数 -40% | バグ修正箇所が 1 箇所に | 既存テスト流用可 |
| P1-2 `query_ships()` 追加 | 重複 ~90行削減 | ECS クエリ変更が 1 箇所 | system 単体テストが書きやすく |
| P1-3 data_loader 分割 | 1ファイル→3（各100行） | 各ローダーを独立編集可 | 各ローダーを独立テスト可 |
| P2-1 node.rs 分割 | 3,100行→200行+α | 機能追加の影響範囲を限定 | モジュール単位のテストが可能 |
| P2-2 main.rs 分割 | 1,200行→50行+6ファイル | モード追加がファイル追加で完結 | 各モード独立テスト可 |
| P2-3 ws JSON 分離 | プロトコル変更が 1 箇所 | UI 変更が ws_server に波及しない | JSON 構造のテストが容易 |

Phase 1 だけで実装コードが約 150行削減。Phase 2 完了後は `node.rs` と `main.rs` の 2 大ファイルが消滅し、新機能追加コストが現在の 1/3〜1/2 に低減する見込み。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- ECS systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- Raft 合意層（dawn-consensus）— 正しいアルゴリズム、変更リスク高
- Event sourcing 基盤（dawn-core / dawn-event-store）— 設計の核、INV-001 維持
- dawn-actor の Actor 基盤 — 将来の dawn-replication と接続予定、今は触らない
