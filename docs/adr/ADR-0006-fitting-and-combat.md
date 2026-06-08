---
id      : ADR-0006
title   : Fitting / Combat / Lock-on / Active モジュールシステムの設計
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

// モジュールの活性化モード（dawn-core）
pub enum ActivationMode {
    /// 常時効果。装備するだけで StatDelta が適用される。
    /// 例: Shield Extender, Armor Plate, Rig
    Passive,
    /// プレイヤーがオン/オフを切り替える。
    /// オフ時は StatDelta が適用されない。武器はロック済みのときのみ発射する。
    /// 例: Weapon/Turret, Afterburner, Shield Booster
    Active,
}

// モジュール定義（dawn-core）
pub struct ModuleDefinition {
    pub id                : ModuleId,
    pub name              : String,
    pub kind              : ModuleKind,
    pub slot              : SlotKind,
    pub stat_delta        : StatDelta,
    pub activation_mode   : ActivationMode,
    /// Capacitor consumed once at cycle start (GJ). 0 for Passive modules.
    pub cap_cost_per_cycle: f32,
    /// Duration of one activation cycle (ticks). 0 for Passive modules.
    pub cycle_time_ticks  : u64,
}

// 装備スロット 1 枠（モジュール定義 + 現在の活性化状態）
pub struct FittedSlot {
    pub def            : ModuleDefinition,
    /// Active modules only. true = ON (StatDelta applied, weapon fires).
    /// false = OFF (StatDelta not applied, weapon silent).
    pub is_active      : bool,
    /// Ticks remaining in the current activation cycle.
    /// 0 = cycle over → CapacitorSystem will try to start a new cycle next tick.
    /// Not persisted in snapshots; resets to 0 on restart.
    pub cycle_remaining: u64,
}

// 船の装備スロット全体（dawn-ecs）
pub struct FittingComp {
    pub high : Vec<FittedSlot>,
    pub mid  : Vec<FittedSlot>,
    pub low  : Vec<FittedSlot>,
    pub rig  : Vec<FittedSlot>,
}

// Fitting 結果として集計された最終 stat（dawn-ecs）
// ShipStatsComp を拡張して stat 集計の出力先とする。
// ベース値は ShipTypeDefinition.base_stats から取得する（後述 §4）。
// 武器能力はモジュール装備によってのみ付与される（ベースはゼロ）。
pub struct ShipStatsComp {
    pub max_speed            : f32,
    pub thrust_magnitude     : f32,

    // HP: Shield / Armor / Hull 3層（§5 HP 3層化）
    pub max_shield           : f32,
    pub max_armor            : f32,
    pub max_hull             : f32,

    // 武器（モジュールのみで供給）
    pub weapon_damage        : f32,
    pub weapon_range         : f32,
    pub weapon_cooldown      : u64,

    // ロック
    pub lock_time            : u64,
    pub max_locks            : u32,

    // キャパシタ（Phase 6 追加）
    pub cap_max              : f32,   // 最大容量 (GJ)
    pub cap_recharge_per_tick: f32,   // 毎 Tick の回復量 (GJ/tick)
}

// キャパシタ現在値（live state、HullComp と同パターン）
pub struct CapacitorComp {
    pub current: f32,  // 現在のキャパシタ量 (GJ)
}
```

#### stat 集計フロー

```
FittingComp（装備スロット）
    ↓ apply_fitting(world, ship_id, base_stats)
    ↓   Passive モジュール    : 常に stat_delta を加算
    ↓   Active モジュール ON  : stat_delta を加算
    ↓   Active モジュール OFF : stat_delta を加算しない
ShipStatsComp（最終 stat）= base_stats + Σ(有効モジュールの stat_delta)
    ↓ Combat / Movement システムが参照
```

Active モジュールのオン/オフが変わるたびに `apply_fitting()` を再実行する。

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

#### Active モジュールの活性化イベント（dawn-core）

モジュールのオン/オフは「起きた事実」としてイベントに記録する（INV-001 / INV-002）。

```rust
/// Active モジュールがオンになった。
pub struct ModuleActivated {
    pub ship_id   : ShipId,
    pub module_id : ModuleId,
    pub slot      : SlotKind,
    pub tick      : Tick,
}

