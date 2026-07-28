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

`dawn-client-gdext::WorldSession` is a thin adapter. It parses JSON only at the
Godot boundary, delegates state transitions to `WorldSessionState`, and returns
Godot `Dictionary` snapshots/outcomes. It must not store or accept `Node3D`
references.

`main.gd` remains responsible for Godot presentation and lifecycle: its
`_ships` dictionary maps ship IDs to scene nodes, creates/frees those nodes,
and applies returned state to visual components. It synchronizes scalar and
collection state through `WorldSession.snapshot()` instead of retaining aliases
to Rust-owned collections.

The alternative of keeping the GDScript session and adding Rust helpers was
rejected because it would leave two state owners and preserve scene-tree
coupling. Moving the entire scene registry to Rust was rejected because Rust
must not own Godot scene nodes or presentation lifecycle.

## 実装の変遷（2026-07-28 追記）

上の「決定」節はこのADRを書いた時点の実装を具体的に描写しており、その後3点が
事実と食い違ったまま残っていた。**決定そのもの**——`WorldSessionState`が純粋状態を
所有し、`dawn-client-gdext::WorldSession`は薄いadapterで、`main.gd`がGodotの
表示・ライフサイクルを持つ——は今も有効である。変わったのは実現手段だけなので、
supersedeせずここに現状を記録する。

| 決定節の記述 | 現状 | 変更した経緯 |
|---|---|---|
| 「JSONをGodot境界でのみパースする」 | JSONパースは無い | `register_ship`が受け取っていた`JSON.stringify`済み文字列を`Dictionary`直接受け取りへ（issue #178） |
| 「`WorldSession.snapshot()`を通じてscalar/collection状態を同期する」 | `snapshot()`は削除 | まず`main.gd`が利用時点で個別accessorを読む形へ移行し、`snapshot()`はテスト専用として残っていた（本ADR candidate 5）。テストと本番で読み取り経路が2本あるとドリフトしても本番側が気づけないため、今回削除しテストも同じaccessorを使う |
| 「Godot `Dictionary`のsnapshot/outcomeを返す」 | 型付きクラスを返す | `Dictionary`は呼び出し側にキー文字列と型キャストを要求し、レコードの形が`main.gd`側の記憶に置かれていた。`GateRecord`/`StationRecord`/`CelestialBodyRecord`/`BuildableShipType`/`ShipHealth`/`CapacitorStatus`/`DestructionOutcome`（`session_record_gd.rs`）へ移行。既存の`ItemRow`/`ModuleRow`と同じ形 |

あわせて、outcomeに対してdeletion testを適用した。`RegistrationOutcome`（3
フィールド）・`RemovalOutcome`（4）・`HealthEventOutcome`（3）は、GDScript側にも
Rustテスト側にも**1フィールドしか読み手がいなかった**。残りは内部状態遷移を駆動する
ローカル変数の値を外へエコーしていただけで、報告先が無かった。「内部で計算する」ことと
「外へ報告する」ことは別で、荷重がかかっていたのは前者だけなので、これらの構造体は
削除し、各メソッドは実際に消費されていた1つの値を返す（`register_ship -> bool`、
`remove_ship -> bool`、`apply_hp_event -> ()`）。`DestructionOutcome`だけは
3フィールドとも読み手がある——`destroyed`でシーンノードを解放し、
`destroyed_player`/`destroyed_opponent`でHUDのDEFEAT/VICTORY表示を切り替える——ため
そのまま残る。

削除対象を選ぶ際は、フィールド名でgrepして本番・テスト両方の読み手を数えること。
この作業中に`destroyed_player`を読み手なしと誤判定して一度削除し、Godot側の
パースエラーで気づいた（`main.gd`の撃墜時DEFEAT表示が唯一の読み手だった）。
呼び出し直後のブロックだけを見ると、同じ関数の後半にある読み手を見落とす。

`dock_status()`は逆方向の是正で、4キーの`Dictionary`を返していたが9箇所の
呼び出しのうち8箇所は1値しか読んでいなかった。`docked_station_id()` /
`docked_station_name()` / `latest_dock_state_tick()`（既存の`is_docked()`と揃う）
へ分解した。

## Implementation checklist

- [x] Add `WorldSessionState` and typed input/record/outcome types to
      `crates/dawn-client-core`.
- [x] Add the `WorldSession` GDExtension adapter in
      `crates/dawn-client-gdext`.
- [x] Keep the Godot scene-node registry in `client/scripts/main.gd` and remove
      `client/scripts/world_session.gd`.
- [x] Add pure Rust and GdUnit4 coverage for navigation, ship lifecycle, HP,
      locks, ticks/capacitor, and docking state.
- [x] Update crate-boundary and architecture documentation.
