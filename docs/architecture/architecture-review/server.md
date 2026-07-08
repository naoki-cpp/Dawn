---
scope    : コードベース全体の保守性・設計品質レビュー — 構造評価（グレード・ファイルサイズ）
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-07-09（定期再計測。`station.rs` の deepening を反映して `station_lifecycle.rs` / `station_materialization.rs` を新規記録。server 総合 B+ 維持、client 側は別途 client.md 参照）
---

# Architecture Review — Dawn Codebase（構造評価）

Rust シニアアーキテクト視点での現状分析。**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

このファイルは「今どういう状態か」（グレード・ファイルサイズ・行数）だけを扱う。
issue の詳細・保留判断・トリガーは
[server-pending.md](./server-pending.md)、
解消済みの作業ログは
[server-completed.md](./server-completed.md) を参照。

---

## 現状評価

**総合: B+**（2026-07-09 再計測で維持。`station.rs` の deepening により
Station operations は shared vocabulary + sibling module へ整理され、前回の
「単一ファイルに dock/undock/build/assemble/disassemble/disembark が同居」という
watch は解消した。一方で `commands.rs` 1460・`warp.rs` 1024・`transit_flow.rs` 863・
`orbit.rs` 790 は依然 watch 帯で、特に `commands.rs` の再肥大がボトルネック軸として残るため
ファイルサイズ観点の総合は B+ のまま）

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector / dawn-replication が分離済み（ADR-0026/0027）。M-7 解消で `ClientCommand` を `dawn-core` へ移動し DAG が整理された（`dawn-sector` が `dawn-actor` 非依存のまま dispatch を保持できるようになった）。Player Command Dispatch のための新 crate は引き続き不要 |
| ファイルサイズ | B+ | 2026-07-09 再計測。`station.rs` 1288 は `station.rs` 50 / `station_lifecycle.rs` 374 / `station_materialization.rs` 404 に深分割され watch 解除。一方で `commands.rs` は 1460 まで伸び、`warp.rs` 1024・`transit_flow.rs` 863・`orbit.rs` 790・`mod.rs` 782 とあわせて watch 帯が続く。R-4/R-5 後の役割分担は保たれているが、軸の評価自体は B+ を維持 |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。`InventoryComp`（ADR-0032）・`RepairLayer`/`RepairApplied`（ADR-0033）・`ItemId`（ADR-0034、`dawn-core/src/item.rs`）も既存型設計に整合 |
| 重複 | A− | WS 境界は dawn-actor へ集約（M-4 解消）。AoI delivery、production runtime、Command dispatch は deep module 化済み（M-7 解消で `apply_client_command` が `SimulationNode` に集約）。2026-07-08、`ItemId -> ItemRow` JSON変換の重複（`serialization.rs` 2箇所）を `item_id_to_row_json` へ集約し解消済み。残る両バイナリ間グルー重複（M-6）・Fit経路のテール重複（M-8）は許容判断のまま |
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。`TransitOp::Commit` は ADR-0032 で `Box<ShipSnapshot>` 化しサイズ非対称を解消済み |
| AI開発由来 | A− | 命名汚染なし。残る `SectorSimulatorActor` の密結合（M-3）は本番パス外の in-process 専用で実害小 |

---

## ファイルサイズ一覧（2026-07-09 時点）

