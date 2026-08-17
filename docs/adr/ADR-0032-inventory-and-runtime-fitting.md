---
id      : ADR-0032
title   : Inventory and Runtime Fitting — InventoryComp / FitModuleCommand(owned) / UnfitModuleCommand
status  : accepted
date    : 2026-06-24
deciders: [human, ai-agent]
related : ADR-0006（モジュール装備システムの初出）, ADR-0016（FBD-008 撤廃・インベントリ解禁）, AI_DEVELOPMENT_GUIDE.md §1/§6/§11
---

# ADR-0032 — Inventory and Runtime Fitting

## 背景

`FitModuleCommand` / `FittingComp` は ADR-0006 から存在するが、現状の用途は
スポーン時にサーバー内部が船種の既定ロードアウトを設定するためだけであり、
クライアントから送れる経路が一切ない（`ClientCommand` に Fit/Unfit が無い）。
「所有」の概念も無く、`fit_module` は `module_registry`（船種を問わず全モジュール
定義を保持するグローバル表）から無制限に取得して即装備する、検証なしの内部 API。

ユーザー要望: 「アイテムとインベントリシステムを作ってfittingを変えられるように
したい」。ADR-0016 で FBD-008（MVP範囲外実装の禁止）が撤廃され、インベントリ /
市場・経済は ADR 起票のうえ解禁されている。市場・経済システム自体はまだ無いため、
今回はその前段——「プレイヤーが所有するモジュールの集合（インベントリ）」と
「インベントリとシップスロットの間でモジュールを移動する手段（Fit/Unfit）」——
に範囲を絞る。

## 決定

### 1. 所有モデル: 固定初期セット（補充なし）

経済システムが無い現状で「どこからアイテムが来るか」を発明しないため、
プレイヤーは **スポーン時に登録済みモジュール全種を1個ずつ所有**する
（`module_registry` の全エントリ、現在12種）。補充・購入・ドロップは無い
（将来 Economy/Loot フェーズで `InventoryComp` に積む経路を追加すれば足りる
——`InventoryComp` 自体は供給源を問わない単純な所持リストなので、後から
供給源を増やすのは本 ADR の変更を必要としない）。

NPC ship には `InventoryComp` を付与しない（インベントリ UI が要らない・
Fit/Unfit は人間プレイヤーのみが行う操作のため）。

### 2. 換装の制約: いつでもどこでも可能（MVP）

ドッキング/セーフゾーンの概念が存在しないため、Fit/Unfit に位置・状態の制約は
設けない（Move/Stop/Approach 等の既存コマンドが Transit 中のみ拒否するのと
同様の最小限のガードのみ）。戦闘中の換装が問題になった場合は、別 ADR で
制約（速度ゼロ必須、Tackle 中拒否、等）を追加する。

### 3. データモデル

```rust
// dawn-ecs/src/components/inventory.rs
pub struct InventoryComp {
    pub items: BTreeMap<ItemId, u64>, // ItemIdごとのスタック個数
}
```

`ItemId` は個体差のないアイテムの正規識別子であり、個数はスタック値として
保持する（ItemIdの一般化とスタック化は ADR-0034 で確定）。`FittingComp` の
各スロットへ移動する際に1個消費し、Unfitで1個返す。

### 4. コマンド

`FitModuleCommand { ship_id, module_id, slot }` は既存（dawn-core）。
新規: `UnfitModuleCommand { ship_id, module_id, slot }`（対称な形）。

既存の内部用 `fit_module()`（スポーン時の特権フィット・インベントリ不要）は
**変更しない**——挙動互換・既存テスト保護のため。新規にプレイヤー操作用の
ラッパーを追加する:

- `fit_module_owned(player_id, cmd) -> bool`:
  所有権チェック → `module_registry` に存在し `def.slot == cmd.slot` か検証 →
  船種の `SlotLayout` に対するスロット空き容量チェック → `InventoryComp` から
  1個消費（無ければ拒否）→ スロットへ push・`apply_fitting`・`ShipFitted` 発行。
- `unfit_module_owned(player_id, cmd: UnfitModuleCommand) -> bool`:
  所有権チェック → 指定スロットに該当 `module_id` のスロットが無ければ拒否 →
  そのスロットを除去・`InventoryComp` へ1個返却 → `apply_fitting`・
  `ShipFitted` 発行。

スロット容量チェックは新規ラッパーのみで行う（既存 `fit_module` は
スポーン時の確定済みロードアウトにのみ使われ、データファイル側の責任で
容量内に収まっているため、ここを変えると既存動作にリスクを負わせるだけで
得るものがない）。

### 5. イベント: `ShipFitted` にインベントリ・スナップショットを追加

新規イベント型は導入しない。既存の `ShipFitted { ship_id, fitting, tick }` に
`inventory: BTreeMap<ItemId, u64>` を追加し、装備変更と対になるインベントリ変化を
同じイベントで運ぶ（Fit/Unfit は常に両方を同時に変えるため、1イベントで
両方の結果状態を記録するのが最小の追加で済む——新規イベント型 + 新規 replay
分岐 + 新規 catalog エントリを避けられる）。

