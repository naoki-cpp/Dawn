---
id      : ADR-0033
title   : Local Repair Module — active self-repair (Shield Booster / Armor Repairer)
status  : accepted
date    : 2026-06-24
deciders: [human, ai-agent]
related : ADR-0006（モジュール装備）, ADR-0016 §5（戦闘の深みロードマップ・Logistics の前段）, ADR-0012（命中・HP モデル）, AI_DEVELOPMENT_GUIDE.md §1/§6
---

# ADR-0033 — Local Repair Module（アクティブ自己修理）

> **status: accepted** — 2026-06-24 に実装済み。

## 背景

ADR-0016 §5 の戦闘の深みロードマップは Tackle → Signature Resolution →
Orbit/Keep at Range → **Logistics（遠隔修理）** の順。Orbit/Keep at Range
（ADR-0031）まで完了し、残るは Logistics。

Logistics の本体は「ロックした味方を遠隔修理する」だが、これは以下を一度に要求する：
修理サイクル基盤・HP 回復イベント・回復の視覚表現・**他船をターゲットした効果適用**。
最後の要素（他船への効果）は新規で、命中式や AoI とも絡む。

そこで **まず「ローカルリペア」（自己修理）を切り出す**。ターゲット選択が不要で、
修理サイクル・`RepairApplied` イベント・クライアント表現という Logistics と共通の
土台を、最小スコープで先に通す。遠隔修理（味方ターゲット）は別 ADR で本土台の上に積む。

ゲーム的には、現状 HP は減る一方（`DamageTaken` のみ）で回復手段がない。
ローカルリペアは「キャパシタを武器/AB と取り合いながら HP を保つ」判断を生み、
Keep at Range（被弾を減らす）と組み合わさって防御の選択肢を作る。

## 決定（提案）

### 1. モジュール — アクティブな Shield Booster / Armor Repairer

`ModuleKind::ShieldBooster` / `ArmorRepairer` は既存だが、現状はすべて
`activation_mode = "Passive"`（`max_shield_add` / `max_armor_add` の最大 HP バフ）。
これに **Active 版**を追加する：サイクル開始ごとにキャパシタを消費し、
その層の **現在 HP** を一定量回復する。

- `StatDelta` に `repair_amount: f32` を追加（既定 0）。
- Active な ShieldBooster → `current_shield` を回復（`max_shield` で頭打ち）。
- Active な ArmorRepairer → `current_armor` を回復（`max_armor` で頭打ち）。
- ハル層は局所修理の対象外（EVE 準拠：構造材修理はレア）。
- `data/modules.toml` に Active 版エントリを追加（cap_cost / cycle_time / repair_amount）。

層特化にするのは EVE 準拠であり、Shield 艦／Armor 艦というフィッティング分化を生むため。

### 2. HullComp に層特化の回復メソッド

```rust
// crates/dawn-ecs/src/components/combat.rs
pub fn repair_shield(&mut self, amount: f32, max_shield: f32) -> f32 // 回復後の current_shield
pub fn repair_armor (&mut self, amount: f32, max_armor:  f32) -> f32
```

`apply_damage`（既存）の対称。`max_*` は `ShipStatsComp`（フィッティング後の最大値）から渡す。

### 3. キャパシタシステムの拡張 — 修理サイクル開始の収集

`CapacitorResult` は既に `weapon_cycles_started: Vec<ShipId>`（Weapon が今 Tick に
サイクル開始した船）を返す。これと同じ仕組みで、**修理モジュールがサイクル開始した船**
を集める。キャパシタ不足での強制 OFF（`ModuleDeactivated`）は既存ロジックがそのまま効く
（武器と同じ扱い）ので、新規の cap ガードは不要。

案: `weapon_cycles_started` を `cycles_started: Vec<(ShipId, flat_idx, ModuleKind)>` 的な
汎用形へ寄せるか、`repair_cycles_started` を並置するか。**既存 Combat 連携を壊さない**
ため、まずは `repair_cycles_started` 並置を採る（Combat の `weapon_cycles_started` 依存はそのまま）。

