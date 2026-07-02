---
id      : ADR-0035
title   : Per-Slot Module Targeting — foundation for targeted Active modules (Weapon / Tackle / Logistics)
status  : accepted
date    : 2026-07-02
deciders: [human, ai-agent]
related : ADR-0016 §5（戦闘の深みロードマップ・Logistics）, ADR-0033（Local Repair — Logistics の前段）,
          ADR-0006（Fitting / Combat / Lock-on）, ADR-0031（Orbit / Keep at Range）,
          CONTEXT.md, docs/architecture/tick-model.md
---

# ADR-0035 — Per-Slot Module Targeting（モジュール起動/ロック基盤の改善）

> **status: accepted** — 2026-07-02 にグリリング完了・同日実装済み（下記チェックリスト）。
> Logistics（遠隔修理）本体の実装は、本 ADR の基盤が確定してから着手する。

## 背景

ADR-0016 §5 の戦闘の深みロードマップは Tackle → Signature Resolution →
Orbit/Keep at Range → **Logistics（遠隔修理）** の順で、Local Repair（ADR-0033）まで
完了し、残るは Logistics 本体のみ。

Logistics の設計セッション（`/grilling`）を進める中で、既存の「モジュール起動 /
ターゲット選択」の仕組みに以下の不整合が見つかった:

- **Weapon**: `FittedSlot` 単位のターゲットを持たず、船全体で `LockComp::first_locked()`
  （ロックリストの先頭）を全タレット共通の攻撃対象として使う。射程外でも `is_active` は
  落ちず、命中判定側で「射程外なら確定ミス」として扱うだけ（`ModuleDeactivated` は発行されない）。
- **Tackle**: 同じく船全体でロック対象を見るが、射程外になると `TackledComp` 側の効果だけ
  外れ（`TackleReleased` 発行）、`FittedSlot.is_active` 自体は true のまま維持される。
- **Logistics（未実装）**: EVE 同様「複数の味方をロックしつつ、今どの艦を回復するか」を
  モジュールごとに選ぶ戦術的判断が本質であり、船全体の代表ターゲット方式（`first_locked()`）
  では成立しない。モジュールごとの明示的ターゲット選択が必須。

Logistics だけ非対称な仕組み（Weapon/Tackle は船全体、Logistics だけ per-slot）で実装する
案も検討したが、**「射程外で自動 OFF にならない」という既存の Weapon/Tackle の挙動自体が
そもそも直すべき不整合である**という判断に至り、Logistics 実装より前に、3 種のモジュールが
共有できる「per-slot ターゲット + 射程外自動 deactivate」の基盤を先に作ることにした。

## 決定

### 1. `LockComp` の役割 — 「ロック済み候補プール」

`LockComp`（`Locking → Locked` の時間コストを持つ既存の投資概念）は廃止しない。
役割を明確化する: **ロック済みターゲットの候補プール**を管理するだけにする。
どのロック済み候補を実際にどのモジュールの効果対象にするかは、モジュール側が選ぶ。

### 2. `FittedSlot` に per-slot ターゲットを持たせる

`FittedSlot` に `target_ship_id: Option<ShipId>` を追加する。モジュールの効果適用は
船全体の代表ターゲットではなく、このフィールドを参照する。

制約（Q4 由来）: `target_ship_id` は必ず `LockComp` の中で `Locked` 状態のエントリで
なければならない。未ロックの船を指定した場合はコマンドを却下する。

Weapon にも per-slot ターゲットを導入する（Q7 で確認済み）。個別ターゲッティングは
Logistics だけの特例ではなく、Weapon/Tackle/Logistics 共通の基盤として扱う。

### 3. `ActivateModuleCommand` へのターゲット指定方法

`ActivateModuleCommand` に `target_ship_id: Option<ShipId>` を共通フィールドとして
追加する。`ModuleKind`（または `ModuleDefinition` の新フィールド、例えば
`requires_target: bool`）側に「ターゲット必須 / 不可 / 任意」の性質を持たせ、
サーバ側でバリデーションする（例: Local Repair に `Some(..)` が来たら却下、
Weapon/Tackle/Logistics に `None` が来たら却下）。

コマンド種別を分ける案（例: `ActivateModuleCommand` と
`ActivateTargetedModuleCommand`）は採らない。Q3（Local Repair 設計時点の踏襲）と
同じ理由で、クライアント側が「どちらのコマンドを送るべきか」をモジュール種別ごとに
分岐する二重管理を避けるため。

### 4. Range Gate System — 新しい共通 Tick ステップ

射程外になったモジュールを自動的に `deactivate` する処理を、Weapon / Tackle / Logistics
共通の新しい Tick ステップとして 1 箇所に実装する（Q7）。

- 配置は Lock System（Step 5）の直後、Combat/Tackle/Repair より前（新 Step 5.5）。
  各 Step は「入る時点で射程内が保証されている」前提を置ける。
