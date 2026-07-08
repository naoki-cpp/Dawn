---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md
date     : 2026-07-08（定期再計測。前回記録から drift していた行数を実測で更新し、
分割済み `dawn-actor/src/protocol.rs` を `protocol/mod.rs` / `client_command.rs` /
`server_event.rs` / `hello_resume.rs` に置き換えて R-5 を解消済みに移動。server 総合 B+ 維持、
client 側は別途 architecture-review-client.md 参照）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B+**（2026-07-08 再計測で維持。前回レビュー以降に実装が進んだぶん記録値は広く古くなっていたが、
今回の実測では `dawn-actor/src/protocol.rs` 分割によって R-5 は解消済みへ移動し、
新たな red file は発生していない。一方で `warp.rs` / `station.rs` / `commands.rs` /
`orbit.rs` / `transit_flow.rs` といった watch 帯の大型ファイル群は残っているため、
ボトルネック軸は引き続きファイルサイズで B+ のまま）

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector / dawn-replication が分離済み（ADR-0026/0027）。M-7 解消で `ClientCommand` を `dawn-core` へ移動し DAG が整理された（`dawn-sector` が `dawn-actor` 非依存のまま dispatch を保持できるようになった）。Player Command Dispatch のための新 crate は引き続き不要 |
| ファイルサイズ | B+ | 2026-07-08 再計測。`warp.rs` 1024・`station.rs` 1288・`commands.rs` 1022・`transit_flow.rs` 863・`orbit.rs` 790 は依然 watch 帯で、R-3 の defer 判断は妥当なまま。`node/mod.rs` は 746、`node/coordinates.rs` は 174 と R-4 後の役割分担が保たれている。前回の R-5 は `dawn-actor/src/protocol.rs` 分割で解消済みだが、残る大型ファイル群があるため軸の評価自体は B+ を維持 |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。`InventoryComp`（ADR-0032）・`RepairLayer`/`RepairApplied`（ADR-0033）・`ItemId`（ADR-0034、`dawn-core/src/item.rs`）も既存型設計に整合 |
| 重複 | A− | WS 境界は dawn-actor へ集約（M-4 解消）。AoI delivery、production runtime、Command dispatch は deep module 化済み（M-7 解消で `apply_client_command` が `SimulationNode` に集約）。2026-07-08、`ItemId -> ItemRow` JSON変換の重複（`serialization.rs` 2箇所）を `item_id_to_row_json` へ集約し解消済み。残る両バイナリ間グルー重複（M-6）・Fit経路のテール重複（M-8）は許容判断のまま |
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。`TransitOp::Commit` は ADR-0032 で `Box<ShipSnapshot>` 化しサイズ非対称を解消済み |
| AI開発由来 | A− | 命名汚染なし。残る `SectorSimulatorActor` の密結合（M-3）は本番パス外の in-process 専用で実害小 |

---

## ファイルサイズ一覧（2026-07-08 時点）

