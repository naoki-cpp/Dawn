---
id      : ADR-0023
title   : Propulsion Physics — Inertia Model / Afterburner Modules
status  : proposed
date    : 2026-06-16
deciders: [human, ai-agent]
related : ADR-0006（Fitting System）, ADR-0008（INV-MOVE）, ADR-0016 §5（戦闘の深み）,
          ADR-0022（intra-Sector Warp / align time）,
          docs/reference/eve-reference.md §7.4.1,
          https://wiki.eveuniversity.org/Propulsion_equipment
---

# ADR-0023 — Propulsion Physics: Inertia Model / Afterburner Modules

## 背景

現状の移動システムは `thrust_magnitude`（加速量/tick）と `max_speed`（ハードキャップ）の
2 パラメータで動く線形モデルである。これには以下の問題がある:

1. **align time が静的**。ADR-0022 では整列条件を「max_speed の 75% 到達」とし
   「align time は船の機動性から自然に決まる」と定義したが、現在の線形加速では
   `max_speed / thrust_magnitude` ticks という粗い近似にしかならない。
   EVE の指数接近モデルと異なり、機敏さのパラメータが一本化されていない。

2. **Afterburner が表現できない**。EVE では AB / MWD が `Thrust` を追加して
   `Vmax = Vbase × (1 + Vbonus × Thrust/Mass)` を押し上げる。
   現状の `max_speed` はハードキャップであり、モジュールが上限を変動させる設計になっていない。

3. **Oversized AB が表現できない**。10MN AB を frigate に積むと速度が上がる一方で
   質量増加により align time が著しく悪化する（EVE wiki: "30-second align time"）。
   これは「速度 vs 生存性」の戦術的判断を生む重要なメカニクスである。
   現行モデルでは質量と慣性を分離していないため実現不可能。

本 ADR はこれらを解決する **推進物理モデルの刷新** を定める。

## 決定

### 1. 速度更新式を線形加算から指数接近（EVE 準拠）へ変更

#### 変更前（線形加算）

```
velocity += normalize(thrust) * thrust_magnitude
velocity = clamp(velocity, max_speed)
```

#### 変更後（指数接近）

```
τ_ticks = total_mass × inertia_modifier / MASS_SCALE
α       = 1 - exp(-1 / τ_ticks)

v_target = effective_max_speed × normalize(thrust_dir)   // 推力中
v_target = ZERO                                          // ブレーキ中

v(t+1) = v(t) + (v_target - v(t)) × α
```

`α` は 1 tick あたりの収束率。`τ_ticks` が大きい（重い・慣性大）ほど α が小さく、
速度変化が遅い。`v_target` への漸近なので **速度クランプ不要**（v_target を超えない）。

`MASS_SCALE` はゲームバランス調整定数（→ §5 具体値参照）。

### 2. 船体パラメータの変更

#### 廃止

| パラメータ | 理由 |
|---|---|
| `thrust_magnitude` | 加速度の速さと最大速度の両方を暗黙に支配していた。役割を分離するため廃止。|

#### 追加（ShipBaseStats / ShipStatsComp）

| パラメータ | 型 | 意味 |
|---|---|---|
| `base_max_speed` | f32 (units/tick) | モジュール未装備時の最大速度 |
| `mass` | f32 (kg) | 船体質量。Vmax と align time の両方に影響 |
| `inertia_modifier` | f32 | align time のみを支配する慣性係数（低いほど機敏）|

**直交性**: `base_max_speed` × モジュール → `effective_max_speed`。
`mass` × `inertia_modifier` → `τ`（align time）。それぞれ独立に調整できる。

### 3. 実効最大速度の計算

Afterburner / MWD が active のとき `effective_max_speed` を計算する。
EVE の公式 `Vmax = Vbase × (1 + Vbonus × Thrust/Mass)` を
`speed_multiplier` フィールドに統合する（§4 参照）。

```
effective_max_speed = base_max_speed
                    × Π { delta.speed_multiplier | active module }
```

モジュール未装備・全モジュール OFF の場合 `effective_max_speed = base_max_speed`。

### 4. StatDelta に追加するフィールド

```rust
pub struct StatDelta {
    // ── 既存フィールド（省略）──

    /// Multiplicative speed bonus applied when this module is active.
    /// Formula origin: EVE's (1 + Vbonus × Thrust/Mass).
    /// For a properly-sized 1MN AB: Thrust/Mass ≈ 1, so multiplier ≈ 1 + Vbonus.
    /// Default: 1.0 (no effect).
    pub speed_multiplier: f32,

    /// Mass added to the ship when this module is **fitted** (passive, always active).
    /// Increases τ_ticks → longer align time, even when the module is OFF.
    /// This is what makes oversized ABs a meaningful tradeoff:
    ///   10MN AB on a frigate → high speed when ON, terrible align time always.
    /// Default: 0.0 kg.
    pub mass_add: f32,
}
```