/// Active モジュールがオフになった。
pub struct ModuleDeactivated {
    pub ship_id   : ShipId,
    pub module_id : ModuleId,
    pub slot      : SlotKind,
    pub tick      : Tick,
}
```

**`is_active: bool` を 1 つのイベントにまとめない理由:**
`is_active` は状態の記述であって事実ではない。
「オンにした」「オフにした」という動作そのものをイベントとして表現することで、
Replay 時も活性化の履歴が正確に再現される（Event Sourcing の原則）。

#### コマンド（dawn-core）

```rust
pub struct ActivateModuleCommand   { pub ship_id: ShipId, pub module_id: ModuleId }
pub struct DeactivateModuleCommand { pub ship_id: ShipId, pub module_id: ModuleId }
```

`ClientCommand` enum にも `Activate` / `Deactivate` variant を追加する。

#### NPC と プレイヤーの挙動の違い

| | NPC | プレイヤー |
|---|---|---|
| 武器モジュール装備直後 | `is_active = true`（自動オン） | `is_active = false`（手動でオン） |
| ロックオン | 自動（`IsNpcComp` による） | 手動（`LockOnCommand`） |
| 武器発射条件 | `is_active == true` && Locked | `is_active == true` && Locked |

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
Phase 6 以降の Tick 処理:
  1. Tick カウンタをインクリメント
  2. コマンドキューを処理する（Move / LockOn / Activate / Deactivate）
  3. Movement System を実行する
  4. Capacitor System を実行する           ← Phase 6 追加（Movement の後）
     a. 毎 Tick: cap を recharge_per_tick 分回復
     b. Active-ON モジュールのサイクルを進行（cycle_remaining--）
     c. cycle_remaining == 0 → 新サイクル開始: cap 消費
     d. cap 不足 → 強制 OFF → ModuleDeactivated イベント生成
  5. Lock System を実行する
  6. Combat System を実行する              ← Lock System の後
     a. 射程内の敵を検索（O(n²)、Phase 8 で Spatial Index に移行）
     b. クールダウンが明けていれば WeaponFired を生成
     c. ターゲットの HullComp にダメージを適用し DamageTaken を生成
     d. HP ≤ 0 なら ShipDestroyed を生成し destroyed リストに積む
  7. Bot System を実行する                 ← Phase 6 追加（Combat の後）
  8. 全イベントを EventStore に Append
  9. ReplicationBus に差分を転送
  10. TickSummary を返す
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

---

### 4. ShipType システム（Option B 決定）

#### 設計方針

`ModuleDefinition` に対応する概念として `ShipTypeDefinition` を導入する。
船 spawn 時に `ShipTypeId` を指定することで船種固有の base_stats が決まる。

```rust
// dawn-core/ship_type.rs

pub struct ShipTypeId(pub u32);

pub enum ShipClass { Frigate, Cruiser, Battleship }

pub struct SlotLayout { pub high: u8, pub mid: u8, pub low: u8, pub rig: u8 }

/// 装備なし時の船種固有ベーススタット。
/// weapon_* は含まない（モジュール装備のみで供給）。
pub struct ShipBaseStats {
    pub max_speed            : f32,
    pub thrust_magnitude     : f32,
    pub max_shield           : f32,
    pub max_armor            : f32,
    pub max_hull             : f32,
    pub lock_time            : u64,
    pub max_locks            : u32,
    // キャパシタ（Phase 6 追加）
    pub cap_max              : f32,
    pub cap_recharge_per_tick: f32,
}

