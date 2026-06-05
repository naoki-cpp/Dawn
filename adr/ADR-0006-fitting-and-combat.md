---
id      : ADR-0006
title   : Fitting / Combat / Lock-on システムの設計
status  : accepted
date    : 2026-06-05
updated : 2026-06-05
authors : human + AI
related : ADR-0001（Event Sourcing）, ADR-0002（Actor Model）, ADR-0005（ClientCommand）, ADR-0007（マルチプレイヤー）
---

# ADR-0006 — Fitting システムと Combat システムの設計

## Context

Phase 4 Cycle 3 として EVE Online 準拠の Fitting システムと
それを前提とした Combat システムを実装する。

実装順序の決定が必要だった。
- Fitting を先にすると Combat の stat ソースが確定してからダメージ計算を実装できる
- Combat を先にするとハードコードされた stats を後から Fitting で上書きするリファクタが必要になる

→ **Fitting → Combat の順で実装する。**

---

## Decision

### 1. Fitting システム

#### コンポーネント設計（dawn-ecs）

```rust
// スロット種別（dawn-core）
pub enum SlotKind { High, Mid, Low, Rig }

// モジュール定義（dawn-core）
// FittedModule という別型は作らず、ModuleDefinition を FittingComp に直接格納する。
// ModuleDefinition がスロット・stat 差分・名前を一元管理するため冗長な型が不要。
pub struct ModuleDefinition {
    pub id         : ModuleId,
    pub name       : String,
    pub kind       : ModuleKind,
    pub slot       : SlotKind,
    pub stat_delta : StatDelta,
}

// 船の装備スロット全体（dawn-ecs）
pub struct FittingComp {
    pub high : Vec<ModuleDefinition>,
    pub mid  : Vec<ModuleDefinition>,
    pub low  : Vec<ModuleDefinition>,
    pub rig  : Vec<ModuleDefinition>,
}

// Fitting 結果として集計された最終 stat（dawn-ecs）
// ShipStatsComp を拡張して stat 集計の出力先とする。
// ベース値（装備なし時）は weapon_damage=0, weapon_range=0 とする。
// 武器能力はモジュール装備によってのみ付与される。
pub struct ShipStatsComp {
    // Phase 2 から存在するフィールド（変更しない）
    pub max_speed        : f32,
    pub thrust_magnitude : f32,

    // Phase 4 Cycle 3 で追加（ベース値はゼロ）
    pub max_hp           : f32,   // シールド + アーマー + ハル の合計
    pub weapon_damage    : f32,   // 1 発あたりのダメージ（0 = 武器なし）
    pub weapon_range     : f32,   // 有効射程（server units）
    pub weapon_cooldown  : u64,   // 発射間隔（Tick 数）
}
```

#### stat 集計フロー

```
FittingComp（装備スロット）
    ↓ apply_fitting(world, ship_id, base_stats) → stat_delta を合計
ShipStatsComp（最終 stat）= base_stats + Σ(module.stat_delta)
    ↓ Combat / Movement システムが参照
```

#### base_stats の管理（二重加算防止）

`apply_fitting` は毎回 `base_stats`（装備なし時の素の性能値）を起点として
全モジュールの delta を合計する。

`SimulationNode` が `base_stats: HashMap<ShipId, ShipStatsComp>` を保持し、
spawn 時に記録する。モジュールを追加装備するたびに `base_stats` から再集計することで
二重加算を防ぐ。

```rust
// SimulationNode のフィールド
base_stats: HashMap<ShipId, ShipStatsComp>,

// spawn 時に記録
self.base_stats.insert(ship_id, ShipStatsComp::NPC);

// fit_module 時は base_stats を起点に全モジュールを集計
let base = self.base_stats.get(&ship_id).copied().unwrap_or(ShipStatsComp::NPC);
apply_fitting(&mut self.world, ship_id, base);
```

#### モジュール定義カタログ（dawn-simulation）

標準モジュール定義は `modules.rs` に集約する。
サーバー起動時に `SimulationNode::register_module()` で登録し、
`fit_module()` が ID → 定義を解決する。