> **2026-07-08、全ファイル再計測（`/architecture-review`）。** 前回パス（2026-07-06/07）
> 以降に landed した9B-5/ADR-0037系の機能（Assemble/Disembark/複数船ロスターUI/
> TransferToStationCommand）で、`inventory.rs`（428→570）・`spawner_logic.rs`（623→669）・
> `apply_event.rs`（498→566）・`commands.rs`（dawn-core、473→551）・`events.rs`（657→694）・
> 今回の実測では、前回レビュー以降の deepening と整理を反映して
> `warp.rs` 1024・`spawner_logic.rs` 611・`orbit.rs` 790・`mod.rs` 746・`coordinates.rs` 174・
> `transit_flow.rs` 863・`station.rs` 1288・`snapshot_io.rs` 591・`inventory.rs` 522・
> `dawn-core/src/commands.rs` 508・`serialization.rs` 982・`apply_event.rs` 781 に更新した。
> `dawn-actor` 側では単一の `protocol.rs` は消え、`protocol/mod.rs` 710 /
> `client_command.rs` 357 / `server_event.rs` 252 / `hello_resume.rs` 29 へ分割済み。
> これにより前回起票した R-5 は解消済みに移動する。

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/warp.rs` | 1024 | 🟡 R-1 新設（2026-06-23）。warp 幾何の単一責務だが総行数が閾値を超過。前回レビュー時の 1093 からは縮小したが、依然 watch 対象 |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 611 | 🟢 P4-2 + P7-1 + ADR-0029 + ADR-0032。残るのは spawn mechanics（spawn / inventory seed）のみで、R-3 の観察対象から外れたまま |
| `crates/dawn-sector/src/node/bot_ai.rs` | 347 | 🟢 `spawner_logic.rs` から `process_bots` を抽出した Bot AI 決定ループ。純粋移動、挙動変更なし |
| `crates/dawn-sector/src/node/orbit.rs` | 790 | 🟡 ADR-0031 新設。Orbit / Keep at Range の操船一式。単一責務で許容だが総行数は watch 帯 |
| `crates/dawn-sector/src/node/mod.rs` | 746 | 🟢 R-4 完了（2026-07-07）。`coordinates.rs` 抽出後の役割分担が維持され、構造体宣言・定数・コンストラクタ・population backstop・identity/observation アクセサへ責務が戻っている |
| `crates/dawn-sector/src/node/coordinates.rs` | 174 | 🟢 R-4（2026-07-07新設）。`AnchorTable`（ADR-0029）呼び出し側の座標合成アクセサを一元化した deep module |
| `crates/dawn-sector/src/node/transit_flow.rs` | 863 | 🟢 `prepare_transit_commit`/`handle_transit_commit`（公開面 5→2 に集約）+ `rebase_after_transit`。大きいが責務は cohesive |
| `crates/dawn-sector/src/node/station.rs` | 1288 | 🟡 ADR-0034/9B foundation。dock/undock・station inventory・build/disassemble/assemble/disembark が1ファイルに集まっており、単一の「Station operations」としては読めるが総行数は watch 帯 |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 591 | 🟢 P7-pre + ADR-0032（inventory 永続化）。ほぼテスト |
| `crates/dawn-sector/src/node/inventory.rs` | 522 | 🟢 ADR-0032 新設。fit/unfit_module_owned + seed + テスト。station inventory transfer 追加後も責務は単一 |
| `crates/dawn-sector/src/node/commands.rs` | 1022（impl 644） | 🟡 P7-1 + ADR-0032 + M-7（Issue #56）+ ADR-0035 + 9B station commands。command dispatch と操作検証の責務は保っているが、総行数は watch 帯。2026-07-07、ADR-0037 で `owns_ship` に加え `is_active_ship` を新設し `apply_client_command` の各 arm を active_ship 解決へ書き換え（962→1017）。同日、Phase 9B-5 Assemble 実装に伴い `ClientCommandFollowup::RefreshFitting` を `ShipId` から `PlayerId` へ変更（Disassemble後にRefreshFittingがship_id経由でplayer_idを解決できず更新が届かないバグを修正）、Dock/Undock/Build/Disassemble/SelectActiveShip/Assembleの各armを簡略化（1017→1014）。2026-07-08、`TransferToStationCommand` の dispatch arm を1件追加（1014→1022）。impl 644 でトリガー未発火だが引き続き watch 対象 |
| `crates/dawn-sector/src/node/serialization.rs` | 982 | 🟢 InitialState / PlayerLoadout / handoff payload の組み立て。依然大きいが責務は単一で、`ItemId -> ItemRow JSON` 重複も `item_id_to_row_json` へ集約済み |
| `crates/dawn-sector/src/galaxy.rs` | 459 | 🟢 ADR-0029 AU→units 変換・ゲート AU 化 |
| `crates/dawn-sector/src/node/apply_event.rs` | 781 | 🟢 P7-pre + ADR-0032 + ADR-0035。replay apply の責務は単一。サイズは伸びたが、履歴再生の owner として一貫している |
| `crates/dawn-sector/src/node/tackle.rs` | 345 | 🟢 P7-pre。ADR-0035（PR #62）で距離判定を `entity_absolute_f64` の f64 差分に修正（真 AU スケールでの f32 丸め対策・ADR-0029 パターン準拠）。PR #66 で手組みの delta 計算を `SimulationNode::ship_distance` 呼び出しに置換し未使用 `PositionComp` import を削除（358→345） |
| `crates/dawn-sector/src/node/range_gate.rs` | 479（impl 150） | 🟢 ADR-0035 新設（PR #62）。Range Gate System（Step 5.5）— Weapon/Tackle/Remote Repair のターゲットが射程外に出たら強制 OFF（`ModuleDeactivated { forced_reason: OutOfRange }`）。PR #63 で flat-index 解決を `FittingComp::slot_at_flat_mut` に置換（403→382）。PR #66 で距離判定を `SimulationNode::ship_distance` 呼び出しに置換（382→362）。ADR-0036 で `effective_range_for_kind`/`process_range_gate` に Remote Repair 2 kind を追加 + 活性化/Range Gate/回復のテスト3件を追加（362→469） |
| `crates/dawn-sector/src/aoi.rs` | 629（impl 311） | 🟢 `AoiDelivery`/`AoiSink`/`Observer`（旧 dawn-simulation・dawn-sector-node 重複の集約先）。半分弱はテスト。2026-07-01、`deliver_frame` を `<S: EventStore>` でジェネリック化 |
| `crates/dawn-sector/src/anchor.rs` | 311 | 🟢 ADR-0029 新設（AnchorTable・静的 f64 アンカー絶対座標）。2026-07-07、`/improve-codebase-architecture` で `node/mod.rs` が再実装していた逆変換を `to_relative()` として新設し、`rebase()` をその合成に書き直し。座標合成代数の唯一の所有者になった（292→311） |
| `crates/dawn-sector/src/transit.rs` | 419（impl 242） | 🟢 PR #30 で `run_runtime_tick` / `RuntimeTickOutput` を追加。Request/Commit ハンドラが `prepare_transit_commit`/`handle_transit_commit` に委譲し Gate-lookup 知識を手放した。2026-07-02、`propose_jump` / `propose_auto_jump` を新設し、jump fallback outcome → `TransitOp::Request` 提案の組み立てを `dawn-sector-node`・`dawn-simulation` 双方の重複から集約（282→418、テスト2件追加でimpl 240） |
| `crates/dawn-sector/src/modules.rs` | 246 | 🟢 ADR-0033 で Active 修理モジュール定義を追加。ADR-0036 で Remote Shield Booster / Remote Armor Repairer を追加（211→246） |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 201 | 🟢 ADR-0032 で `ShipSnapshot.inventory` 追加 |
| `crates/dawn-sector/src/dilation.rs` | 164 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 174 | 🟢 |
| `crates/dawn-sector/src/node/approach.rs` | 565（impl 182） | 🟢 R-1 新設（2026-06-23）。approach 系 + ADR-0031 で clear_steering_modes 連携。2026-07-01、独自の検証チェックリストを `orbit.rs` の `begin_maneuver` 呼び出しに置き換え、Orbit/KeepAtRange と完全に同じ経路を通るように統一。同日、`apply_approach_jump_fallback`（1行ラッパー）を `jump.rs` へ移設・削除し、`apply_approach_command_with_auto_jump` を `pub(super)` 化。PR #68（候補2）で `dest_in_ship_frame_abs` を `node/mod.rs` へ移設（4サブモジュールから呼ばれる共有アクセサのため、577→562） |
| `crates/dawn-sector/src/node/jump.rs` | 250（impl 89） | 🟢 新設（2026-07-01）。PR #54 で `apply_jump_with_fallback`（3択フォールバック）、PR #55 で `resolve_auto_jump`（auto-jump 判定）を追加。両 PR でテストも追加（186→250）。impl 88 行で健全 |
| `crates/dawn-sector/src/node/tick.rs` | 267 | 🟢 P4-1 + ADR-0031 Step 2.55/2.56 + ADR-0033 Step 6.5 配線。PR #67 で cap-refit ループを `reapply_fitting` に、destroyed-ship 削除ループを `remove_ship` に置換（177→171） |
| `crates/dawn-sector/src/spawner.rs` | 133 | 🟢 |
| `crates/dawn-sector/src/ship_types.rs` | 91 | 🟢 |
| `crates/dawn-sector/src/node/navigation.rs` | 161 | 🟢 R-1 後。`can_propose_jump` / `can_propose_warp` + ADR-0017 dead-zone テスト |
| `crates/dawn-sector/src/node/ship_registry.rs` | 76 | 🟢 P3-1。PR #67（アーキテクチャレビュー候補1）で `remove(ship_id, world)` を新設し、index/type_ids/owners/by_player の削除と ECS despawn を1メソッドに集約（33→56）。従来は各削除元（tick.rs/apply_event.rs/transit_flow.rs）が個別に4-6行を手組みしており、`transit_flow.rs` の1箇所は owners/by_player の削除を欠落させていた。2026-07-07、ADR-0037 で `by_player` を `active_ship` に改名し、`remove()` は削除される船が実際に active だった場合のみ `active_ship` を消すよう修正（複数所有時に別の所有船削除でactiveポインタが誤って消える潜在バグの修正、56→76） |
| `crates/dawn-sector/src/node/sector_map.rs` | 28 | 🟢 P3-1 |

### dawn-actor（クライアント転送境界・M-4 集約先）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-actor/src/protocol/mod.rs` | 710 | 🟢 R-5 完了（2026-07-08）。wire protocol の公開面と統合テスト・schema freshness test を束ねる薄い入口に縮小 |
| `crates/dawn-actor/src/protocol/client_command.rs` | 357 | 🟢 client -> server wire translation の deep module。`ClientCommandJson` / `parse_client_command` / schema 出力を集約 |
| `crates/dawn-actor/src/protocol/server_event.rs` | 252 | 🟢 server -> client wire translation の deep module。`EventJson` / `domain_event_to_json` / redirect payload を集約 |
| `crates/dawn-actor/src/protocol/hello_resume.rs` | 29 | 🟢 Hello / resume handshake の小さな補助モジュール |
| `crates/dawn-actor/src/client_connection.rs` | 262 | 🟢 ClientConnection trait + InProcess/Ws 実装 |
| `crates/dawn-actor/src/ws_server.rs` | 275 | 🟢 M-4 集約（WsServer / PlayerSession）+ ADR-0032 `send_raw` |
| `crates/dawn-actor/src/lib.rs` | 41 | 🟢 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 630 | 🟢 Raft クラスター配線（in-process テスト用） |
| `crates/dawn-simulation/src/serve/mod.rs` | 334 | 🟢 P5-1 共通ヘルパー。`node.apply_client_command` 呼び出しに統一済み |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 470 | 🟡 M-3（本番パス外・保留） |
| `crates/dawn-simulation/src/bench.rs` | 493 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 239 | 🟢 `AoiDelivery` を持ち、入力処理と runtime 呼び出し中心 |
| `crates/dawn-simulation/src/serve/runtime.rs` | 183 | 🟢 auto-jump / ownership handoff / scoped InitialState resend を集約 |
| `crates/dawn-simulation/src/serve/aoi_delivery.rs` | 119 | 🟢 配信ロジック本体を `dawn_sector::aoi::AoiDelivery` へ移動。残りは adapter のみ |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 224 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 235 | 🟢 P5-1。AoI delivery 詳細を `AoiDelivery` に委譲 |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 189 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 77 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 9 | 🟢 P5-2 |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 593 | 🟡 許容範囲（Raft 実装の核） |
| `crates/dawn-sector-node/src/runtime.rs` | 256 | 🟢 production Node の jump fallback / tick stepping / replication publish 呼び出し / Redirect / AoI delivery を集約。本ファイルは orchestration のみ |
| `crates/dawn-sector-node/src/client_admission.rs` | 236 | 🟢 client admission state machine |
| `crates/dawn-sector-node/src/main.rs` | 322 | 🟢 8D-4 本番バイナリ。config / TCP transport / accept channel / data loading の配線に縮小 |
| `crates/dawn-core/src/events.rs` | 694 | 🟢 ADR-0032 `ShipFitted.inventory`・ADR-0033 `RepairApplied`/`RepairLayer`・ADR-0035 `ModuleActivated.target_ship_id`/`ModuleDeactivated.forced_reason`/`ModuleDeactivationReason` 追加。2026-07-07、ADR-0034/0037 `ShipAssembled` イベントを追加（657→694） |
| `crates/dawn-core/src/item.rs` | 36 | 🟢 **新規記録**（本表に未記載だった）。ADR-0034 `ItemId`（`Module`/`PackagedShip`/`ScrapMetal`）— 経済系機能全体が参照する小さく安定した型定義 |
| `crates/dawn-ecs/src/systems/combat.rs` | 578 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 485 | 🟢 ADR-0033 `repair_cycles_started` 収集を並置。ADR-0035 で強制 OFF に `forced_reason: CapacitorExhausted` を付与。PR #63 で flat-index 境界計算と 4-way chain を `FittingComp::slot_at_flat_mut`/`iter_slots` に置換（504→479）。PR #65 でスロット変更3行を `FittedSlot::force_off()` 呼び出しに置換（479→476）。ADR-0036 で `SlotInfo.target_ship_id` を追加し `RepairCycle` へ `slot.target_ship_id.unwrap_or(snap.ship_id)` を渡すよう変更（476→490） |
| `crates/dawn-consensus/src/actor.rs` | 476 | 🟢 8D-5 実機検証で使う Raft role-transition ログ（`eprintln!`）を保持 |
| `crates/dawn-event-store/src/file.rs` | 464 | 🟢 |
| `crates/dawn-core/src/fitting.rs` | 430 | 🟢 **新規記録**（本表に未記載だった）。`ModuleDefinition`/`ModuleKind`/`SlotKind`/`StatDelta`/`FittingSnapshot` 等、Fitting ドメイン型の定義一式。ADR-0036 で `ModuleKind::RemoteShieldBooster`/`RemoteArmorRepairer` + `StatDelta.repair_range_add` を追加（320→341） |
| `crates/dawn-consensus/src/transport.rs` | 204 | 🟢 **新規記録**（本表に未記載だった）。`RaftTransport` trait 定義 + in-process 実装 |
| `crates/dawn-event-store/src/memory.rs` | 184 | 🟢 **新規記録**（本表に未記載だった）。`InMemoryEventStore` |
| `crates/dawn-ecs/src/systems/movement.rs` | 415 | 🟢 |
| `crates/dawn-ecs/src/systems/lock.rs` | 375 | 🟢 |
| `crates/dawn-core/src/commands.rs` | 551 | 🟢 Command enum 群（継続的に variant 追加）。M-7（Issue #56）で `ClientCommand` enum を `dawn-actor` から移動（359→395）。ADR-0035 で `ActivateModuleCommand.target_ship_id` 追加（395→401）。2026-07-07、ADR-0037 で操縦系/Undock コマンド構造体から `ship_id` を除去し `SelectActiveShipCommand` を新設（457→473）。2026-07-07〜08、`AssembleCommand`/`DisembarkCommand`/`TransferToStationCommand` を新設（473→551） |
| `crates/dawn-consensus/src/rpc.rs` | 371 | 🟢 343→371。Raft RPC 型定義 |
| `crates/dawn-consensus/src/tcp_transport.rs` | 353 | 🟢 337→351。8D-3 TcpRaftTransport |
| `crates/dawn-ecs/src/systems/fitting.rs` | 315 | 🟢 PR #63 で `apply_fitting()` の4-way chain を `FittingComp::iter_slots()` に置換 |
| `crates/dawn-replication/src/tcp.rs` | 288 | 🟢 283→287。8D-2c |
| `crates/dawn-ecs/src/components/movement.rs` | 291 | 🟢 ADR-0036 で `ShipStatsComp.repair_range` を追加（284→291） |
| `crates/dawn-ecs/src/world.rs` | 294 | 🟢 270→285。クエリヘルパー |
| `crates/dawn-sector-node/src/data_loader.rs` | 283 | 🟢 178→278（+100）。8D-4/8D-5 のテスト追加が主因。module/ship type TOML ローダー。ADR-0036 で `repair_range_add`/Remote Repair 2 kind の TOML パースを追加（278→283） |
| `crates/dawn-ecs/src/components/fitting.rs` | 384 | 🟢 ADR-0035 `FittedSlot.target_ship_id` 追加。PR #63 で `slot_at_flat`/`slot_at_flat_mut`/`iter_slots`/`iter_slots_mut` を新設し、3系統に分散していた flat-index 境界計算と6+箇所の4-way chain の唯一の所有者になった。PR #65 で `FittedSlot::force_off()` を新設し、capacitor/Range Gate/player-deactivate の3箇所が個別に手組みしていたスロット強制OFF処理（is_active/cycle_remaining/target_ship_idの3フィールド）の唯一の所有者になった（370→384） |
| `crates/dawn-ecs/src/components/combat.rs` | 376 | 🟢 |
| `crates/dawn-replication/src/anti_entropy.rs` | 216 | 🟢 211→215。8D-2b |
| `crates/dawn-ecs/src/systems/repair.rs` | 257 | 🟢 ADR-0033 新設（Step 6.5 Repair System・RepairApplied 発行 + テスト）。ADR-0036 で `RepairCycle.target_ship_id` を追加し `ship_id`→`target_ship_id` 検索に変更（自己/遠隔を区別しない共通コードパス）+ 遠隔修理テスト1件追加（213→255） |
| `crates/dawn-replication/src/replica.rs` | 225 | 🟢 M-5（ReplicaSet・複製ログ消費側） |
| `crates/dawn-replication/src/bus.rs` | 237 | 🟢 188→236（+48）。8D-2a。テスト追加が主因 |
| `crates/dawn-core/src/navigation.rs` | 253 | 🟢 184→196。ナビゲーション型定義 |
| `crates/dawn-replication/src/snapshot.rs` | 175 | 🟢 8D-2d SnapshotTransfer（ジェネリック / 256 MiB cap） |
| `crates/dawn-core/src/ship_type.rs` | 177 | 🟢 |
| `crates/dawn-replication/src/outbound.rs` | 142 | 🟢 sender-side `OutboundLogPublisher`。append-log cursor と `LogBatch` suffix 構築を保持 |
| `crates/dawn-replication/src/lib.rs` | 110 | 🟢 8D-2a/2b/2c/2d public API |
| `crates/dawn-ecs/src/components/inventory.rs` | 127 | 🟢 ADR-0032 新設（InventoryComp）。2026-07-08、ADR-0034 9B `take_all`（whole-stack removal、`TransferToStationCommand`向け）+ テスト2件を追加（104→127） |
| `crates/dawn-sector-node/src/config.rs` | 90 | 🟢 8D-4 TOML 静的 config。2026-07-01、永続化パス（`event_log_path`/`snapshot_path`/`cold_path`/`checkpoint_interval_ticks`）を追加（全て `#[serde(default)]` 付きで後方互換） |