pub struct ShipTypeDefinition {
    pub id          : ShipTypeId,
    pub name        : String,
    pub class       : ShipClass,
    pub base_stats  : ShipBaseStats,
    pub slot_layout : SlotLayout,
}
```

`ShipSpawned` イベントに `ship_type_id` を追加する：

```rust
pub struct ShipSpawned {
    pub ship_id           : ShipId,
    pub sector_id         : SectorId,
    pub initial_position  : Position,
    pub ship_type_id      : ShipTypeId,   // 追加
    pub tick              : Tick,
}
```

サーバー起動時に `register_ship_type()` で登録し、
`spawn_ship(ship_type_id, pos, vel)` で参照する。

#### base_stats の変更

従来の `ShipStatsComp::NPC` / `ShipStatsComp::PLAYER` 定数は廃止。
ベーススタットは `ShipTypeDefinition.base_stats` に移動する。

```
spawn_ship(ship_type_id, ...)
    → シップタイプレジストリで ShipTypeDefinition を解決
    → base_stats = def.base_stats を ShipStatsComp に設定
    → base_stats_map に記録（Fitting の二重加算防止）
```

---

### 5. HP 3層化（Shield / Armor / Hull）

#### 設計方針

単一の `max_hp` を Shield / Armor / Hull の 3 層に分割する。

```
ダメージ適用順序:
  1. Shield が尽きるまで Shield から引く
  2. Shield = 0 → Armor から引く
  3. Armor = 0 → Hull から引く
  4. Hull = 0 → ShipDestroyed