```
modules.rs に定義する標準モジュール（Phase 4）:
  MODULE_RAILGUN_SMALL  (1) : damage=25, range=1500, High slot
  MODULE_RAILGUN_MEDIUM (2) : damage=50, range=2500, cooldown+4, High slot
  MODULE_SHIELD_BASIC   (3) : max_hp+300, Mid slot
  MODULE_ARMOR_BASIC    (4) : max_hp+200, Low slot
  MODULE_AFTERBURNER    (5) : speed+150, thrust+10, Mid slot
```

#### Fitting イベント（dawn-core）

```rust
// 装備変更時に発行
pub struct ShipFitted {
    pub ship_id  : ShipId,
    pub fitting  : FittingSnapshot,  // 装備全体のスナップショット（モジュール ID リスト）
    pub tick     : Tick,
}
```

**`stats` フィールドを持たない理由:**
Replay 時は `FittingSnapshot` から `FittingComp` を復元し、`apply_fitting()` で
`ShipStatsComp` を再計算する。stat をイベントに含める必要はなく、冗長になるため省略した。
`FittingSnapshot` があれば INV-002（Event Replay で完全復元）は満たされる。

---

### 2. Combat システム

#### コンポーネント設計（dawn-ecs）

```rust
// HP 状態
pub struct HullComp {
    pub current_hp   : f32,
    pub is_destroyed : bool,
}

// 武器クールダウン追跡
pub struct WeaponComp {
    pub last_fired_tick : Tick,
}
```

#### Combat イベント（dawn-core）

```rust
pub struct WeaponFired {
    pub attacker_id : ShipId,
    pub target_id   : ShipId,
    pub damage      : f32,
    pub tick        : Tick,
}

pub struct DamageTaken {
    pub ship_id    : ShipId,
    pub amount     : f32,
    pub current_hp : f32,   // Replay 時に HullComp を復元するために必要
    pub tick       : Tick,
}

pub struct ShipDestroyed {
    pub ship_id   : ShipId,
    pub killer_id : ShipId,
    pub tick      : Tick,
}
```

#### DamageTaken の Replay 処理（INV-002）

`apply_event` で `DamageTaken` を受け取ったとき `HullComp.current_hp` を
`e.current_hp` で更新する。`WeaponFired` は ECS 状態を変えないためスキップする。

#### Combat System フロー（CLAUDE.md §6 Tick 処理順序に準拠）

```
Phase 4 Cycle 3 以降の Tick 処理:
  1. Tick カウンタをインクリメント
  2. MoveCommand / AttackCommand キューを処理する
  3. Movement System を実行する
  4. Combat System を実行する              ← Movement の後
     a. 射程内の敵を検索（O(n²)、Phase 8 で Spatial Index に移行）
     b. クールダウンが明けていれば WeaponFired を生成
     c. ターゲットの HullComp にダメージを適用し DamageTaken を生成
     d. HP ≤ 0 なら ShipDestroyed を生成し destroyed リストに積む
  5. 全イベントを EventStore に Append
  6. ReplicationBus に差分を転送
  7. TickSummary を返す
```

呼び出し元（`SimulationNode::tick()`）が `CombatResult.destroyed` を受け取り
ECS と `ship_index` から Ship を削除する。

#### プレイヤー操作（Phase 4 暫定）

- NPC は自動で射程内の最近傍 Ship を攻撃する
- プレイヤー操作による攻撃対象の指定は Phase 5 で実装（ADR-0007）
- `AttackCommand` の WsServer JSON パーサーは未実装（Phase 5 で追加）

---

### 3. コマンド（dawn-core）

```rust
pub struct FitModuleCommand {
    pub ship_id   : ShipId,
    pub slot      : SlotKind,
    pub module_id : ModuleId,
}

pub struct AttackCommand {
    pub attacker_id : ShipId,
    pub target_id   : ShipId,
}
```

Phase 5 では `PlayerId` による所有権検証が加わる（ADR-0007 参照）。

---

### 4. Crate 配置