---

## 問題一覧

### Medium

#### M-3（優先度低・本番パス外）: `sector_simulator_actor.rs` と `SimulationNode` の密結合

`SectorSimulatorActor` は `SimulationNode` の公開メソッドをほぼ全て呼ぶ薄いラッパーで、
`SimulationNode` の変更が即 Actor に波及する。

**ただし本番パス外。** `SectorSimulatorActor` を使うのは `MultiNodeCluster`
（dawn-simulation のインプロセス・テスト/ベンチ用クラスタ）のみ。本番バイナリ
`dawn-sector-node` は 8D-4 で独自の main ループを持ち、この Actor を使わない。

このため当初の「8D-5 実機検証で境界の揺れが確定してから着手」という前提は無効化した
（8D-5 が動かすのは dawn-sector-node であり、この Actor を一切経由しない）。
加えて各ハンドラ（Tick / SpawnShip / Transit / Jump …）は「メッセージ → node メソッド → 返信」の
薄いアダプタで、sync な node を async メッセージングへ繋ぐ Actor の性質上ある程度は本質的。
コマンド/応答 enum 化しても本番価値は薄く、インプロセス・クラスタテストを壊すリスクが上回る。

優先度を下げて保留する。再評価のトリガー: `SectorSimulatorActor` の main ループと
`dawn-sector-node` の main ループの重複（両者とも tick + Raft + replication を駆動）が
保守上の実害になったとき、または in-process クラスタを本番に近づける必要が出たとき。

> M-4（WS 境界の `dawn-actor` 集約・2026-06-20）、M-5（replication 消費側 `ReplicaSet`・
> 2026-06-20）、dawn-simulation 側の AoI delivery deepening（PR #34）、
> および Sector Node runtime deepening（2026-06-29）は解消済み。
> 詳細は「改善ロードマップ > 完了済み」を参照。

#### M-6（縮小・許容）: 2つの serve バイナリに残る adapter 重複

M-4（WS 境界）、PR #34（dawn-simulation 側 AoI delivery deepening）、
Sector Node runtime deepening、AoI delivery の dawn-sector への集約後も、
両バイナリの「アプリケーション層」adapter/glue は一部重複している:

| 重複 | dawn-simulation | dawn-sector-node | 備考 |
|---|---|---|---|
| ~~Player Command Dispatch~~ | ~~`serve/mod.rs::apply_common_command`~~ | ~~`runtime.rs::collect_player_commands`~~ | **解消済み（M-7・Issue #56）**: `node.apply_client_command` に統一 |
| `data_loader`（`load_modules` / `load_ship_types` / `parse_*`） | `data_loader/*.rs`（実装 ~280行）| `data_loader.rs`（278行）| TOML ローダー |
| `spawn_npcs` / `spawn_npc_frigates` | `serve/mod.rs:278` | `main.rs:298` | **実質同一**（~12行）|

