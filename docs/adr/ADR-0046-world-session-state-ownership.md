---
id      : ADR-0046
title   : WorldSession pure state ownership in dawn-client-core
status  : accepted
date    : 2026-07-24
deciders: [human, ai-agent]
related : ADR-0039 (dawn-client-core client domain model), ADR-0040 (Godot adapter),
          ADR-0045 (single-owner client motion state), docs/architecture/architecture.md
---

# ADR-0046 - WorldSession pure state ownership in dawn-client-core

## Context

The former `client/scripts/world_session.gd` mixed live-world state with the
Godot scene-tree representation of ships. That made the state model depend on
`Node3D`, made state transitions difficult to exercise without a scene, and
allowed `main.gd` to retain mutable aliases to session state. The client-core
crate already owns the pure Loadout and motion models, so WorldSession state
belongs at the same boundary.

## Decision

`dawn-client-core::WorldSessionState` owns pure client state:

- ship metadata, health, player/opponent membership, and lock state;
- navigation records and current system state;
- tick, capacitor, and dock transition state;
- typed transition outcomes for registration, removal, destruction, health, and
  docking operations.

`dawn-client-gdext::WorldSession` is a thin adapter. It must not store or accept
`Node3D` references. `main.gd` remains responsible for Godot presentation and
scene lifecycle: its `_ships` dictionary maps ship IDs to scene nodes and
creates/frees those nodes.

The alternative of keeping the GDScript session and adding Rust helpers was
rejected because it would leave two state owners and preserve scene-tree
coupling. Moving the entire scene registry to Rust was rejected because Rust
must not own Godot scene nodes or presentation lifecycle.

## 実装の変遷（2026-07-28 追記）

決定そのもの——`WorldSessionState`が純粋状態を所有し、
`dawn-client-gdext::WorldSession`は薄いadapterで、`main.gd`がGodotの表示・
ライフサイクルを持つ——は今も有効である。変わったのは実現手段である。

- JSON/Dictionaryによる同型stateの往復を削除した。
- `WorldSession.snapshot()`を削除し、本番とテストが同じread accessorを使う。
- record-shaped outcomeは`ShipHealth`、`CapacitorStatus`、`DestructionOutcome`などの
  型付きGDExtension classへ移した。
- 実際に1フィールドしか読まれなかったregistration/removal/health outcomeは削除し、
  必要なscalarだけを返す形へ縮小した。

## Typed server-outcome application（2026-08-02、issue #238）

サーバー受信経路を型付きの単一経路へ移行した。`ServerMessageDecoder`はpostcardを
Rustのwire型へ復号し、状態更新はGDScript callbackより前に完了する。

この境界では次を禁止する。

- Rustのwire型をGodot `Dictionary`へ投影し、GDScriptが同じ値をRustへ戻すこと。
- presentation handlerが`WorldSession`のauthoritative stateを二重に更新すること。
- `PlayerLoadout`やMarket payloadをstring-keyed bagとして受け渡すこと。

GDScriptが受け取るのはscene/HUD更新に必要なprimitiveまたは型付きpresentation
recordだけである。

## Single test surface（2026-08-02、issue #255）

`WorldSession`のGodot公開面から、ship選択、health、lock、ship removal/destruction、
system、server tick、dock/undock、loadout dock contextを直接書き換えるpass-through
methodを削除した。これらは本番受信経路では使われず、GdUnitが本番では生成できない
状態や順序を作るためだけに残っていた。

Godotから公開する操作はread accessor、接続切断時の`reset()`、および明示的に
client-ownedな予測時計を進める`advance_client_ticks()`に限る。server-driven stateは
productionとtestのどちらも`ServerMessageOutcome::dispatch`から適用する。
順序・拒否・遷移結果の細部はpure Rust testで直接検証し、GdUnitはtyped inbound
wiringとGodot公開read/reset/client-clock surfaceを検証する。

## Server-fact policy ownership（2026-08-02、issue #251）

#238後もGDExtension adapterには、despawnとAoI leaveのlock clearing差、station name解決、
stale dock eventの適用、loadout replacementとsession reconciliationの順序、module activation、
tick/capacitor advancementといったwire非依存policyが残っていた。

これらを`dawn-client-core::ClientState`と`ClientFact`へ移した。adapterの責務は次に限定する。

1. postcard frameをdecodeし、Godot整数範囲とcanonical item identityを検証する。
2. wire値をwire非依存の`ClientFact`とpresentation値へ変換する。
3. `ClientState::apply`でsession/loadout transactionを完了する。
4. `WorldSessionEffect`から必要な値を取り出して最終callback引数へ変換し、最終callbackを一度だけ呼ぶ。

`ClientState`は`WorldSessionState`と`Option<PlayerLoadoutMsg>`を同時にborrowするため、
loadout replacement、dock reconciliation、tick simulation、module activationをadapterが
別々に並べ替えられない。despawn/AoI差は`ShipLeaveReason`として表現し、station表示名は
core-owned navigation stateから解決する。

presentation seamは`ServerMessageOutcome::dispatch`の1箇所だけである。decode結果は検証済みの
`dawn_protocol::ServerMessage`をそのまま保持し、同型の`ClientOutcome` mirrorは削除した。
world eventはGDScriptが明示したscene ownerの最終`_handle_*` callbackへ直接dispatchする。
Rust adapterは`get_parent()`などでscene tree構造を推測しない。以前の
`ServerEventOutcome`生成→signal→再dispatchという二段経路と互換classは削除した。

## Implementation checklist

- [x] Add `WorldSessionState` and typed input/record/outcome types to
      `crates/dawn-client-core`.
- [x] Add the `WorldSession` GDExtension adapter in
      `crates/dawn-client-gdext`.
- [x] Keep the Godot scene-node registry in `client/scripts/main.gd`.
- [x] Add pure Rust and GdUnit4 coverage for navigation, ship lifecycle, HP,
      locks, ticks/capacitor, and docking state.
- [x] Add the `ClientState` / `ClientFact` server-fact boundary.
- [x] Remove the redundant `ClientOutcome` mirror and runtime two-stage event dispatch.
- [x] Preserve issue #255's single public test surface.
- [x] Update crate-boundary and architecture documentation.