- 既存の唯一の強制 deactivate 前例である `capacitor.rs::deactivate_modules()`
  （Cap 不足時に `ModuleDeactivated` を発行し、呼び出し側が `apply_fitting()` を
  再実行するパターン）を踏襲する。
- 対象ターゲットの取得方法は `ModuleKind` ごとに差し替え可能な関数として抽象化する
  （Weapon/Tackle は `FittedSlot.target_ship_id`、Logistics も同じフィールドを見る —
  §2 により全モジュールが per-slot ターゲットに統一されるため、取得ロジック自体は
  もはや非対称ではない）。
- 射程は `ModuleKind` ごとに専用フィールドを持つ（Q5）: 既存の `weapon_range` /
  `tackle_range` に加え、新規 `repair_range`（`data/modules.toml` に `repair_range_add`、
  `StatDelta` に `repair_range_add: f32` を追加し、`weapon_range_add`/`tackle_range_add`
  と同じ変換チェーンに乗せる）。
- 射程外に出た場合の挙動（Q6 → Q6' で範囲を拡大）: 該当モジュールを強制 OFF にし、
  `ModuleDeactivated` を発行する。**Weapon・Tackle・Logistics の 3 種で統一**する
  （現状の Weapon=確定ミス, Tackle=効果だけ外れる, という不整合を解消する）。

### 5. Logistics（遠隔修理）本体は次 ADR

Logistics 本体（`RepairApplied` を他船に適用する Command/Event/Tick 配線）は、
本基盤が確定してから別 ADR として起票する。ADR-0033 の設計踏襲（`repair_cycles_started`
方式、`RepairApplied` イベント形状、Repair System の Tick 位置）はそのまま使える想定。

## 未着手・次セッションで検討

- **Weapon の複数タレット個別ターゲット選択 UI** は未着手。今回のクライアント実装は
  「F1-F8 の Weapon/Tackle トグルに、現在の単一ロックターゲット（`player_lock_target`）を
  自動的に添付する」までに留めた（複数ロック中に個々のタレットへ別々のターゲットを
  割り当てる UI は Logistics 実装時にまとめて検討する）。
- **`repair_range_add`** はまだ追加していない。消費する `ModuleKind`（Logistics の
  Repairer）がまだ存在しないため、Logistics 本体の ADR で導入する。
- 既存 Weapon/Tackle の挙動変更（確定ミス/効果外れ→強制 OFF）による既存プレイテスト
  体験への影響は、実装後の playtest で評価する。

## 実装チェックリスト

- [x] `dawn-ecs`: `FittedSlot.target_ship_id: Option<ShipId>` 追加
- [x] `dawn-core`: `ActivateModuleCommand.target_ship_id: Option<ShipId>` 追加 + バリデーション
      （`ModuleKind::requires_target()` でターゲット要否を判定）
- [ ] `dawn-core`/`dawn-ecs`: `repair_range_add: f32`（`StatDelta`）+ `data/modules.toml` 反映
      — Logistics 本体の ADR に持ち越し（消費する ModuleKind が未実装のため）
- [x] `dawn-sector`: Range Gate System 新設（`node/range_gate.rs`、Step 5.5、
      Weapon/Tackle 共通。effective range は Weapon=`weapon_range+weapon_falloff`、
      Tackle=`tackle_range`）+ `capacitor.rs::deactivate_modules()` 相当のロジックを再利用
- [x] `dawn-ecs`/`dawn-sector`: 既存 Weapon（確定ミス）・Tackle（効果だけ外れる）を
      Range Gate System 経由の強制 OFF に統一
- [x] クライアント: `_toggle_module_by_index`（F1-F8 / モジュールバー共通経路）で
      Weapon/Tackle 起動時に現在のロックターゲットを `target_ship_id` として送信
- [x] `docs`: tick-model.md（Step 5.5）・event-catalog.md（`ModuleActivated.target_ship_id`）更新済み
- [x] 検証: `cargo test --workspace`（218 tests）+ `cargo fmt` + `cargo clippy --workspace --all-targets -- -D warnings`

## 却下した代替案

- **Logistics だけ per-slot ターゲット、Weapon/Tackle は `first_locked()` のまま
  （非対称のまま進める）**: いったん採用しかけたが、「射程外で自動 OFF にならない」という
  Weapon/Tackle 側の挙動自体が是正すべき不整合だと判断し、3 種統一の基盤整備を先に行う
  方針に転換（このセッションの Q8 直後の方針転換）。
- **`LockComp` を廃止し `target_ship_id` だけで完結させる**: ロックの時間コスト
  （`Locking → Locked`）という既存の投資概念・戦術的価値を失うため却下（Q9）。

## 影響

- INV-001〜006 は維持予定（新規 Tick ステップも決定論的順序に挿入し FBD-003/INV-005 を維持）。
- Weapon/Tackle の**挙動変更**を伴う（確定ミス/効果外れ → 強制 OFF）。既存プレイテスト
  体験への影響評価が実装前に必要（「未決定」節を参照）。