> AoI フレーム配信の重複は解消済み（2026-06-29）。
> Player Command Dispatch の重複は M-7（Issue #56）で解消済み（2026-07-01）。

現在の実態では、`dawn-simulation` 側は `serve/runtime.rs` と `serve/aoi_delivery.rs` によって
single/cluster の内部知識をかなり集約済みで、`dawn-sector-node` 側も `runtime.rs` によって
production process model 固有の frame orchestration を集約済みである。問題は「同じ大きな serve loop が
二重化している」ではなく、**2つの process model がそれぞれ adapter を持つ**ことに縮小した。
8D-4 で `dawn-sector-node` を `dawn-simulation` の serve 経路からコピーして作った名残はあるが、
WS protocol は `dawn-actor` に、ゲームロジックは `dawn-sector` に、両 runtime の frame policy は
それぞれのローカル module に寄っており、残る重複は低頻度の glue に縮小している。

~~Player Command Dispatch は `ClientCommand` と `SimulationNode` の両方を知るため、`dawn-actor` / `dawn-sector` のどちらにも置きにくかった。~~ M-7（Issue #56）で解消: `ClientCommand` を `dawn-core` へ移動し DAG ブロッカーを外したうえで `SimulationNode::apply_client_command` に集約した。
`data_loader` / NPC spawn は I/O と demo wiring の低頻度 glue で、共有 crate へ
押し込むほどの深さがない。

#### 判断: 当面は許容する（新規 crate は作らない）

`dawn-server`（仮称）のような大きい共有 runtime crate を新設する案は、文書全体に照らして
**過剰**と判断し採らない。理由:

- **Player Command Dispatch は crate seam としては浅い。** Command 追加時に drift しやすい
  match と fitting refresh / jump follow-up 判定はあるが、現時点では2 runtime 間の100行前後の重複で、
  ADR を伴う新 crate にするほどの depth ではない。
- **8D 最小化方針**（roadmap「巨大基盤の一括建設をしない・薄いスライス」）に逆行する。
- **前例との整合**: `dawn-proto` は「見返りが乏しい」と却下、P4-3 は `_owned` 統合を
  「統合コストが効果を上回る」とスキップ。現在残る安定したグルーの重複も同じ費用対効果で許容が妥当。
- **残るドリフトの実害が限定的**: M-4 で直した `protocol`（18 variant・wire 境界・変更頻度高）と違い、
  Player Command Dispatch / `data_loader` / NPC spawn は process model に近い adapter で、差分が見えやすい。

再評価トリガー（このいずれかが起きたら設計し直す）:
- `data_loader` / NPC spawn が実際にドリフトしてバグを生んだとき
- 3つ目の serve バイナリが必要になったとき
- 2バイナリの process モデル差を解消し1バイナリ化できる見込みが立ったとき
  （その場合は新規クレートではなくバイナリ統合を優先検討する）

> 2026-07-01、Player Command Dispatch（M-7・Issue #56）を解消。`dawn-sector-node/runtime.rs` の
> 13分岐 match と `dawn-simulation/serve/mod.rs::apply_common_command` を
> `SimulationNode::apply_client_command` に統一した。M-6 の残る重複（data_loader / NPC spawn）は
> 引き続き許容判断・トリガー付きで保留。

#### ~~M-7~~（**解消済み** 2026-07-01）: Player Command Dispatch のルーティングが `dawn-sector` の外に漏れていた

`runtime.rs`/`protocol.rs` に分かれていたコマンドルーティングを `SimulationNode::apply_client_command`
に統一して解消。詳細は「改善ロードマップ > 完了済み」の M-7 行・Issue #56 を参照。

#### M-8（新規・2026-07-01・許容）: `fit_module` / `fit_module_owned` の共有テール重複

`commands.rs::fit_module`（spawn 時の無検証・特権パス）と
`inventory.rs::fit_module_owned`（プレイヤー操作・所有権/在庫/スロット検証あり）は、
`apply_fitting` 呼び出しから `ShipFitted` イベント発行までのテールがほぼ同型で重複する。

**根本原因**: 2つの Fit 経路（特権 spawn 時 / プレイヤー操作）が要求する検証が
非対称なため、テールだけ共有する形に自然となった。

**判断: 許容（現状追認）。** `inventory.rs` 冒頭のモジュールコメント自体が
「`fit_module` は既存の挙動・テストを守るため意図的に手を加えない特権パスとして残す」と
明記しており、これは未管理の負債ではなくドキュメント化済みの設計判断。
テール（`apply_fitting` → snapshot → `ShipFitted` emit）だけを private helper に
くくり出す余地はあるが、効果が小さく優先度なし。

再評価トリガー: 3つ目の Fit 経路（例: NPC ループ内リフィット等）が必要になり、
テール重複が3箇所に増えたとき。

#### ~~Steering-mode 排他制御の非対称性~~（**解消済み** 2026-07-01）

`/improve-codebase-architecture` の「5ハンドラの重複」指摘を調査する過程で、スタイル上の重複ではなく
実害のある非対称性3件（Warpが`clear_steering_modes`を呼ばない、Approachに`is_warping`ガードがない、
Aligningフェーズ中はWarp優先チェックが素通りする）を発見・修正。詳細は「改善ロードマップ > 完了済み」の
該当行を参照。

#### M-9（新規・2026-07-01・保留）: `EventStore::append` がinfallibleと偽る

`/improve-codebase-architecture` の指摘: トレイト `EventStore::append` は `u64` を
返すのみで失敗を表現できないが、`FileEventStore::append`（file.rs:232-240）は
書き込み/flush失敗時に `.expect()` で panic する。tickのホットパス上にあるため、
ディスクフル等が起きるとSectorプロセス全体が落ちる。

調査の結果、この経路は**2026-07-01の永続化配線（上記参照）まで本番で到達不可能**
だった（`dawn-sector-node` は `InMemoryEventStore` のみで稼働していたため）。
配線完了により実際に到達可能になった。

**判断: 保留（トリガー付き）。** トレイトを `Result` 化する案は、戻り値を使う
6箇所以上の `apply_*_command` の戻り値型変更（`bool` → `Result<bool, _>`）に波及し、
かつ「tick処理中に一部のイベントだけappend失敗する」状態はINV-005（tick決定性）的に
中途半端な復旧ができない。1 Sector = 1 プロセス（8D-4）構成では panic = そのプロセスのみ
クラッシュし、再起動時にスナップショット+ホットログから復旧する設計（ADR-0017、
上記の永続化配線で実際に動作確認済み）なので、crash-only としての panic 自体は
不合理ではない。8D最小化方針に照らし、全面 `Result` 化より panic メッセージの充実化・
意図の明文化（トレイトdocコメントへの追記）の方が費用対効果が高いと判断し保留する。

再評価トリガー: 実機運用でディスクフルによる予期しないクラッシュが実際に発生したとき、
または `dawn-sector-node` がマルチSector・マルチスレッド構成に変わり panic の影響範囲が
1Sectorを超えるようになったとき。

---

## 改善ロードマップ

### 完了済み

**Phase 2〜8D（2026-06-19〜2026-06-30、アーカイブ済み）**: node.rs のサブモジュール化
（commands/navigation/serialization/sector_map/ship_registry/tick/spawner_logic/tackle/
snapshot_io/apply_event/transit_flow）、main.rs・serve.rs・data_loader.rs の分割、
`SimWorld` クエリヘルパー追加、`dawn-sector`/`dawn-replication` 新設（ADR-0026/0027）、
Phase 8D 全項目（TCP replication/Raft transport/本番バイナリ `dawn-sector-node`/
Raspberry Pi 実機検証 PASS）、命名整理（`navigation.rs`/`galaxy.rs`）、`CelestialBodyDef.sector`
追加、M-4（WS境界集約）・M-5（replication消費側 `ReplicaSet`）解消、R-1（`navigation.rs`
1092行を warp/approach/navigation に3分割）、runtime tick pipeline collapse、AoI delivery
deepening（`dawn-sector::aoi::AoiDelivery` への集約でM-6のAoI重複解消）、Sector Node runtime
deepening、production outbound replication publisher deepening、Client admission deepening
（`client_admission.rs`）、Sector Transit プロトコル公開面 5→2 集約。全て純粋移動/deep module化で
挙動変更なし、`cargo test --workspace` 全件通過を都度確認済み。詳細な差分は各PRのコミット履歴を参照。

