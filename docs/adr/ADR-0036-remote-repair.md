---
id      : ADR-0036
title   : Remote Repair — Logistics (targeted ally repair)
status  : accepted
date    : 2026-07-03
deciders: [human, ai-agent]
related : ADR-0033（Local Repair Module — 自己修理・共通土台）, ADR-0035（Per-Slot Module
          Targeting — target_ship_id・requires_target()・Range Gate System）,
          ADR-0024（Tackle — 先行する対象指定モジュール）, ADR-0016 §5（戦闘の深みロードマップ）
---

# ADR-0036 — Remote Repair（Logistics 本体）

## 背景

ADR-0033（Local Repair）で修理サイクル・`RepairApplied` イベント・クライアント表現という
土台を確立し、ADR-0035（Per-Slot Module Targeting）で `target_ship_id` による対象指定・
`requires_target()`・Range Gate System（射程外で強制 OFF）という Weapon/Tackle 共通基盤を
確立した。ロードマップ（`docs/process/roadmap.md` §10）はこの2つの上に
「`repair_range_add` と Repairer 系 `ModuleKind` を追加するだけで乗る想定」と明記しており、
本 ADR はその想定通りの実装を記録する。

Logistics 本体（遠隔修理）で ADR-0016 §5 の戦闘の深みロードマップ
（Tackle → Signature Resolution → Orbit/Keep at Range → Local Repair → **Remote Repair**）
が一巡する。

## 決定

### 1. 新規 `ModuleKind`（Local Repair とは別種）

Local Repair（`ShieldBooster`/`ArmorRepairer`、`requires_target() == false`、常に自己対象）
と Remote Repair は **別の `ModuleKind`** に分ける。同一 kind でターゲット有無により
自己/他者を切り替える案も検討したが、`requires_target()` が bool 固定である ADR-0035 の
設計を崩さずに済み、`data/modules.toml`/`modules.rs` のモジュール定義もどちらか一方の
フィッティングに素直に対応するため、こちらを採る。

```rust
// dawn-core/src/fitting.rs
pub enum ModuleKind {
    // ...既存...
    RemoteShieldBooster,   // 味方対象。Shield 層を回復。
    RemoteArmorRepairer,   // 味方対象。Armor 層を回復。
}

impl ModuleKind {
    pub fn requires_target(self) -> bool {
        matches!(
            self,
            ModuleKind::Weapon
                | ModuleKind::Tackle
                | ModuleKind::RemoteShieldBooster
                | ModuleKind::RemoteArmorRepairer
        )
    }
}
```

ターゲット要件は Weapon/Tackle と同じ経路（ADR-0035 Q4: `LockComp` の `Locked` 状態必須）
に乗る。味方/敵の判定（faction）は現状どのモジュールにも存在しないため、Remote Repair も
同様に「Locked であること」だけを要求し、味方限定の強制は行わない（プレイヤーの選択に委ねる）。

### 2. 射程 — `repair_range_add` / `ShipStatsComp.repair_range`

```rust
// StatDelta
pub repair_range_add: f32,

// ShipStatsComp
pub repair_range: f32,
```

`tackle_range_add`/`tackle_range` と全く同じ集計経路（`apply_delta` で
`base.repair_range + delta.repair_range_add`、effective スロットのみ合算）に乗せる。
特別扱いなし。

### 3. Range Gate System への統合（新規コードなし）

`node/range_gate.rs::effective_range_for_kind` に2行追加するだけで済む：

```rust
ModuleKind::RemoteShieldBooster | ModuleKind::RemoteArmorRepairer => Some(stats.repair_range),
```

Weapon/Tackle と全く同じ Range Gate System（Step 5.5）が、射程外に出た Remote Repair も
自動的に強制 OFF する。ADR-0035 の設計時点で「Logistics はこれが要る」と明記されていた
とおりの再利用。

### 4. `RepairCycle` に `target_ship_id` を追加

Local Repair は「サイクルを回している船 = 回復対象」だが、Remote Repair は別の船が対象。
`RepairCycle`（`dawn-ecs/src/systems/repair.rs`）に `target_ship_id: ShipId` を追加し、
Capacitor System 側で

```rust
target_ship_id: slot.target_ship_id.unwrap_or(snap.ship_id)
```

とする。Local Repair の `FittedSlot.target_ship_id` は `requires_target() == false` のため
常に `None`（ADR-0035 のアクティベーション時バリデーションが保証）で、`unwrap_or` が
そのまま自己対象にフォールバックする。Remote Repair は常に `Some` なので他船が対象になる。
**Repair System 自体のロジック分岐（自己 vs 他者）は不要** — `target_ship_id` で探す
`RepairSnap` が偶然自分自身であるかどうかだけの違いになる。