| 型 | 配置 |
|---|---|
| `SlotKind`, `ModuleKind`, `StatDelta`, `ModuleDefinition`, `FittingSnapshot` | `dawn-core/src/fitting.rs` |
| イベント型（`ShipFitted`, `WeaponFired`, `DamageTaken`, `ShipDestroyed`） | `dawn-core/src/events.rs` |
| コマンド型（`FitModuleCommand`, `AttackCommand`） | `dawn-core/src/commands.rs` |
| `FittingComp` | `dawn-ecs/src/components/fitting.rs` |
| `HullComp`, `WeaponComp` | `dawn-ecs/src/components/combat.rs` |
| `ShipStatsComp` 拡張 | `dawn-ecs/src/components/movement.rs` |
| `apply_fitting()`, `apply_delta()` | `dawn-ecs/src/systems/fitting.rs` |
| `CombatSystem::run()` | `dawn-ecs/src/systems/combat.rs` |
| 標準モジュール定義カタログ | `dawn-simulation/src/modules.rs` |

独立クレート（`dawn-combat/` 等）は**作成しない**。
Phase 8 でスケール上の課題が生じた時点で分割を検討する。

---

---

### 3. Lock-on システム

Combat を「ロックオン」→「発射」の2フェーズに分けることをユーザーが要求した。
現状では CombatSystem が自動的に最近傍を攻撃しており、意図的な攻撃選択ができなかった。

#### コンポーネント設計（dawn-ecs）

```rust
pub enum LockState {
    Locking { remaining_ticks: u64 },  // ロック中（カウントダウン）
    Locked,                            // ロック完了 → 発射可能
}

pub struct LockEntry {
    pub target_id : ShipId,
    pub state     : LockState,
}

pub struct LockComp {
    pub entries: Vec<LockEntry>,  // 最大 ShipStatsComp::max_locks 件
}
```

#### ShipStatsComp への追加フィールド

```rust
pub lock_time  : u64,   // ロック完了までの Tick 数（モジュールで短縮可能）
pub max_locks  : u32,   // 同時ロック上限（モジュールで増加可能）
```

#### StatDelta への追加フィールド

```rust
pub lock_time_add  : i32,  // 負の値でロック時間を短縮
pub max_locks_add  : i32,  // 正の値で同時ロック上限を増加
```

対応モジュール例: `Sensor Booster I`（Mid slot）→ `lock_time -2`, `max_locks +1`

#### Lock-on イベント（dawn-core）

```rust
pub struct TargetLocked { pub locker_id: ShipId, pub target_id: ShipId, pub tick: Tick }
pub struct LockLost     { pub locker_id: ShipId, pub target_id: ShipId, pub tick: Tick }
```

#### Lock System フロー

```
LockSystem::run(world, tick, pending_commands):
  1. 既存ロックエントリを処理する
     - Locking: remaining_ticks をデクリメント。0 → Locked → TargetLocked
     - ターゲットが存在しない → LockLost → エントリ削除
  2. LockOnCommand（プレイヤー操作）を適用する
  3. NPC 自動ロック: 武器あり + スロット空き → 射程内最近傍に自動ロック開始

CombatSystem::run:
  LockComp.state == Locked のターゲットにのみ発射する（変更前: 最近傍自動攻撃）
```

#### プレイヤー操作フロー（Godot）

```
右クリック → レイキャスト → 最近傍 Ship を検出
    → connection.send_lock_on_command(player_ship_id, target_id)
    → JSON: {"type":"LockOnCommand","ship_id":1,"target_id":5}
    → ws_server.rs でパース → ClientCommand::LockOn
    → main.rs で tick_with_lock_commands() に渡す
    → LockSystem がカウントダウン開始
```

#### 更新された Tick 処理順序

```
1. Tick カウンタをインクリメント
2. コマンドキュー処理（Move / LockOn）
3. MovementSystem
4. LockSystem    ← Phase 4 Cycle 3 で追加
5. CombatSystem  ← Locked ターゲットのみ発射に変更
6. EventStore Append
7. Replication
```

---

## Consequences

### メリット

- Fitting → Combat の順で実装したことで stat 集計が確定してからダメージ計算を実装でき、リファクタが不要だった
- `ShipStatsComp` への集約で Combat System が Fitting の詳細を知らなくてよい（凝集度が高い）
- `base_stats` パターンにより複数モジュールを装備しても二重加算が起きない
- `DamageTaken.current_hp` をイベントに含めることで INV-002 を満たした Replay が可能
- 武器能力をベース値ゼロにしたことで「装備なし = 戦闘不能」が型で表現される