**`mass_add` は passive（常時適用）**: モジュールが OFF でも装着している限り質量に加算される。
これが Oversized AB のデメリット（align time 悪化）の根拠。

### 5. 具体的な数値

#### MASS_SCALE の導出

目標: Magpie（cruiser 相当）の align time ≈ 5 秒 = 50 tick（10 tick/s）

```
τ_ticks = align_time / -ln(0.25) = 50 / 1.386 ≈ 36 tick
MASS_SCALE = mass × inertia_modifier / τ_ticks
           = 12_000_000 × 0.3 / 36
           ≈ 100_000
```

```rust
/// Converts (mass_kg × inertia_modifier) to τ in ticks.
/// Derived so that a cruiser-class ship (mass=12M, inertia=0.3) has τ≈36 tick.
const MASS_SCALE: f32 = 100_000.0;
```

#### 船種パラメータ（data/ship_types.toml）

| 船種 | class | base_max_speed | mass (kg) | inertia_modifier | τ (tick) | align (秒) |
|---|---|---|---|---|---|---|
| Magpie | Cruiser | 40.0 | 12_000_000 | 0.3 | 36 | 5.0 |
| ※小型（frigate） | Frigate | 60.0 | 1_500_000 | 0.4 | 6 | 0.8 |
| ※大型（battleship） | Battleship | 20.0 | 90_000_000 | 0.15 | 135 | 18.7 |

※ frigate / battleship は将来追加予定。現状は Magpie のみ。

#### モジュールパラメータ（data/modules.toml）

| モジュール | speed_multiplier | mass_add (kg) | cap_cost | 用途 |
|---|---|---|---|---|
| 1MN Afterburner I | 2.35 | 0 | 10 | frigate 適正サイズ |
| 10MN Afterburner I | 2.35 | 8_000_000 | 25 | frigate 搭載 → Oversized AB |
| 100MN Afterburner I | 2.35 | 80_000_000 | 60 | cruiser 搭載 → Oversized AB |

**Oversized AB の例（10MN AB on Magpie、mass=12M kg）:**

```
total_mass           = 12_000_000 + 8_000_000 = 20_000_000 kg
τ_ticks              = 20_000_000 × 0.3 / 100_000 = 60 tick
align_time           = 60 × 1.386 / 10 ≈ 8.3 秒  （通常の 5.0 秒から悪化）
effective_max_speed  = 40.0 × 2.35 = 94.0 u/t    （AB ON 時）
```

### 6. apply_fitting() の更新

```rust
// In apply_fitting() (dawn-ecs/src/systems/fitting.rs):

// Total mass = base + ALL fitted modules' mass_add (passive, always).
let total_mass: f32 = base_stats.mass
    + slots.iter().map(|s| s.module.stat_delta.mass_add).sum::<f32>();

// τ and α for movement system.
let tau_ticks = total_mass * stats.inertia_modifier / MASS_SCALE;
// Store τ_ticks in ShipStatsComp so MovementSystem can read it.
stats.tau_ticks = tau_ticks.max(1.0);  // clamp to avoid div-by-zero

// Effective max speed = base × product of active speed multipliers.
let speed_mult: f32 = slots.iter()
    .filter(|s| s.is_active)
    .map(|s| s.module.stat_delta.speed_multiplier)
    .product();
stats.max_speed = base_stats.base_max_speed * speed_mult;
```

### 7. ShipStatsComp の変更

```rust
pub struct ShipStatsComp {
    // ── Movement ──
    pub base_max_speed    : f32,   // hull base (no modules)
    pub max_speed         : f32,   // effective (base × active multipliers)
    pub mass              : f32,   // kg, base hull mass
    pub inertia_modifier  : f32,
    pub tau_ticks         : f32,   // precomputed τ; updated by apply_fitting()

    // ── (thrust_magnitude 削除) ──

    // ── HP, Combat, Lock, Capacitor（変更なし）──
    // ...
}
```

`tau_ticks` は `apply_fitting()` が毎回再計算する派生値。
MovementSystem は `tau_ticks` を読んで `α` を算出するだけ。

### 8. MovementSystem の更新