### 4. Repair System — 新 Tick ステップ

Combat（Step 6）の **後** に Repair System を置く（例: Step 6.5）。

```
4.   Capacitor System          ← サイクル開始（武器/修理）を確定
...
6.   Combat System             ← 被弾を適用（DamageTaken）
6.5  Repair System（新規）      ← 修理サイクル開始船に回復を適用（RepairApplied）
7.   Bot System
```

Combat の後に置くのは、「同 Tick の被弾を受けてから回復」＝修理がその Tick を救える挙動
にするため（順序は ADR 確定事項。`tick-model.md` §3 / `AI_DEVELOPMENT_GUIDE.md` §6 に追記）。
Repair System は `repair_cycles_started` を読み、各船の `HullComp` を `ShipStatsComp` の
最大値で clamp して回復し、`RepairApplied` を発行する。

### 5. 新イベント `RepairApplied`

`DamageTaken` と対称の形：

```rust
pub struct RepairApplied {
    pub ship_id: ShipId,
    pub amount: f32,        // 実際に回復した量（clamp 後）
    pub layer: RepairLayer, // Shield | Armor（どの層か）
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub tick: Tick,
}
```

`DamageTaken` を負ダメージで再利用しない理由：(1) クライアント表現が異なる（緑フラッシュ vs 赤）、
(2) イベントログ（INV-001 append-only 履歴）の意味が明確、(3) `layer` で層を持てる。
`event-catalog.md` に追記。

### 6. クライアント表現

- `RepairApplied` 受信で対象船を **緑フラッシュ**（`flash_damage` の対称 `flash_repair`）。
- HP バー更新（既存 `_ship_hp` 更新パスに相乗り）。
- モジュールバーは既存のアクティブ表示・cap サイクルがそのまま使える（追加 UI 不要）。

### 7. Bot AI（任意・本 ADR では最小）

低 HP 時に逃走（warp）の前段として修理モジュールを ON にする選択は自然だが、
スコープを絞るため**本 ADR では bot の修理活用は含めない**（別途検討）。

## 実装チェックリスト

- [x] `dawn-core`: `StatDelta.repair_amount` 追加・`RepairApplied` イベント・`RepairLayer` 列挙 + テスト
- [x] `dawn-ecs`: `HullComp::repair_shield/repair_armor` + テスト
- [x] `dawn-ecs`: Capacitor System に `repair_cycles_started` 収集を追加 + テスト
- [x] `dawn-ecs`: Repair System 実装（回復適用 + `RepairApplied` 発行）+ テスト
- [x] `dawn-sector`: Tick Step 6.5 に Repair System を配線（Combat の後）
- [x] `data/modules.toml`: Active な Shield Booster / Armor Repairer エントリ追加
- [x] クライアント: `RepairApplied` ハンドラ・`flash_repair`・HP バー更新
- [x] `docs`: event-catalog（RepairApplied）・tick-model（Step 6.5）・roadmap
- [x] 検証: `cargo test --workspace`

## 却下した代替案

- **`DamageTaken` を負ダメージで再利用**: §5 の理由（表現・履歴の明確さ・層情報）で却下。
- **ハル層も局所修理可能にする**: EVE 準拠で局所はシールド/アーマーのみ。ハル修理は将来の構造材/ステーション系で検討。
- **最初から遠隔修理（Logistics 本体）を実装**: ターゲット選択・他船効果・AoI 連携を一度に抱える。
  ローカルを先に通して土台（修理サイクル・RepairApplied）を確立し、遠隔は別 ADR で積む。
- **Repair を Combat の前に置く**: 同 Tick の被弾前に回復しても上限で頭打ちになりがちで、
  「被弾を受けてから救う」というアクティブ修理の手応えが出ない。Combat 後に置く。

## 影響

- INV-001〜006 は維持（`RepairApplied` は append-only な事実イベント、State は再現可能）。
- 新 Tick ステップ（6.5）は決定論的順序に挿入（FBD-003 / INV-005 維持）。
- 既存の Combat / Capacitor の挙動は変えない（`weapon_cycles_started` 依存はそのまま）。