| 作業 | 完了日 | 内容 |
|---|---|---|
| `dawn-sector-node` への永続化配線 | 2026-07-01 | `/improve-codebase-architecture` で「`EventStore::append` がinfallibleと嘘をついている」と指摘されたのを調査する過程で、より大きな問題を発見: `dawn-sector-node`（本番バイナリ）は `SimulationNode::new`（デフォルト `InMemoryEventStore`）で動いており、`FileEventStore`/`checkpoint()`/`CheckpointScheduler`/`restore_from`（Phase 3 実装・テスト済み）は本番に一切配線されていなかった（`maybe_checkpoint` の呼び出しは `dawn-simulation/src/bench.rs` のみ）。`NodeConfig` に永続化パス4フィールドを追加し、`build_node` でスナップショットの有無により新規/復元を分岐（`StateSnapshot::load` が `NotFound` なら新規、それ以外のエラーなら panic——サイレントなデータ損失を避ける）。復元時は `spawn_npcs` を呼ばない（NPC重複生成防止、`is_fresh` フラグで判定）。tickループに `CheckpointScheduler::maybe_checkpoint` を配線し、チェックポイント失敗はログのみで継続（ホットログへのappendは別経路で動き続ける）。`SectorNodeRuntime`/`ClientAdmission`/`AoiDelivery::deliver_frame` を `<S: EventStore>` でジェネリック化し `SimulationNode<FileEventStore>` に対応。実機での起動→kill→再起動でtick/log_indexが継続し、NPCが重複生成されないことを手動確認済み。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| Steering-mode 排他制御の非対称性を是正 | 2026-07-01 | Warpが`clear_steering_modes`を呼ばずOrbit中断後も`OrbitComp`が残る、Approachに`is_warping`ガードがない、Aligningフェーズ中はWarp優先チェックが素通りする、という実害のある非対称性3件を修正。`begin_maneuver`ヘルパー（`orbit.rs`）に共通スカフォールドを集約。回帰テスト5本追加 |
| M-7 ClientCommand dispatch 統一（Issue #56） | 2026-07-01 | `ClientCommand` enum を `dawn-actor` → `dawn-core` へ移動（`dawn-actor` は `pub use dawn_core::ClientCommand` で後方互換維持）。`dawn-sector::node::SimulationNode::apply_client_command(player_id, cmd, lock_commands) -> Option<ClientCommandFollowup>` を新設し、`dawn-sector-node/src/runtime.rs` の13分岐 match と `dawn-simulation/src/serve/` の `apply_common_command`（両バイナリの重複）を1呼び出しに統一。`ClientCommandFollowup` で Jump と RefreshFitting を呼び出し元に返す。`cargo test --workspace` 全件通過 |
| Client handshake payload の集約（PR #59） | 2026-07-02 | `single.rs`（dawn-simulation）と `client_admission.rs`（dawn-sector-node）が、identity 選択後の InitialState/PlayerLoadout JSON 組み立てを同一コードで重複していた（identity 選択自体は resume 対応の有無で別物なので統一対象外）。`dawn_sector::node::SimulationNode::build_handoff_payload(ship_id, aoi_cell_size) -> HandoffPayload` を新設し両呼び出し元から呼ぶ形に統一。ユニットテスト1件追加。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| Jump proposal orchestration 統一 | 2026-07-02 | `apply_jump_with_fallback` の outcome（in-range → Raft へ `TransitOp::Request` 提案）と auto-jump 提案パスが、`dawn-sector-node/src/runtime.rs` と `dawn-simulation/src/serve/{cluster,runtime}.rs` の3箇所に重複していた。`dawn_sector::transit::propose_jump` / `propose_auto_jump` を新設し、fallback chain の結果を Raft 提案へ橋渡しする部分を1箇所に集約。呼び出し元は返り値の `JumpOutcome`/`Option<SectorId>` を自分のログ整形にだけ使う。`SimulationNode::set_spawn_anchor_abs` を `pub(super)`→`pub(crate)`（`#[cfg(test)]` のまま）に広げ、新規テスト2件を追加。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| Module force-off の3実装統一（ADR-0035） | 2026-07-03 | Capacitor 枯渇・Range Gate・player-issued deactivate の3箇所が「`is_active=false; cycle_remaining=0; target_ship_id=None`」というスロット変更を個別に手組みしていた（`forced_reason`/イベント構築/`apply_fitting` タイミングは各自の関心事のため据え置き）。`FittedSlot::force_off()`（`dawn-ecs/src/components/fitting.rs`）に集約。副作用として `commands.rs::write_module_slot_state` の player-deactivate 経路が `cycle_remaining` をリセットしていなかったバグを解消（PR #65）。同時に発見した `apply_event.rs` の `ModuleDeactivated` リプレイ側の同型バグも別issue化して即修正（PR #65 で `force_off()` 適用、Issue #64 で記録）|
| Range Gate / Tackle の距離計算を `ship_distance` に統一 | 2026-07-03 | `range_gate.rs::is_target_within_range` と `tackle.rs::process_tackle` が、`SimulationNode::ship_distance` と同じ f64-アンカー合成の距離計算をそれぞれ手組みしていた（ADR-0029 精度パターン）。`is_target_within_range` は `Entity` 引数を `ShipId` に変更し `ship_distance` へ委譲、`tackle.rs` は手組みのdelta計算を `ship_distance` 呼び出しに置換（未使用になった `PositionComp` importも削除）。挙動変更なし（PR #66） |
| ShipRegistry が削除処理を一元所有 + `reapply_fitting` 統一（`/improve-codebase-architecture` 候補1+4） | 2026-07-03 | Ship の同一性は `index`/`type_ids`/`owners`/`by_player`（`ShipRegistry`）+ `base_stats`（`SimulationNode`）の5マップに分散しており、削除時の「4-6行の手組み削除シーケンス」が combat death（tick.rs）・`ShipDespawned`/`ShipDestroyed` replay（apply_event.rs）・Sector Transit 離脱（transit_flow.rs）の4箇所に重複、うち transit_flow.rs の1箇所は `owners`/`by_player` の削除を欠落させ、転移した player ship の所有権エントリがダングリングする実バグがあった。`ShipRegistry::remove(ship_id, world)` が4マップの削除+ECS despawn を一元所有し、`SimulationNode::remove_ship` がこれに `base_stats` 削除を足す薄いラッパーとして全4箇所から呼ばれる形に統一（回帰テスト `export_transit_clears_ownership_maps_for_a_player_ship` を追加）。あわせて `base_stats` 参照＋`apply_fitting` の反復（tick.rs/apply_event.rs/commands.rs/inventory.rs/range_gate.rs/spawner_logic.rs の計9箇所）を `SimulationNode::reapply_fitting(ship_id)` に統一（force-off 系は引き続き `ShipFitted` を発行しない）。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過（PR #67） |
| アンカー合成ヘルパーを2つの真のコアに統一（`/improve-codebase-architecture` 候補2） | 2026-07-03 | ADR-0029 のアンカー+offset 合成が `node/mod.rs`・`node/approach.rs`・`node/warp.rs` に7つの近い実装として散らばり、シグネチャと精度が微妙に異なっていた（過去セッションの「近くの船が射程外と誤判定される」バグと同じ精度クラス）。`entity_absolute_f64`（offset+anchor→絶対座標）と `dest_in_ship_frame_abs`（絶対座標→船のアンカー相対）の2つのコアへ集約し、`ship_absolute`/`entity_absolute`（f32版）はどちらも前者に委譲。`dest_in_ship_frame_abs` は4つの node サブモジュールから呼ばれるため `approach.rs` から `node/mod.rs` へ移設、`warp.rs::dest_in_ship_frame` はアップキャストしてそれに委譲する形に置換。挙動変更なし。`cargo test --workspace`（194/194）/ `fmt` / `clippy -D warnings` 全件通過（PR #68） |
| Bot AI 決定ループを `node/bot_ai.rs` へ抽出（`/improve-codebase-architecture` 候補3） | 2026-07-03 | `spawner_logic.rs` が spawn mechanics（ECS 挿入・inventory seed・`ShipSpawned` 発行）と Bot AI 決定ロジック（`process_bots` — target selection・低HP時の退避・engage range 操船・武器起動）という無関係な2つの関心事を同居させていた。両者を繋ぐのは「bot もspawnされた船である」という偶然のみで、`process_bots` は tick loop から呼ばれ spawn からは呼ばれない。`process_bots`（と、それをテストする2件のテスト）を新設 `node/bot_ai.rs` へ移動。`spawn_bot_ship`（船を作り `IsBotComp` を付けるだけの spawn mechanics）は `spawner_logic.rs` に残置。純粋移動、挙動変更なし。`spawner_logic.rs` 881→575行に縮小し R-3 の観察対象から外れた。`cargo test --workspace`（194/194、移動した2テストは `node::bot_ai::tests` で再確認）/ `fmt` / `clippy -D warnings` 全件通過（PR #69） |
| R-4 `node/mod.rs` フィールド定義と補助impl分離 | 2026-07-07 | 座標合成アクセサ群（`entity_absolute`/`entity_abs_pos`/`entity_abs_pos_f64`/`entity_absolute_f64`/`dest_in_ship_frame_abs`/`ship_distance`/`ship_distance_to_point`/`ship_anchor_and_offset`）+ `debug_assert_missing_anchor` を新設 `node/coordinates.rs` へ純粋移動。`mod.rs` は939→821行、impl 748→700未満に復帰。可視性・挙動変更なし |
| R-5 `dawn-actor/protocol.rs` 分割 | 2026-07-08 | 前回レビューで保留起票した `dawn-actor/src/protocol.rs` 1003行（impl 701）を、`protocol/mod.rs` / `client_command.rs` / `server_event.rs` / `hello_resume.rs` へ分割。`mod.rs` は wire protocol の入口・schema freshness test・統合テストだけを持つ薄い束ね役になり、server/client 各 message family の変換は独立した deep module に移った |
| Owned ship / Active ship モデル実装（ADR-0037、Phase 9B-5 Assemble の前提） | 2026-07-07 | `ShipRegistry.by_player`（1player=1shipの暗黙前提）を `active_ship` に改名し、`owners`（既に複数所有対応済みだった）と分離。`remove()` は削除される船が実際に active だった場合のみ `active_ship` を消すよう修正（複数所有時に別の所有船削除でactiveポインタが誤って消える潜在バグを先回りで解消）。`SelectActiveShipCommand`（station-local 切替のみ）を新設。操縦系/Undock コマンド（Move/Stop/Approach/Warp/Orbit/KeepAtRange/Jump/LockOn/Activate/Deactivate/Undock）は `ship_id` を持たず常に caller の active ship へ解決（`is_active_ship`）、station 管理系（Fit/Unfit/Dock/BuildPackagedShip/DisassembleShip）は `ship_id` を維持し `owns_ship` のまま。wire protocol・スキーマ・Godotクライアント（`connection.gd`/`main.gd`）を追従。`cargo test --workspace`（229/229 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過、GdUnit4 164/164 通過。詳細は `docs/architecture/ownership.md` §7・ADR-0037 |
| Phase 9B-5 Assemble コマンド実装 + RefreshFitting player_id 化（`/add-event`） | 2026-07-07 | `AssembleCommand`/`ShipAssembled`（`dawn-core`）と `SimulationNode::assemble_ship_owned`（`crates/dawn-sector/src/node/station.rs`、`Result<ShipId, StationOperationRejection>` — 既存 `StationOperationOutcome` は失敗時にも実在する `ship_id` を要求するため不採用という設計判断あり）を新設。ECS挿入コアを `spawn_ship` から `insert_ship_entity`（`spawner_logic.rs`、`pub(super)`）へ抽出し `assemble_ship_owned`・`apply_event` のReplay armと共有（`ShipSpawned`のReplay armは別実装のまま、意図的にリスクを取らず不統一のリスクを回避）。`active_ship` は自動変更しない（ADR-0037）ため、唯一の船をDisassembleして詰んだプレイヤーは Assemble → `SelectActiveShipCommand` → Undock で復帰可能に。実装過程で発見した副次バグ: `ClientCommandFollowup::RefreshFitting` が `ShipId` を持っていたため、Disassemble後に削除済みship_idからplayer_idを解決できず更新後のStation Inventoryがクライアントに一切届かない実バグがあった（`/improve-codebase-architecture`ではなくユーザー報告「disassembleしても何も起きない」で発覚）。`RefreshFitting(PlayerId)` に変更し、`build_player_loadout_json_for_player`（active shipがあれば既存経路に委譲、無ければ空艤装+station inventoryのみを返す）を新設して修正。wire protocol（`AssembleCommand`/`ShipAssembled`）追加・スキーマ再生成。回帰テスト（Assemble受理/却下3件、active_ship不変、Disassemble後もstation inventoryが届くことを検証する回帰テスト1件、wire round-trip2件）追加。`cargo test --workspace`（235/235 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過。クライアントUI（roadmap.md §12 タスク8の方針通り、server側完了後に着手）は未着手。詳細は `docs/architecture/ownership.md` §8 |
| DisembarkCommand 実装（ADR-0037、船を降りる操作の一級化） | 2026-07-07 | `DisembarkCommand`（`dawn-core`、フィールドなし・`UndockCommand`と同型）と `SimulationNode::disembark_owned`（`Result<ShipId, StationOperationRejection>` — Assembleと同じ理由で `StationOperationOutcome` は不採用）を新設。ドック中に active ship を「所有権・ドック状態はそのまま、操縦対象からだけ外す」ことを初めてプレイヤーが能動的に選べるようにした（従来は唯一の船をDisassembleした際の事故でしか到達しなかった状態）。Session-local・event-sourcedではない（`SelectActiveShipCommand`と同格、`DomainEvent`もwireイベントも新設せず）。client側もキーバインド `[X] Disembark`（`input_decoder.gd`/`connection.gd`/`main.gd`）まで含めて実装（Assembleとは異なりUIを別タスクに分離しなかった）。wire protocol（`DisembarkCommand`）追加・スキーマ再生成。回帰テスト（受理/却下2パターン、Disembark→SelectActiveShip→Undockのラウンドトリップ）+ GdUnit4テスト2件追加。`cargo test --workspace`（239/239 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過、GdUnit4 166/166 通過。詳細は `docs/architecture/ownership.md` §8 |
| Disembark後のクライアント可視性ギャップ修正 + station近接表示の複数化 | 2026-07-07 | ユーザー報告「dockした状態でXを押しても何も変わらない」で発覚: `PlayerLoadout`にはどの船の艤装かを示すフィールドが無く、クライアントはactive shipが変わったことを検知できなかった。`PlayerLoadout`に`active_ship_id: Option<u64>`（`null`=船なし）を追加し、`build_player_loadout_json`内で呼び出し元が渡した`ship_id`をそのまま返すのではなく`player_id`から実際の`active_ship`を再導出する形にした（呼び出し元が古い`ship_id`を渡していても正しい値を返す）。client側は`player_loadout.gd::active_ship_id()`が`main.gd`の`_player_ship_id`を更新（既知の船or-1の場合のみ、別の未知の所有船への切替はカメラ再アタッチが必要になるため対象外）。HUDも「Disembarked at: X (no active ship)」を新設。あわせて`_nearby_station_id`（単数int）を`_nearby_station_ids`（`Array[int]`、距離順）に変更し複数ステーション同時近接に対応、`[D]`は最も近い方にドック。実装中に発見したGDScript typed-array落とし穴（未型付き`[]`リテラルを`Array[int]`プロパティへ動的setterまたは三項演算子経由で代入すると失敗する）も修正。`cargo test --workspace`（242/242 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過、GdUnit4 168/168 通過。詳細は `docs/architecture/ownership.md` §8 |
| 新規プレイヤーへのスターターPackagedShip付与 + 複数所有船切り替えUI | 2026-07-08 | `spawn_player_ship_at`で新規プレイヤーに`PackagedShip`を1隻自動付与し、Disembark→Assemble→SelectActiveShip→Undockの一連を初回接続から即座に試せるようにした（既存テスト2件の期待値をベースライン+1へ更新、新規テスト1件追加）。`PlayerLoadout`に`owned_ships: [{ship_id, ship_type_id, ship_type_name, docked_station_id, is_active}]`を追加（新設`owned_ships_json`が`ShipRegistry.owners`を逆引き）。clientのインベントリパネルに3列目「SHIPS」を追加し、非activeな行クリックで`SelectActiveShipCommand`を送信（`connection.gd::send_select_active_ship_command`新設）。実装過程で`StateSnapshot`が`owners`/`active_ship`を実際には永続化していない（`ownership.md`の既存記述が不正確だった）ことを発見・記録（今回は対象外、別途対応）。`cargo test --workspace`（244/244 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過、GdUnit4 171/171 通過。詳細は `docs/architecture/ownership.md` §8 |
| Ship cargo / Station inventory UI分離 + TransferToStationCommand実装 | 2026-07-08 | ユーザー報告「ステーションのインベントリと船のインベントリがごっちゃになっていると思う」で発覚: インベントリパネルがship側cargoとstation inventoryを同じ「INVENTORY」列に`[Station]`プレフィックスのみで同居させており、roadmap.md タスク10自身が明記する要件（Ship側/Station側が混ざらないこと）に反していた。`hud_manager.gd`のパネルを3列→4列（FITTED/SHIP CARGO/STATION/SHIPS）に分割・520px→680pxへ拡幅して修正。続けてユーザー要望「船のインベントリをステーションインベントリにも移せるようにしたい」を`/grilling`で設計: 汎用コマンド（Module/ScrapMetal共通）、docked中のみ、全量転送のみ（部分数転送なし）で確定。UIトリガーは当初ScrapMetal限定案を提示したがユーザーが却下（「そっちのほうが複雑」）、右クリックによる統一ジェスチャーへ変更。`TransferToStationCommand { ship_id, station_id, item_id }`（`dawn-core`）と`InventoryComp::take_all`（`dawn-ecs`）、`SimulationNode::transfer_to_station_owned`（`node/inventory.rs`、`fit_module_owned`/`unfit_module_owned`と同じ`owns_ship`+`can_use_station`検証パターン、戻り値は`bool`——Assemble/Disembarkと異なり`ship_id`はコマンド自体から既知のため`Result<ShipId, _>`は不要）を新設。wire protocolは`ItemId`をそのまま送らず`item_type`(String)+`module_id`+`ship_type_id`のフラット表現（`ItemRow`と同じ形）を採用しスキーマ再生成。client側は`hud_manager.gd`の行データに`item_type`/`count`/`source`（"ship_cargo"/"station"、右クリック対象を`action`の値衝突なしに判定するため）を追加し、`main.gd::_handle_inventory_row_right_click`で送信。`cargo test --workspace`（249/249 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過、GdUnit4 171/171 通過（0 orphans）。詳細は `docs/architecture/ownership.md` §8 |
| ItemId→ItemRow JSON変換の重複除去（`/improve-codebase-architecture`） | 2026-07-08 | アーキテクチャレビューで発見: `ItemId`から7キー必須のItemRow JSON（`item_type`/`module_id`/`ship_type_id`/`name`/`kind`/`slot`/`count`）へ変換する`match ItemId { .. }`が`build_player_loadout_json`（ship inventory）と`station_inventory_json`（station inventory）の2箇所に独立してコピーされており、過去に実際踏んだキー欠落バグ（行がクライアント側で無言drop）と同じ形の再発リスクだった。`item_id_to_row_json(&self, item_id, count) -> Option<Value>`に集約し、両呼び出し元をこの1メソッド経由に置換。回帰テスト2件追加（全variantが7キーを満たすこと、未登録registryエントリで`None`を返すこと）。純粋なリファクタで挙動は変えない。`cargo test --workspace`（251/251 dawn-sector）/ `fmt` / `clippy -D warnings` 全件通過（client側の変更なし、GdUnit4再実行不要） |