```

#### HullComp の変更

```rust
pub struct HullComp {
    pub current_shield : f32,
    pub current_armor  : f32,
    pub current_hull   : f32,
    pub is_destroyed   : bool,
}
```

#### DamageTaken イベントの変更

```rust
pub struct DamageTaken {
    pub ship_id        : ShipId,
    pub damage         : f32,
    pub current_shield : f32,
    pub current_armor  : f32,
    pub current_hull   : f32,
    pub tick           : Tick,
}
```

`current_hp: f32` → 3 フィールドに分割（スキーマ変更）。
既存ログとの後方互換: `DamageTaken` は Phase 5 新規追加のため Upcaster 不要。

#### StatDelta の変更

```rust
// 旧: max_hp_add
// 新:
pub max_shield_add : f32,   // シールド HP への加算
pub max_armor_add  : f32,   // アーマー HP への加算
pub max_hull_add   : f32,   // ハル HP への加算
```

モジュールを種別化：
- `Shield Extender` → `max_shield_add`
- `Armor Plate`     → `max_armor_add`

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
- [x] `dawn-core`: `ActivationMode` を `ModuleDefinition` に追加（Passive / Active）
- [x] `dawn-core`: `ModuleDefinition` に `cap_cost_per_cycle`, `cycle_time_ticks` 追加（Phase 6）
- [x] `dawn-ecs`: `FittingComp`（`Vec<FittedSlot { def, is_active, cycle_remaining }>`）
- [x] `dawn-ecs`: `apply_fitting()` で Active OFF モジュールの delta をスキップ
- [x] `dawn-core`: `ShipFitted` イベント, `FitModuleCommand` コマンド
- [x] `dawn-core`: `ModuleActivated` / `ModuleDeactivated` イベント
- [x] `dawn-core`: `ActivateModuleCommand` / `DeactivateModuleCommand` コマンド
- [x] `dawn-actor`: `ClientCommand` に `Activate` / `Deactivate` variant 追加
- [x] `dawn-simulation`: NPC 武器モジュールを `is_active = true` で装備
- [x] `dawn-simulation`: CombatSystem を `is_active == true` の武器のみ発射に変更
- [x] `dawn-ecs`: `FittingComp` コンポーネント
- [x] `dawn-ecs`: `ShipStatsComp` に Combat フィールド + cap フィールド追加
- [x] `dawn-ecs`: `apply_fitting()` / `apply_delta()` システム関数（cap フィールド対応済み）
- [x] `dawn-simulation`: `SimulationNode::register_module()` / `fit_module()` メソッド
- [x] `dawn-simulation`: `base_stats` パターンで二重加算を防止
- [x] `dawn-simulation`: `modules.rs` 標準モジュール定義カタログ（11種）
- [x] `dawn-simulation`: `data/modules.toml` TOML 外部化（リビルド不要）

### Capacitor System（Phase 6 追加）

> 設計判断（サイクルベース消費 / クライアント側シミュレーション）の詳細は ADR-0011 参照。

- [x] `dawn-core`: `StatDelta` に `cap_max_add`, `cap_recharge_add` 追加
- [x] `dawn-core`: `ShipBaseStats` に `cap_max`, `cap_recharge_per_tick` 追加
- [x] `dawn-ecs/components/movement.rs`: `ShipStatsComp` に `cap_max`, `cap_recharge_per_tick` 追加
- [x] `dawn-ecs/components/combat.rs`: `CapacitorComp { current: f32 }` 追加
- [x] `dawn-ecs/systems/capacitor.rs`: `CapacitorSystem::run()` — サイクルベース cap 管理
- [x] `dawn-simulation/node.rs`: spawn 時に `CapacitorComp` 初期化
- [x] `dawn-simulation/node.rs`: Tick ループに CapacitorSystem 追加（Movement → Cap → Lock → Combat）
- [x] `data/ship_types.toml`: 全船種に cap フィールド追加
- [x] `data/modules.toml`: Active モジュールに `cap_cost_per_cycle`, `cycle_time_ticks` 追加
- [x] 全テスト通過（154/154）

### Combat

- [x] `dawn-core`: `WeaponFired`, `DamageTaken`, `ShipDestroyed` イベント
- [x] `dawn-core`: `AttackCommand` コマンド
- [x] `dawn-actor/ws_server.rs`: `AttackCommand` JSON パーサー実装済み（Phase 5）
- [x] `dawn-ecs`: `HullComp`, `WeaponComp` コンポーネント
- [x] `dawn-ecs`: `CombatSystem::run()` — 射程判定 / ダメージ / 破壊
- [x] `dawn-simulation`: Tick 内に CombatSystem を組み込む（Lock の後）
- [x] `dawn-simulation`: `DamageTaken` を Replay 時に HullComp へ反映（INV-002）
- [x] `dawn-simulation`: 破壊 Ship を ECS と ship_index から自動削除
- [x] Godot: HP ゲージ HUD / 破壊エフェクト / ロック枠線 / 被弾フラッシュ

### Lock-on

- [x] `dawn-core`: `LockOnCommand` コマンド
- [x] `dawn-core`: `TargetLocked`, `LockLost` イベント
- [x] `dawn-core/fitting.rs`: `StatDelta` に `lock_time_add`, `max_locks_add` 追加
- [x] `dawn-core/fitting.rs`: `ModuleKind::Sensor` 追加
- [x] `dawn-ecs/components/combat.rs`: `LockState`, `LockEntry`, `LockComp` 追加
- [x] `dawn-ecs/components/movement.rs`: `ShipStatsComp` に `lock_time`, `max_locks` 追加
- [x] `dawn-ecs/systems/fitting.rs`: `apply_delta` に lock フィールドのクランプ追加
- [x] `dawn-ecs/systems/lock.rs`: `LockSystem::run()` — カウントダウン / 自動ロック / プレイヤーコマンド処理
- [x] `dawn-ecs/systems/combat.rs`: LockComp の Locked ターゲットのみ発射
- [x] `dawn-simulation/modules.rs`: `Sensor Booster I` 追加
- [x] `dawn-simulation/node.rs`: `tick_with_lock_commands()`, Replay 対応
- [x] `dawn-actor/client_connection.rs`: `ClientCommand` enum（`Move`, `LockOn`, `Activate`, `Deactivate`, `Attack`）
- [x] `dawn-simulation/ws_server.rs`: 全コマンド JSON パーサー
- [x] `client/scripts/connection.gd`: `send_lock_on_command()` 追加
- [x] `client/scripts/main.gd`: 右クリック → ロックオン / F1-F8 モジュール ON-OFF
- [x] `client/scripts/ship_controller.gd`: `flash_lock_indicator()` 追加
- [x] 全テスト通過（154/154）