```rust
// Compute α from precomputed τ.
let alpha = 1.0_f32 - (-1.0 / stats.tau_ticks).exp();

let v_target = if thrust_comp.is_braking {
    Velocity::ZERO
} else {
    let mag = magnitude(thrust_comp.direction);
    if mag > f32::EPSILON {
        let scale = stats.max_speed / mag;
        Velocity {
            dx: thrust_comp.direction.dx * scale,
            dy: thrust_comp.direction.dy * scale,
            dz: thrust_comp.direction.dz * scale,
        }
    } else {
        vel_comp.0  // no thrust → coast (v_target = current vel, no change)
    }
};

vel_comp.0.dx += (v_target.dx - vel_comp.0.dx) * alpha;
vel_comp.0.dy += (v_target.dy - vel_comp.0.dy) * alpha;
vel_comp.0.dz += (v_target.dz - vel_comp.0.dz) * alpha;
// No explicit clamp needed — exponential approach cannot exceed v_target.
```

**推力なし（コースト）の扱い**: `thrust_direction = ZERO` のとき `v_target = vel_comp.0`
とすることで `(v_target - v) = ZERO` となり速度が変化しない（慣性飛行）。
EVE の「スラスターを切ると等速直線運動」と同じ挙動。

### 9. ADR-0022（Warp align）との関係

ADR-0022 では「align time は船の機動性（thrust / max_speed）から自然に決まる」と記述していたが、
本 ADR で **「align time は τ_ticks から自然に決まる」** に精緻化される。

align time（75% に達するまでの tick 数）:

```
align_ticks = -ln(0.25) × τ_ticks = 1.386 × τ_ticks
```

これは固定タイマーではなく `total_mass × inertia_modifier` から導出されるため、
ADR-0022 の設計意図（「整列時間は機動性から自然に決まる → Tackle 窓の長さは船種依存」）を
より忠実に実現する。

## 却下した選択肢

### A: `thrust_magnitude` を残して `inertia_modifier` を乗数として追加

```rust
// 却下案
effective_thrust = thrust_magnitude / inertia_modifier;
velocity += effective_thrust * dir;
```

**却下理由**: `thrust_magnitude` が「加速の速さ」と「最大速度」の両方を
暗黙に担う構造が変わらない。また Afterburner で max_speed が上がる仕組みと
整合しない（thrust_magnitude を増やしても max_speed ハードキャップが変わらないため）。

### B: `speed_multiplier` を `thrust × velocity_bonus` の 2 フィールドに分離

EVE の公式 `(1 + Vbonus × Thrust/Mass)` を忠実に再現する案。

**却下理由**: `Thrust/Mass` は船体 mass に依存するため apply_fitting() が複雑になる。
また EVE wiki が「適正サイズなら Thrust/Mass ≈ 1」と明示しているため、
`speed_multiplier = 1 + Vbonus` への統合で実用上の損失はない。
Oversized AB の `mass_add` によって align time 悪化は別途表現できる。

### C: `α = 1 / τ_ticks`（一次近似）

`α = 1 - exp(-1/τ)` の代わりに `1/τ` を使う近似案。

**却下理由**: 機敏な船（τ_ticks ≈ 5）で 10% 程度の誤差が生じ、
align time の計算式 `1.386 × τ` が成立しなくなる。
正確な式を使う計算コストはほぼゼロ（`exp` は 1 call/ship/tick）。

## 実装チェックリスト

- [ ] `dawn-core/src/ship_type.rs`: `ShipBaseStats` に `base_max_speed`, `mass`, `inertia_modifier` 追加、`thrust_magnitude` 削除
- [ ] `dawn-core/src/events.rs` / `modules.rs`: `StatDelta` に `speed_multiplier`, `mass_add` 追加
- [ ] `dawn-ecs/src/components/movement.rs`: `ShipStatsComp` を更新（`tau_ticks` 追加、`thrust_magnitude` 削除）
- [ ] `dawn-ecs/src/systems/fitting.rs`: `apply_fitting()` で `tau_ticks`, `max_speed` を再計算
- [ ] `dawn-ecs/src/systems/movement.rs`: 指数接近モデルに変更、speed clamp 削除
- [ ] `data/ship_types.toml`: Magpie に `mass`, `inertia_modifier` 追加、`thrust_magnitude` 削除
- [ ] `data/modules.toml`: AB 各サイズに `speed_multiplier`, `mass_add` 追加
- [ ] `crates/dawn-simulation/src/node.rs`: `ShipStatsComp::PLAYER/NPC` fallback 定数を更新
- [ ] `client/scripts/main.gd`: align time / speed 表示の更新（必要であれば）
- [ ] `docs/event-catalog.md`: StatDelta フィールド更新
- [ ] CLAUDE.md §1 Scope に `inertia_modifier`, `mass` を追記
- [ ] 全テストがゼロエラーで通過すること（`cargo test --workspace`）
- [ ] `ShipStatsComp::from_base()` テストで `tau_ticks` が正しく計算されることを確認
- [ ] MovementSystem テストで指数接近モデルの align time が理論値 `1.386 × τ_ticks` と一致することを確認
