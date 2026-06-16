---
id      : ADR-0024
title   : Tackle — Fold Disruptor Module
status  : accepted
date    : 2026-06-16
deciders: [human, ai-agent]
related : ADR-0006（Fitting System）, ADR-0011（Capacitor）, ADR-0022（Warp）,
          ADR-0009（Jump / Transit）, ADR-0014（Raft Consensus）,
          ADR-0016 §5（戦闘の深み）
---

# ADR-0024 — Tackle: Fold Disruptor Module

## 背景

ADR-0022 で intra-Sector ワープが実装された。「逃がさない」という戦闘の核を
成立させるには、ワープ・ジャンプ・Transit をすべて封じるモジュールが必要である。
EVE Online の Warp Disruptor に相当する。

lore: **Fold Disruptor** — ターゲットの Fold Drive（ワープ・ジャンプ）を妨害する
      高エネルギー干渉場を展開するモジュール。

## 決定

### 1. TackledComp（dawn-ecs）

```rust
/// Set of ships currently tackling this ship.
/// Ship is tackled as long as this component exists and is non-empty.
/// Persisted in ShipSnapshot (unlike WarpComp/ApproachComp) because
/// losing tackle state on restart would allow the tackled ship to escape.
#[derive(Debug, Clone)]
pub struct TackledComp {
    pub tacklers: Vec<ShipId>,
}
```

- `tacklers` が空でない限り tackled 状態。
- **スナップショットに含める**（INV-002 準拠）。WarpComp と違い tackle 状態の
  消失はゲームプレイ上の実害（逃亡）を生むため。
- 複数の tackler に対応。

### 2. StatDelta 拡張（dawn-core）

```rust
pub struct StatDelta {
    // ...既存フィールド...
    /// Tackle range added by this module (units). 0 = no tackle capability.
    pub tackle_range_add: f32,
}
```

### 3. ModuleKind 拡張（dawn-core）

```rust
pub enum ModuleKind {
    Weapon,
    Shield,
    Armor,
    Propulsion,
    Tackle,   // ← 追加
}
```

### 4. 新イベント（dawn-core）

```rust
/// A ship begins tackling another ship (Fold Disruptor activated in range).
pub struct TackleApplied {
    pub ship_id : ShipId,   // tackled ship
    pub by      : ShipId,   // tackler
    pub tick    : Tick,
}

/// A ship's tackle on another ship ended (module off / out of range / destroyed).
pub struct TackleReleased {
    pub ship_id : ShipId,   // formerly tackled ship
    pub by      : ShipId,   // former tackler
    pub tick    : Tick,
}
```

### 5. Tick Step 4.5 — Tackle System（dawn-ecs / dawn-simulation）

処理順: Capacitor System（Step 4）の後、Lock System（Step 5）の前。

```
for each ship with an active Tackle module (ModuleKind::Tackle, is_active=true):
    tackle_range = ShipStatsComp.tackle_range (= Σ tackle_range_add)
    locked_targets = LockComp の locked 状態のターゲット一覧
    for each locked_target:
        dist = pos(self) - pos(target)
        if dist <= tackle_range:
            if self not in target.TackledComp.tacklers:
                target.TackledComp.tacklers.push(self)
                emit TackleApplied { ship_id: target, by: self, tick }
        else:
            if self in target.TackledComp.tacklers:
                target.TackledComp.tacklers.remove(self)
                if target.TackledComp.tacklers.is_empty():
                    remove TackledComp from target
                emit TackleReleased { ship_id: target, by: self, tick }

for each ship NOT holding an active Tackle module
    (module deactivated / cap-forced off / destroyed this tick):
    remove self from all TackledComp.tacklers we appear in
    emit TackleReleased for each removal
```

ShipDestroyed 時: tackler または tackled が破壊されたら TackleReleased を発行しコンポーネントを除去。

### 6. バリデーション変更（dawn-simulation）