### リファクタロードマップ（2026-06-23 追加・ADR-0029 後の再計測で起票）

機能追加（ADR-0029）で再び閾値を超えたファイルの分割を、過去の P7 系（`transit_flow.rs` /
`tackle.rs` / `snapshot_io.rs` を `node/mod.rs` から切り出した）と同じ「責務ごとに sibling
モジュールへ抽出、テストも実装と同じファイルへ」方式で行う。挙動は変えない（純粋な移動）。

#### ~~R-1~~: `node/navigation.rs` 1092 行の分割（完了・上記「完了済み」参照）

#### R-2（一部着手済み）: クライアント `main.gd` 1127 行

ADR-0029 以降に増加した `main.gd` は、`WorldSession` 抽出に続いて 2026-07-05 に
`WorldInteraction` を抽出し、1165→1127 に縮小。InitialState / AoI / HP / lock / tick-cap の
live world state は `client/scripts/world_session.gd`、selection state / double-click /
click→intent は `client/scripts/world_interaction.gd` へ移動済み。残りは scene lifecycle /
scene node generation / network send / HUD adapter のオーケストレーション層。さらなる分割は `.tscn` 化コンポーネントへの
シーン参照切れリスクが上回るため引き続き保留（client レビューの「採らない方針」と同根。
C-3 はフェイルファストガードで解消済み・2026-06-23 だが、これはこの判断とは独立——
更なる分割を妨げるのはシーン参照切れリスクそのもので、C-3 の有無は前提条件ではなかった）。