初期インベントリ（§1 の固定セット）はイベント化しない: `ShipSpawned` の
`ship_type_id` と起動時に読み込まれる `module_registry`（決定的・全ノードで
同一）から再現できるため、`HullComp`/`CapacitorComp` の初期値と同じ扱いで
spawn 時に直接構築する（`apply_event` の `ShipSpawned` 分岐でも同じ関数を
呼び、リプレイでも同じ初期インベントリを再現する）。

### 6. 永続化（スナップショット）

`ShipSnapshot`（`dawn-sector::persistence::snapshot`）に
`#[serde(default)] pub inventory: BTreeMap<ItemId, u64>` を追加する。
`#[serde(default)]` は既存スナップショットとの後方互換のため
（`tackled_by` で確立済みの規約と同じ）。

## 却下した代替案

- **市場・購入を今回一緒に作る**: 経済システムは別フェーズの大きな決定空間
  （価格形成・NPC在庫・取引UI）であり、今回の「所有してFit/Unfitできる」MVPの
  スコープを大きく超える。`InventoryComp` を供給源不問の単純なリストにしたのは、
  後で Market/Loot を足すときに本 ADR の変更が要らないようにするため。
- **ドッキング/セーフゾーン制約を今回入れる**: Station 等のエンティティ種別が
  まだ無く、前提が無い制約を先取りすると設計の手戻りリスクが大きい。MVP は
  無制約とし、問題が顕在化したら別 ADR で締める。
- **新規イベント型 `ShipInventoryChanged` を追加する**: `ShipFitted` と常に
  対になって発行されるため、2イベント化するメリットがない。既存イベントへの
  フィールド追加で足りる。
- **アイテムをスタック数ではなく個体（ユニークID）で持つ**: 現状のモジュールに
  個体差（劣化・改造）が無いため、`BTreeMap<ItemId, u64>` のスタックで十分。
  個体差が要る機能（ダメージ付きモジュール等）が出たら、その時に型を作る。

## 実装チェックリスト

- [x] dawn-core: `UnfitModuleCommand` 追加・スタック型の `ShipFitted.inventory` 追加
- [x] dawn-core: `SlotLayout::capacity_for(SlotKind) -> u8`
- [x] dawn-ecs: `InventoryComp` 追加（`take`/`add`）
- [x] dawn-ecs: `FittingComp::slot(&self, SlotKind) -> &[FittedSlot]`（読み取り専用）
- [x] dawn-sector: `node/inventory.rs`（`fit_module_owned`/`unfit_module_owned`・初期シード関数）
- [x] dawn-sector: spawn（live・replay 両方）で初期インベントリをシード
- [x] dawn-sector: snapshot 保存/復元に `inventory` を追加
- [x] dawn-sector: `build_player_loadout_json` にインベントリを含める
- [x] dawn-actor: `ClientCommand::Fit`/`Unfit` 追加・protocol.rs 配線
- [x] dawn-simulation/dawn-sector-node: 両 dispatch site に配線
- [x] client: インベントリパネル UI・Fit/Unfit 送信
- [x] `cargo test --workspace` 全緑・GdUnit4 全緑

## 追記（2026-07-08）: §2 の制約トリガーが発火 — ドッキング必須化

§2「いつでもどこでも可能（MVP）」・却下した代替案「ドッキング/セーフゾーン
制約を今回入れる」は、当時 Station 等のエンティティ種別が存在せず前提が
無かったための時限的な単純化だった。ADR-0034/ADR-0037 で Station・
ドッキング状態が実装済みになり、この ADR 自身が名指ししていたトリガー
（「戦闘中の換装が問題になった場合は、別 ADR で制約を追加する」— 実際には
戦闘中に限らず「宇宙にいる間ずっと換装できる」こと自体が問題として報告された）
が発火したと判断し、新規 ADR を起票する代わりに本項を追記する。

**決定**: `fit_module_owned`/`unfit_module_owned`/新設
`reorder_fitted_module_owned`（ドラッグ&ドロップによる FITTED 内並べ替え、
別途ドラッグ&ドロップ機能一式と合わせて実装）は、呼び出し元の船が
**ドッキング中でなければ拒否**する（`is_ship_docked` チェック追加）。
スポーン時の特権パス `commands.rs::fit_module`（内部専用、既定ロードアウト
設定用）は対象外のまま——ADR-0032 冒頭の説明どおり、プレイヤー操作ではない。

**却下した代替**: 速度ゼロ必須・Tackle 中拒否等のより細かい制約は見送り。
「ドッキング中のみ」の方が Station という既存の実体に素直に対応し、
Assemble/Disassemble/Build と同じ「ドック中のみ」ルールに揃うため、
一貫性が高い。

実装: `crates/dawn-sector/src/node/inventory.rs`。テスト:
`fit_module_owned_is_rejected_when_the_ship_is_undocked`・
`unfit_module_owned_is_rejected_when_the_ship_is_undocked`・
`reorder_fitted_module_owned_is_rejected_when_the_ship_is_undocked`。