```rust
pub fn can_propose_warp(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
    // ...既存チェック...
    // NEW: tackled ships cannot warp
    if self.world.is_tackled(ship_id) { return false; }
    // ...
}

pub fn can_propose_jump(&self, ship_id: ShipId, gate_id: JumpGateId) -> bool {
    // ...既存チェック...
    // NEW: tackled ships cannot jump
    if self.world.is_tackled(ship_id) { return false; }
    // ...
}

// propose_transit (Raft 経由) も同様に拒否
```

### 7. ShipStatsComp 拡張（dawn-ecs）

```rust
pub struct ShipStatsComp {
    // ...既存フィールド...
    /// Effective tackle range (units) after active Tackle modules.
    pub tackle_range: f32,
}
```

### 8. データ定義（data/modules.toml）

```toml
[[modules]]
id                 = 12
name               = "Fold Disruptor I"
kind               = "Tackle"
slot               = "Mid"
activation_mode    = "Active"
cap_cost_per_cycle = 30.0
cycle_time_ticks   = 10
[modules.stat_delta]
tackle_range_add = 20000.0
```

### 9. 射程

**20,000 units**（ゲート activation_radius = 2,000 の 10 倍）。

### 10. ロック要件

**必須**。Tackle System はロック済みターゲットにのみ作用する。
ロックが切れた（LockComp から消えた）場合は即座に tackle 解除。

## スナップショット設計

`TackledComp` を `ShipSnapshot` に含める。理由:

- INV-002: 派生・transient 状態はスナップショットに永続化する。
- WarpComp は失っても「船が止まる」だけだが、TackledComp を失うと
  tackled 船がワープ・ジャンプできてしまう（ゲームプレイ上の実害）。
- シリアライズコストは微小（`Vec<ShipId>` = 数バイト）。

## 実装チェックリスト

- [x] `StatDelta` に `tackle_range_add: f32` 追加（dawn-core）
- [x] `ModuleKind::Tackle` 追加（dawn-core）
- [x] `ShipStatsComp` に `tackle_range: f32` 追加（dawn-ecs）
- [x] `apply_fitting()` で `tackle_range` を集計（dawn-ecs）
- [x] `TackledComp { tacklers: Vec<ShipId> }` 追加（dawn-ecs）
- [x] `SimWorld::is_tackled()` ヘルパー追加（dawn-ecs）
- [x] `TackleApplied` / `TackleReleased` イベント追加（dawn-core）
- [x] `DomainEvent` enum に追加（dawn-core）
- [x] `ShipSnapshot` に `tackled_by: Vec<ShipId>` 追加（dawn-simulation、フィールド名 `tackled_by`）
- [x] `take_snapshot()` / `restore_ship_from_snapshot()` で TackledComp を永続化（dawn-simulation）
- [x] `process_tackle()` 実装（dawn-simulation、Step 4.5 — HashMap desired-state diff、単一 ECS スキャン）
- [x] `can_propose_warp()` / `can_propose_jump()` に is_tackled チェック追加（dawn-simulation）
- [ ] `propose_transit()` に is_tackled チェック追加（未実装 — Raft 経由の Transit は can_propose_jump で upstream 拒否済みのため実害なし。別途検討）
- [x] ShipDestroyed 時の tackle 解除処理（`apply_event` で TackleApplied/TackleReleased を処理）
- [x] `data/modules.toml` に `Fold Disruptor I`（id=12）追加
- [x] `docs/event-catalog.md` を更新（TackleApplied / TackleReleased 追記）
- [x] テスト: tackled 中は warp/jump が拒否される（`tackled_ship_cannot_warp`）
- [x] テスト: tackler が死亡したら tackle 解除（`tackle_releases_when_tackler_dies`）
- [ ] テスト: 射程外に出たら tackle 解除（未実装）
- [ ] テスト: 複数 tackler のうち 1 人が解除しても残りが有効なら tackled 継続（未実装）
- [x] テスト: スナップショット round-trip で TackledComp が保持される（`tackle_snapshot_round_trip_preserves_tackle_state`）