#### R-3（低優先・トリガー保留）: `node/` 系ファイルの再肥大（ADR-0031/0032/0033 後）

2026-07-06 の再計測で、`warp.rs`（1092、impl 533）/ `orbit.rs`（860、impl 324）/
`commands.rs`（962、impl 583）/ `station.rs`（972、impl 443）/ `transit_flow.rs`（949、
impl 368）が総行数で閾値帯に残っている。`spawner_logic.rs` は
`/improve-codebase-architecture` 候補3（PR #69）で `process_bots`（Bot AI 決定ループ）を
`node/bot_ai.rs` へ抽出済みで、下記トリガー一覧から外れたまま（623、impl 未計測だが
2026-07-03 時点で 575→現在623 の増分はテスト主体）。R-1（navigation.rs 分割）後に積まれた
Orbit/KeepAtRange（ADR-0031）・Inventory（ADR-0032）・Repair（ADR-0033）・Station（ADR-0034/9B）の
累積に加え、テストの増加がこれらのファイルの総行数を押し上げ続けている。
**この5ファイルは impl（テスト除く）が700行未満** で、下記トリガーは未発火。
`mod.rs` は同じ観察対象だったが、2026-07-06 の再計測で impl が700行を超えたため
**R-4 として切り出し、保留から着手判断へ格上げした**（下記参照）。

**根本原因**: 機能追加のたびに `node/` 直下へ impl + テストが積まれる構造。これ自体は
P7 系で確立した「責務ごとに sibling モジュールへ抽出」方式の想定内の蓄積であり、
設計の破綻ではない。

**判断: 保留（トリガー付き）。** 総行数はまだ大きいが、現時点では単一責務を保っている。
**今分割すると純粋移動の差分だけが増え、得が薄い。**

再評価トリガー（いずれかで着手）:
- いずれかの **impl 部分**（テスト除く）が ~700 行を超えたとき。
  - `warp.rs` → `process_warp` / Hermite warp 幾何 / コマンド・drain に3分割。
  - `orbit.rs` → Orbit / KeepAtRange の共有幾何と command application を分離。
  - `commands.rs` → command dispatch とバリデーション本体、9B station commands を分離。
  - `station.rs` → dock/undock、station inventory、build/disassemble を分離。
  - `transit_flow.rs` → Request 側と Commit 側のハンドラを分離。
- または `node/` のファイル総数が増えて「どこに何があるか」の見通しが実際に悪化したとき。

#### ~~R-4~~（新設・2026-07-06・**完了** 2026-07-07）: `node/mod.rs` の impl が700行トリガーを超過

`node/mod.rs` は 2026-07-03 時点で impl 641 行（総行数 829）とR-3の観察対象に含まれ、
「700行超で着手」というトリガー付きで保留されていた。2026-07-06 の再計測で総行数 936・
impl 748 行と判明し、**R-3 自身が定めたトリガーが発火した**。

**根本原因**: R-3 の過去の記述が既に指摘していた通り、`mod.rs` は「フィールド定義（構造体宣言・
定数）」と「補助 impl（ヘルパーメソッド群）」が同居する構造になっている。ADR-0031/0032/0035 の
たびにフィールドと対応する小さな impl メソッドが両方とも `mod.rs` に積まれ続けた。

**判断: 直す（改善ロードマップに起票）。** トリガーが明示的に発火した以上、保留を続ける理由がない。
R-3 が示していた分割方針（フィールド定義と補助 impl の分離）に沿って、次のいずれかの形で
分割する:
- `node/fields.rs`（仮）に `SimulationNode` の構造体定義・フィールド・定数を移し、`mod.rs` は
  補助 impl（サブモジュールから呼ばれる共有アクセサ、`dest_in_ship_frame_abs` 等）に絞る。
- または既存の sibling モジュール（`navigation.rs`/`approach.rs` 等）が呼ぶ共有アクセサ群だけを
  新規 `node/shared.rs`（仮）へ抽出する。

どちらの形にするかは実装着手時に既存コードを読んで決定する（本レビューは分析のみ）。
純粋な移動として行い、挙動変更は伴わないこと。