> **2026-07-09、全ファイル再計測（`/architecture-review`）。** 前回パス（2026-07-06/07）
> 以降に landed した9B-5/ADR-0037系の機能（Assemble/Disembark/複数船ロスターUI/
> TransferToStationCommand）で、`inventory.rs`（428→570）・`spawner_logic.rs`（623→669）・
> `apply_event.rs`（498→566）・`commands.rs`（dawn-core、473→551）・`events.rs`（657→694）・
> 今回の実測では、前回レビュー以降の deepening と整理を反映して
> `warp.rs` 1024・`spawner_logic.rs` 613・`orbit.rs` 790・`mod.rs` 782・`coordinates.rs` 174・
> `transit_flow.rs` 863・`station.rs` 50・`station_lifecycle.rs` 374・
> `station_materialization.rs` 404・`snapshot_io.rs` 640・`inventory.rs` 783・
> `dawn-core/src/commands.rs` 539・`serialization.rs` 1011・`apply_event.rs` 806・
> `commands.rs` 1460 に更新した。`dawn-actor` 側では単一の `protocol.rs` は消え、
> `protocol/mod.rs` 741 / `client_command.rs` 387 / `server_event.rs` 252 /
> `hello_resume.rs` 29 へ分割済み。
> これにより前回起票した R-5 は解消済みに移動する（詳細は completed.md）。

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/warp.rs` | 1024 | 🟡 R-1 新設（2026-06-23）。warp 幾何の単一責務だが総行数が閾値を超過。前回レビュー時の 1093 からは縮小したが、依然 watch 対象 |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 613 | 🟢 P4-2 + P7-1 + ADR-0029 + ADR-0032。残るのは spawn mechanics（spawn / inventory seed）のみで、R-3 の観察対象から外れたまま |
| `crates/dawn-sector/src/node/bot_ai.rs` | 347 | 🟢 `spawner_logic.rs` から `process_bots` を抽出した Bot AI 決定ループ。純粋移動、挙動変更なし |
| `crates/dawn-sector/src/node/orbit.rs` | 790 | 🟡 ADR-0031 新設。Orbit / Keep at Range の操船一式。単一責務で許容だが総行数は watch 帯 |
| `crates/dawn-sector/src/node/mod.rs` | 782 | 🟢 R-4 完了（2026-07-07）。`coordinates.rs` 抽出後の役割分担が維持され、構造体宣言・定数・コンストラクタ・population backstop・identity/observation アクセサへ責務が戻っている。`station_lifecycle.rs` / `station_materialization.rs` の sibling 宣言追加で微増したが、責務は変わらない |
| `crates/dawn-sector/src/node/coordinates.rs` | 174 | 🟢 R-4（2026-07-07新設）。`AnchorTable`（ADR-0029）呼び出し側の座標合成アクセサを一元化した deep module |
| `crates/dawn-sector/src/node/transit_flow.rs` | 863 | 🟢 `prepare_transit_commit`/`handle_transit_commit`（公開面 5→2 に集約）+ `rebase_after_transit`。大きいが責務は cohesive |
| `crates/dawn-sector/src/node/station.rs` | 50 | 🟢 2026-07-09、Station operations deepening 後の shared vocabulary module。`StationOperationOutcome` / `StationOperationRejection` だけを持ち、実装は sibling module へ移動 |
| `crates/dawn-sector/src/node/station_lifecycle.rs` | 374 | 🟢 2026-07-09 新設。dock / undock / active-ship selection / disembark / docked lock cleanup を所有する deep module |
| `crates/dawn-sector/src/node/station_materialization.rs` | 404 | 🟢 2026-07-09 新設。build / assemble / disassemble を所有する deep module。Ship materialization の検証と event append がここに集約 |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 640 | 🟢 P7-pre + ADR-0032（inventory 永続化）。ほぼテスト |
| `crates/dawn-sector/src/node/inventory.rs` | 783 | 🟢 ADR-0032 新設。fit/unfit_module_owned + transfer + seed + テスト。大きいが責務は単一 |
| `crates/dawn-sector/src/node/commands.rs` | 1460 | 🟡 P7-1 + ADR-0032 + M-7（Issue #56）+ ADR-0035 + 9B station commands。command dispatch と操作検証の責務は保っているが、総行数は再び強い watch 帯。2026-07-09 時点では Station-family dispatch を `command_station.rs` へ出した後も、回帰テストの蓄積と command family 増加で最大ファイルへ再肥大している |
| `crates/dawn-sector/src/node/serialization.rs` | 1011 | 🟢 InitialState / PlayerLoadout / handoff payload の組み立て。依然大きいが責務は単一で、`ItemId -> ItemRow JSON` 重複も `item_id_to_row_json` へ集約済み |
| `crates/dawn-sector/src/galaxy.rs` | 459 | 🟢 ADR-0029 AU→units 変換・ゲート AU 化 |
| `crates/dawn-sector/src/node/apply_event.rs` | 806 | 🟢 P7-pre + ADR-0032 + ADR-0035。replay apply の責務は単一。サイズは伸びたが、履歴再生の owner として一貫している |
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
| `crates/dawn-actor/src/protocol/mod.rs` | 741 | 🟢 R-5 完了（2026-07-08）。wire protocol の公開面と統合テスト・schema freshness test を束ねる薄い入口に縮小 |
| `crates/dawn-actor/src/protocol/client_command.rs` | 387 | 🟢 client -> server wire translation の deep module。`ClientCommandJson` / `parse_client_command` / schema 出力を集約 |
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

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（ClientConnection 境界）— replication 責務は `dawn-replication` へ移動済み

---

未解消の issue（root cause・decision・trigger）は
[server-pending.md](./server-pending.md)、
解消済みの作業ログは
[server-completed.md](./server-completed.md) を参照。