Repair System の対象検索は「サイクルを回した船」ではなく「`target_ship_id` の船」の
`HullComp`/`ShipStatsComp` を引く形に変更する（既存の自己対象ケースも同じコードパスを通る）。

### 5. モジュール定義

`dawn-sector/src/modules.rs` と `data/modules.toml` の双方に追加（ADR-0033 と同じ二重管理、
既存のとおり）：

| モジュール | kind | slot | cap_cost | cycle_time | repair_amount | repair_range_add |
|---|---|---|---|---|---|---|
| Small Remote Shield Booster I | RemoteShieldBooster | Mid | 55.0 | 8 | 50.0 | 15,000 |
| Small Remote Armor Repairer I | RemoteArmorRepairer | Low | 50.0 | 8 | 45.0 | 15,000 |

Local Repair の同格モジュール（Small Shield/Armor, repair_amount 60/55, cap 45/40）より
やや non-cap-efficient・射程が有限（15 km）にすることで、「自己修理 vs 味方支援」の
トレードオフを作る。射程は Fold Disruptor（20 km）よりやや短く、Weapon の
falloff 込み最大射程（Small Railgun 5 km、Medium 4 km 程度）より大幅に長い —
Logistics 船が前線武器の射程外から支援できる EVE 準拠の距離感。

### 6. Bot AI — 対象外（ADR-0033 §7 を踏襲）

ADR-0033 と同じ理由でスコープ外。bot が Remote Repair を活用する判断ロジックは別途検討。

### 7. クライアント表現 — 対象外（本 ADR ではサーバー実装のみ）

`RepairApplied` イベント自体は既存のまま（`ship_id` が回復対象になるだけ）なので、
`flash_repair` 等の既存クライアント処理はそのまま動作する。ただし「誰が誰を回復したか」
の視覚的な表現（例: 味方への回復ビーム）は本 ADR のスコープ外とし、別途起票する。

## 却下した代替案

- **既存 `ShieldBooster`/`ArmorRepairer` を再利用し `target_ship_id` の有無で自己/他者を
  切り替え**: `requires_target()` が `ModuleKind` 単位の bool 固定という ADR-0035 の設計と
  整合しない（1つの kind が「ターゲット任意」になり、既存の起動時バリデーション
  `kind.requires_target() != target.is_some()` を kind 単位から slot 単位の条件分岐に
  変える必要が生じる）。新規 kind を切ったほうが既存コードへの影響が小さい。
- **味方判定（faction）を導入**: 現状のコードベースにその概念がなく、本 ADR の射程外。
  Locked であれば誰でも回復対象にできる（Weapon/Tackle と対称）。

## 実装チェックリスト

- [x] `dawn-core`: `ModuleKind::RemoteShieldBooster`/`RemoteArmorRepairer` 追加 +
      `requires_target()` に追加 + テスト
- [x] `dawn-core`: `StatDelta.repair_range_add: f32` 追加
- [x] `dawn-ecs`: `ShipStatsComp.repair_range: f32` 追加 + `apply_delta` で集計
- [x] `dawn-ecs`: `RepairCycle.target_ship_id: ShipId` 追加
- [x] `dawn-ecs`: Capacitor System で `target_ship_id: slot.target_ship_id.unwrap_or(snap.ship_id)`
- [x] `dawn-ecs`: Repair System が `target_ship_id` の船を回復するよう変更（自己/他者で
      分岐なし）+ テスト
- [x] `dawn-sector`: `range_gate.rs::effective_range_for_kind` に2 kind 追加
- [x] `data/modules.toml` / `dawn-sector/src/modules.rs`: Remote Shield Booster / Remote
      Armor Repairer エントリ追加
- [x] `docs`: event-catalog（`ModuleActivated`/`ModuleDeactivated` の対象 kind 拡大の
      注記）・tick-model（変更なし、既存 Step 6.5 のまま）・roadmap（Logistics 完了）
- [x] 検証: `cargo test --workspace` + `fmt` + `clippy -D warnings`

## 影響

- INV-001〜006 は維持（新規イベントなし、既存 `RepairApplied`/`ModuleActivated`/
  `ModuleDeactivated` の対象範囲が広がるのみ）。
- Tick 処理順序は変更なし（Step 6.5 Repair System は既存のまま）。
- クライアント: `target_ship_id` を伴う `ActivateModuleCommand` 送信は ADR-0035 で
  Weapon/Tackle 用に既に実装済みの経路（`_toggle_module_by_index`）がそのまま使える
  ため、クライアント側コード変更は不要（新 kind の判定 `requires_target` 相当の
  分岐をクライアントが持っていないか要確認 — 持っていれば1行追加）。
