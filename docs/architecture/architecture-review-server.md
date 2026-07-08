---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md
date     : 2026-07-09（定期再計測。`station.rs` の deepening を反映して `station_lifecycle.rs` / `station_materialization.rs` を新規記録。server 総合 B+ 維持、client 側は別途 architecture-review-client.md 参照）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

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
> これにより前回起票した R-5 は解消済みに移動する。

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
| `dawn-sector-node` への永続化配線 | 2026-07-01 | 本番ノードを `FileEventStore` / snapshot / checkpoint に配線。復元時の NPC 重複も防止。 |
| Steering-mode 排他制御の非対称性を是正 | 2026-07-01 | Warp / Approach / Orbit の排他制御を `begin_maneuver` 系へ整理。回帰テスト追加。 |
| M-7 ClientCommand dispatch 統一（Issue #56） | 2026-07-01 | `ClientCommand` を `dawn-core` へ移し、`apply_client_command` で両バイナリの dispatch を統一。 |
| Client handshake payload の集約（PR #59） | 2026-07-02 | InitialState / PlayerLoadout handoff を `build_handoff_payload` に集約。 |
| Jump proposal orchestration 統一 | 2026-07-02 | Jump fallback 後の Raft 提案組み立てを `propose_jump` / `propose_auto_jump` に集約。 |
| Module force-off の3実装統一（ADR-0035） | 2026-07-03 | capacitor / Range Gate / player deactivate の OFF 処理を `FittedSlot::force_off()` に統一。 |
| Range Gate / Tackle の距離計算を `ship_distance` に統一 | 2026-07-03 | 距離計算の手組みを廃止し `ship_distance` に統一。 |
| ShipRegistry が削除処理を一元所有 + `reapply_fitting` 統一（`/improve-codebase-architecture` 候補1+4） | 2026-07-03 | ship 削除シーケンスを `ShipRegistry::remove` / `SimulationNode::remove_ship` に集約。`reapply_fitting` も共通化。 |
| アンカー合成ヘルパーを2つの真のコアに統一（`/improve-codebase-architecture` 候補2） | 2026-07-03 | 座標合成を `entity_absolute_f64` / `dest_in_ship_frame_abs` に集約。 |
| Bot AI 決定ループを `node/bot_ai.rs` へ抽出（`/improve-codebase-architecture` 候補3） | 2026-07-03 | `spawner_logic.rs` から Bot AI を分離し、spawn mechanics と分担。 |
| R-4 `node/mod.rs` フィールド定義と補助impl分離 | 2026-07-07 | 座標合成アクセサ群を `node/coordinates.rs` へ純粋移動。 |
| R-5 `dawn-actor/protocol.rs` 分割 | 2026-07-08 | `protocol/mod.rs` / `client_command.rs` / `server_event.rs` / `hello_resume.rs` へ分割。 |
| Station operations module の deepening（`/improve-codebase-architecture`） | 2026-07-09 | `station.rs` を shared vocabulary に縮小し、実装を `station_lifecycle.rs` / `station_materialization.rs` へ分割。 |
| Owned ship / Active ship モデル実装（ADR-0037、Phase 9B-5 Assemble の前提） | 2026-07-07 | `owners` と `active_ship` を分離し、操縦系コマンドを active ship 解決に統一。 |
| Phase 9B-5 Assemble コマンド実装 + RefreshFitting player_id 化（`/add-event`） | 2026-07-07 | `AssembleCommand` / `ShipAssembled` を実装し、followup を `PlayerId` 基準に修正。 |
| DisembarkCommand 実装（ADR-0037、船を降りる操作の一級化） | 2026-07-07 | `DisembarkCommand` と `disembark_owned` を追加。 |
| Disembark後のクライアント可視性ギャップ修正 + station近接表示の複数化 | 2026-07-07 | `active_ship_id` を wire に追加し、HUD / station proximity 表示を追従。 |
| 新規プレイヤーへのスターターPackagedShip付与 + 複数所有船切り替えUI | 2026-07-08 | starter packaged ship と owned ship roster UI を追加。 |
| Ship cargo / Station inventory UI分離 + TransferToStationCommand実装 | 2026-07-08 | HUD を cargo/station 分離し、`TransferToStationCommand` を追加。 |
| ItemId→ItemRow JSON変換の重複除去（`/improve-codebase-architecture`） | 2026-07-08 | `item_id_to_row_json` に集約して row schema drift を防止。 |

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

2026-07-06 の再計測では `warp.rs` / `orbit.rs` / `commands.rs` / `station.rs` /
`transit_flow.rs` が総行数で閾値帯に残っていた。2026-07-09 時点で `station.rs` は
deepening により観察対象から外れ、残る主な watch は `commands.rs` 1460・`warp.rs` 1024・
`orbit.rs` 790・`transit_flow.rs` 863。`spawner_logic.rs` は
`/improve-codebase-architecture` 候補3（PR #69）で `process_bots`（Bot AI 決定ループ）を
`node/bot_ai.rs` へ抽出済みで、下記トリガー一覧から外れたまま（623、impl 未計測だが
2026-07-03 時点で 575→現在623 の増分はテスト主体）。R-1（navigation.rs 分割）後に積まれた
Orbit/KeepAtRange（ADR-0031）・Inventory（ADR-0032）・Repair（ADR-0033）・Station（ADR-0034/9B）の
累積に加え、テストの増加がこれらのファイルの総行数を押し上げ続けている。
**この4ファイルは impl（テスト除く）が700行未満か、少なくとも単一責務が保たれている** ため、
下記トリガーは未発火。
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
  - `commands.rs` → command dispatch とバリデーション本体、肥大化した test cluster を分離。
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
| R-2 client `main.gd` 分割 | 品質・一部着手済み | `WorldSession`・`WorldInteraction`・`WorldPresentation` 抽出で live world state / world interaction policy / world visual side effect を移動し、`main.gd` は 1089 行（詳細・最新値は architecture-review-client.md）。残る scene lifecycle / node generation / network send / HUD adapter は `.tscn` 化コンポーネントへのシーン参照切れリスクが上回るため保留（C-3 とは無関係） |
| R-3 `node/` 系再肥大（warp/orbit/commands/transit_flow） | 品質・保留 | `station.rs` は 2026-07-09 の deepening で観察対象から外れた。残る watch は `commands.rs` / `warp.rs` / `orbit.rs` / `transit_flow.rs`。総行数は閾値帯だが、少なくとも現時点では責務単位は保たれている。impl が 700 超、または test cluster を含めた見通し悪化が実害化した時点でファイル別に分割（トリガー付き・上記 R-3）。`mod.rs` はトリガー発火のため R-4 として分離 |
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