再評価トリガー: このリファクタが完了し次第、次回レビューで「完了済み」表へ移動する。

**2026-07-07、一部着手（`/improve-codebase-architecture` 発の deepening）**: 「補助 impl」の
中身を精査した結果、`entity_absolute_f64`/`dest_in_ship_frame_abs`/`ship_distance` は単なる
共有アクセサではなく、`AnchorTable`（`anchor.rs`、ADR-0029 の座標合成代数）の一部を
mod.rs 側で再実装していたと判明。`AnchorTable` に `to_relative()`（`absolute()` の逆変換）を
新設し、`rebase()` をその合成として書き直したうえで、`entity_absolute_f64`/
`dest_in_ship_frame_abs` は `anchor_table.absolute()`/`to_relative()` を呼ぶだけに、
`ship_distance` は各 Ship を `(AnchorId, offset)` に解決してから `anchor_table.distance()` に
委譲する形に置き換えた（f32 ラッパー `entity_absolute`/`entity_abs_pos` は ECS 由来のオフセット
読み出しが関心事のため mod.rs に残置）。挙動変更なし・`cargo test --workspace` / `fmt` /
`clippy -D warnings` 全件通過。`CONTEXT.md` に Anchor の語彙を追加済み。

**2026-07-07、R-4 本体を完了（`/improve-codebase-architecture` → `/grilling` 経由）**:
座標合成アクセサ群（`entity_absolute`/`entity_abs_pos`/`entity_abs_pos_f64`/
`entity_absolute_f64`/`dest_in_ship_frame_abs`/`ship_distance`/`ship_distance_to_point`/
`ship_anchor_and_offset`）と、その両方が使う `debug_assert_missing_anchor` を新設
`node/coordinates.rs` へ `impl<S: EventStore> SimulationNode<S>` ブロックごと移動（可視性は
`pub(super)`/`pub`のまま完全維持）。`mod.rs` は構造体宣言・定数・コンストラクタ・population
backstop・identity/observation アクセサに絞られ、939→821 行、impl は700行未満に戻った。
純粋移動で挙動変更なし。`cargo test --workspace`（dawn-sector 229/229）/ `fmt` /
`clippy -D warnings` 全件通過（既知の無関係な失敗1件: `wire_schema_doc_is_up_to_date` は
チェックイン済み `.schema.json` の改行コード CRLF/LF 差分によるもので、この変更以前から
発生していた別問題）。

#### ~~R-5~~（新設 2026-07-08・**完了** 2026-07-08）: `dawn-actor/src/protocol.rs` の深分割

前回レビューでは `dawn-actor/src/protocol.rs` が 1003 行（impl 701）まで膨らみ、
`EventJson` / `ClientCommandJson` / Hello/resume 解析 / schema freshness test が
単一ファイルへ積み重なっていた。自然な分割軸は既に見えており、実際のコードでも
server->client / client->server / hello-resume が別の関心事として育っていた。

**根本原因**: wire protocol という単一責務の中で、メッセージ family ごとの進化速度が
異なるのに、実装上の所有権が1ファイルに閉じ込められていたこと。

**判断: 解消済み。** 現在は `protocol/mod.rs`（入口と統合テスト） /
`protocol/client_command.rs`（client -> server 変換） /
`protocol/server_event.rs`（server -> client 変換） /
`protocol/hello_resume.rs`（Hello/resume handshake）へ分割済み。最大ファイルは
`protocol/mod.rs` 710 行だが、その大半は統合テストで、変換ロジック本体は family ごとの
deep module に移っている。次回以降は `client_command.rs` または `server_event.rs`
のどちらかが単独で watch 帯に入ったときだけ個別に再評価すればよい。

### 未完了・保留

上記リファクタロードマップ以外で残るのは以下。いずれも現時点では新しい module / crate を
増やすより保留・観察の方が費用対効果が高い、と判断した項目。

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| R-2 client `main.gd` 分割 | 品質・一部着手済み | `WorldSession`・`WorldInteraction`・`WorldPresentation` 抽出で live world state / world interaction policy / world visual side effect を移動し、`main.gd` は 948 行（詳細・最新値は architecture-review-client.md）。残る scene lifecycle / node generation / network send / HUD adapter は `.tscn` 化コンポーネントへのシーン参照切れリスクが上回るため保留（C-3 とは無関係） |
| R-3 `node/` 系再肥大（warp/orbit/commands/station/transit_flow） | 品質・保留 | 総行数は閾値帯だが impl は全て700未満・増分はテスト主体。impl が 700 超でファイル別に分割（トリガー付き・上記 R-3）。`mod.rs` はトリガー発火のため R-4 として分離 |
| ~~R-5 `dawn-actor/protocol.rs` の impl が700行トリガーに到達~~ | 完了 → 「完了済み」参照 | 2026-07-08、`protocol/mod.rs` / `client_command.rs` / `server_event.rs` / `hello_resume.rs` へ分割し、単一ファイルの watch 状態を解消 |
| ~~R-4 `node/mod.rs` impl 700行超過~~ | 完了 → 「完了済み」参照 | 2026-07-07、座標合成アクセサ群 + `debug_assert_missing_anchor` を `node/coordinates.rs` へ純粋移動。939→821行、impl700行未満に復帰 |
| 8D-5 Raspberry Pi 実機検証 | 完了 → 「完了済み」参照 | 2026-07-01、reachability/tick-sla/failover 3項目とも PASS。詳細は `docs/process/8d5-hardware-notes.md` |
| M-3 `SectorSimulatorActor` 密結合 | 品質・保留 | 本番パス外（in-process テスト/ベンチ専用）。P9-1 撤回。優先度低 |
| M-6 アプリ層 adapter 重複（`data_loader` / `spawn_npcs`） | 許容重複（縮小） | AoI / production runtime / Command dispatch は deep module 化済み（M-7 解消で Command dispatch 項目を削除）。残る data_loader / NPC spawn は低頻度 glue として許容。再評価トリガー付き |
| ~~M-7 Player Command Dispatch のルーティングが `dawn-sector` 外に漏れている~~ | 完了 → 「完了済み」参照 | `ClientCommand` を `dawn-core` へ移動・`apply_client_command` を `SimulationNode` に追加し両バイナリで統一。Issue #56 |
| M-8 `fit_module`/`fit_module_owned` 共有テール重複 | 許容（新規 2026-07-01） | `inventory.rs` のモジュールコメントで意図的な分離と明記済み。テールのみの軽微な重複で優先度なし |
| M-9 `EventStore::append` がinfallibleと偽る | 品質・保留（新規 2026-07-01） | 永続化配線完了で実際に到達可能になったpanic経路。1プロセス1Sector構成ではcrash-only設計として不合理ではないため、全面Result化は見送り保留。実機クラッシュ発生 or マルチSectorプロセス化がトリガー |

採らない方針（恒久）:

- CRDT / LWW-Register は採らない（単一所有 + append-only log gossip）
- protobuf / `dawn-proto` は採らない（wire は postcard 再利用）
- TLS / 認証は第1次 LAN 検証では扱わない

---

### Phase 8 — 物理ノード分散の配線（Phase 8D 完了）

`dawn-replication`（ADR-0021/0027・Phase 8D）は 8D-2〜8D-4 を完了済み。
8D-5（Raspberry Pi 実機検証）も 2026-07-01 に完了（上記「完了済み」参照）。8D 全項目が完了。

---

### Phase 9 — 評価の総点検（決着）

Phase 9 時点で総合 A−（現在は B+、上記「現状評価」参照）で決着。新 crate は作らない方針
（M-3/M-6）、ADR-0029 後の再肥大は R-1 で解消済み。残る前進先は戦闘の深み（ADR-0016 §5）
といった機能側で、R-2（client `main.gd`）は保留のまま（client レビューの「採らない方針」参照）。

| 項目 | 状態 |
|---|---|
| P9-1（M-3 解消） | 撤回（下記） |
| P9-2（`CelestialBodyDef.sector`） | 完了 → 「完了済み」へ移動 |

#### ~~P9-1: M-3 解消~~（撤回・保留）

当初計画の「8D-5実機検証後にSectorSimulatorActor境界を疎結合化」は前提が崩れて撤回。
`SectorSimulatorActor`は本番パス外（M-3参照）で8D-5はこの境界を経由しないため無意味だった。
残る品質観点は低頻度glue重複（M-6・許容）と密結合（M-3・本番パス外で低優先）のみ（「未完了・保留」参照）。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（ClientConnection 境界）— replication 責務は `dawn-replication` へ移動済み