### 制約

- Combat System は O(n²) の射程検索を使う。10,000 ships での SLA 違反が発生した場合は Phase 8 の Spatial Index 実装を前倒しする
- `base_stats` は spawn 時に NPC / PLAYER を記録するが、将来「船種」が増えた場合は spawn API の引数を拡張する必要がある
- プレイヤーが攻撃対象を能動的に指定する UI は Phase 5 対応（ADR-0007）

---

## 実装チェックリスト

### Fitting

- [x] `dawn-core`: `ModuleId`, `ModuleKind`, `SlotKind`, `StatDelta`, `ModuleDefinition`, `FittingSnapshot` 型定義
- [x] `dawn-core`: `ShipFitted` イベント, `FitModuleCommand` コマンド
- [x] `dawn-ecs`: `FittingComp` コンポーネント
- [x] `dawn-ecs`: `ShipStatsComp` に Combat フィールド追加（ベース値はゼロ）
- [x] `dawn-ecs`: `apply_fitting()` / `apply_delta()` システム関数
- [x] `dawn-simulation`: `SimulationNode::register_module()` / `fit_module()` メソッド
- [x] `dawn-simulation`: `base_stats` パターンで二重加算を防止
- [x] `dawn-simulation`: `modules.rs` 標準モジュール定義カタログ
- [x] 全テスト通過（114/114）

### Combat

- [x] `dawn-core`: `WeaponFired`, `DamageTaken`, `ShipDestroyed` イベント
- [x] `dawn-core`: `AttackCommand` コマンド（WsServer での処理は Phase 5）
- [x] `dawn-ecs`: `HullComp`, `WeaponComp` コンポーネント
- [x] `dawn-ecs`: `CombatSystem::run()` — 射程判定 / ダメージ / 破壊
- [x] `dawn-simulation`: Tick 内に CombatSystem を組み込む（Movement の後）
- [x] `dawn-simulation`: `DamageTaken` を Replay 時に HullComp へ反映（INV-002）
- [x] `dawn-simulation`: 破壊 Ship を ECS と ship_index から自動削除
- [ ] Godot: `AttackCommand` 送信（Phase 5）
- [ ] Godot: HP ゲージ HUD / 破壊エフェクト（Cycle 3 クライアント実装）
- [ ] 全テスト通過（Godot 側）

### Lock-on

- [x] `dawn-core`: `LockOnCommand` コマンド
- [x] `dawn-core`: `TargetLocked`, `LockLost` イベント
- [x] `dawn-core/fitting.rs`: `StatDelta` に `lock_time_add`, `max_locks_add` 追加
- [x] `dawn-core/fitting.rs`: `ModuleKind::Sensor` 追加
- [x] `dawn-ecs/components/combat.rs`: `LockState`, `LockEntry`, `LockComp` 追加
- [x] `dawn-ecs/components/movement.rs`: `ShipStatsComp` に `lock_time`, `max_locks` 追加
- [x] `dawn-ecs/systems/fitting.rs`: `apply_delta` に lock フィールドのクランプ追加
- [x] `dawn-ecs/systems/lock.rs`: `LockSystem::run()` — カウントダウン / 自動ロック / プレイヤーコマンド処理
- [x] `dawn-ecs/systems/combat.rs`: 最近傍自動攻撃 → LockComp の Locked ターゲットのみ発射に変更
- [x] `dawn-simulation/modules.rs`: `Sensor Booster I` 追加
- [x] `dawn-simulation/node.rs`: `tick_with_lock_commands()`, `apply_event` で `TargetLocked` / `LockLost` Replay
- [x] `dawn-actor/client_connection.rs`: `ClientCommand` enum 導入（`Move`, `LockOn`）
- [x] `dawn-simulation/ws_server.rs`: `LockOnCommand` JSON パーサー追加
- [x] `dawn-simulation/main.rs`: コマンドを種別振り分け
- [x] `client/scripts/connection.gd`: `send_lock_on_command()` 追加
- [x] `client/scripts/main.gd`: 右クリック → ロックオン処理
- [x] `client/scripts/ship_controller.gd`: `flash_lock_indicator()` 追加
- [x] 全テスト通過（124/124）
