---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md
date     : 2026-07-03（doc-sync。ADR-0035（PR #62）と PR #63（`FittingComp::slot_at_flat`/`iter_slots` 新設）の
行数がドキュメントに未反映だった乖離を訂正: 新規 `node/range_gate.rs` 382（impl 180）を追加記録、
`node/commands.rs` 693→795（impl 546）・`node/mod.rs` 800→794・`tackle.rs` 324→358・`apply_event.rs` 339→341・
`aoi.rs` 626→627・`events.rs` 584→603・`dawn-core/commands.rs` 395→401・`protocol.rs` 612→633・
`capacitor.rs` 504→479・`systems/fitting.rs` 333→326・`components/fitting.rs` 267→370。いずれも impl 700 未満で
R-3 トリガー未発火）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B+**（2026-07-01 維持。M-7（Issue #56）解消で `node/commands.rs` に `apply_client_command` が集約され重複観点はさらに改善。R-3 の4ファイル（warp/spawner/orbit/mod）は impl 700 未満でトリガー未発火のまま。`node/commands.rs` は合計 693 行（impl 440）まで増加したが単一責務を保っており 🟢）

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector / dawn-replication が分離済み（ADR-0026/0027）。M-7 解消で `ClientCommand` を `dawn-core` へ移動し DAG が整理された（`dawn-sector` が `dawn-actor` 非依存のまま dispatch を保持できるようになった）。Player Command Dispatch のための新 crate は引き続き不要 |
| ファイルサイズ | B+ | 2026-07-01 doc-sync で再計測（`wc -l` 実測、過去の記録値との乖離を訂正）: `warp.rs` 1094（impl 535）・`spawner_logic.rs` 881（impl 603）・`orbit.rs` 854（impl 318）・`node/mod.rs` 800（impl 612）。新設 `node/jump.rs` 186（impl 66）は閾値内で健全。`dawn-simulation/serve` は `runtime.rs` / `aoi_delivery.rs` へ、`dawn-sector-node` は `runtime.rs` へ分割され、各起動 loop は小さく維持。4ファイルとも impl は700行未満で R-3 トリガーは未発火だが、`node/mod.rs`（612）が最も閾値に近い → 次回 `/architecture-review` での再評価を推奨 |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。`InventoryComp`（ADR-0032）・`RepairLayer`/`RepairApplied`（ADR-0033）も既存型設計に整合 |
| 重複 | A− | WS 境界は dawn-actor へ集約（M-4 解消）。AoI delivery、production runtime、Command dispatch は deep module 化済み（M-7 解消で `apply_client_command` が `SimulationNode` に集約）。残る両バイナリ間グルー重複（M-6）は data loading / NPC spawn の低頻度 glue として許容判断 |
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。`TransitOp::Commit` は ADR-0032 で `Box<ShipSnapshot>` 化しサイズ非対称を解消済み |
| AI開発由来 | A− | 命名汚染なし。残る `SectorSimulatorActor` の密結合（M-3）は本番パス外の in-process 専用で実害小 |

---

## ファイルサイズ一覧（2026-07-01 時点）

> 2026-06-29 の前回計測から、構造変更はなし。各ファイルは継続中の機能追加（ADR-0031/0032/0033
> 系の延長）で増分しているのみ。R-3 の4ファイルは impl 行数（テスト除く）を併記し、
> トリガー（impl 700 超）未発火を確認した。`client_admission.rs`（その他クレート表）は
> 2026-06-29 の deepening（#41）がこれまで記録漏れだったため今回追加。

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/warp.rs` | 1094（impl 534） | 🟡 R-1 新設（2026-06-23）。warp 幾何の単一責務だが総行数が閾値を超過。impl は700未満でトリガー未発火（process_warp / warp 幾何 / コマンドへ分割は引き続き保留）。2026-07-01、`apply_warp_command` に `clear_steering_modes` を追加（下記「Steering-mode 排他制御の非対称性を是正」参照） |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 885（impl 603） | 🟡 P4-2 + P7-1 + ADR-0029 + ADR-0032（inventory seeding）+ ADR-0035（bot 武器起動に `target_ship_id` 添付、PR #63 で `iter_slots()` 使用に置換）。spawn / bot AI / inventory seed が同居。impl 700未満でトリガー未発火 |
| `crates/dawn-sector/src/node/orbit.rs` | 854（impl 317） | 🟡 ADR-0031 新設。Orbit / Keep at Range の操船一式。単一責務で許容、impl 700未満。2026-07-01、`begin_maneuver` ヘルパーを新設し `apply_orbit_command`/`apply_keep_at_range_command`/`apply_approach_command`（approach.rs）の共通スカフォールドを集約。Warp優先チェックを `has_active_warp`（全フェーズ）に修正 |
| `crates/dawn-sector/src/node/mod.rs` | 794（impl 606） | 🟡 P7-2 後 + ADR-0031/0032/0035 のフィールド・定数追加。PR #63 で `get_fitted_module_ids` の4-way chain を `FittingComp::iter_slots()` に置換（800→794）。700行の R-3 閾値に近く、次回 `/architecture-review` での再評価を推奨 |
| `crates/dawn-sector/src/node/transit_flow.rs` | 913（impl 366） | 🟢 `prepare_transit_commit`/`handle_transit_commit`（公開面 5→2 に集約）+ `rebase_after_transit`（#38）。impl は依然小さく、増分はテスト |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 580 | 🟢 P7-pre + ADR-0032（inventory 永続化）。ほぼテスト |
| `crates/dawn-sector/src/node/inventory.rs` | 459 | 🟢 ADR-0032 新設。fit/unfit_module_owned + seed + テスト |
| `crates/dawn-sector/src/node/commands.rs` | 795（impl 546） | 🟢 P7-1 + ADR-0032 + M-7（Issue #56）+ ADR-0035。M-7 で `ClientCommandFollowup` enum・`apply_client_command` メソッドを追加（509→693）。ADR-0035（PR #62）で `set_module_active` にターゲットバリデーション（Locked 必須・射程は仮適用→検証→ロールバック）を追加（693→799）。PR #63 で flat-index 境界計算を `FittingComp::slot_at_flat`/`iter_slots` 呼び出しに置換（799→795）。impl 700未満。単一責務（command dispatch + 操作検証）を維持 |
| `crates/dawn-sector/src/node/serialization.rs` | 507 | 🟢 ADR-0029 + ADR-0032（inventory / slot_capacity を PlayerFitting に追加）。2026-07-02、`build_handoff_payload`（`HandoffPayload { initial_state, player_fitting }`）を新設し、`single.rs`・`client_admission.rs` に重複していた InitialState/PlayerFitting 組み立てを1箇所へ集約（PR #59、450→507、ユニットテスト1件追加） |
| `crates/dawn-sector/src/galaxy.rs` | 360 | 🟢 ADR-0029 AU→units 変換・ゲート AU 化 |
| `crates/dawn-sector/src/node/apply_event.rs` | 341 | 🟢 P7-pre + ADR-0032（ShipFitted/ShipSpawned で inventory 復元）+ ADR-0035（ModuleActivated/Deactivated replay で `target_ship_id` 復元/クリア） |
| `crates/dawn-sector/src/node/tackle.rs` | 358 | 🟢 P7-pre。ADR-0035（PR #62）で距離判定を `entity_absolute_f64` の f64 差分に修正（真 AU スケールでの f32 丸め対策・ADR-0029 パターン準拠） |
| `crates/dawn-sector/src/node/range_gate.rs` | 382（impl 180） | 🟢 ADR-0035 新設（PR #62）。Range Gate System（Step 5.5）— Weapon/Tackle のターゲットが射程外に出たら強制 OFF（`ModuleDeactivated { forced_reason: OutOfRange }`）。PR #63 で flat-index 解決を `FittingComp::slot_at_flat_mut` に置換（403→382）。半分強はテスト |
| `crates/dawn-sector/src/aoi.rs` | 627（impl 307） | 🟢 `AoiDelivery`/`AoiSink`/`Observer`（旧 dawn-simulation・dawn-sector-node 重複の集約先）。半分弱はテスト。2026-07-01、`deliver_frame` を `<S: EventStore>` でジェネリック化 |
| `crates/dawn-sector/src/anchor.rs` | 292 | 🟢 ADR-0029 新設（AnchorTable・静的 f64 アンカー絶対座標） |
| `crates/dawn-sector/src/transit.rs` | 418（impl 240） | 🟢 PR #30 で `run_runtime_tick` / `RuntimeTickOutput` を追加。Request/Commit ハンドラが `prepare_transit_commit`/`handle_transit_commit` に委譲し Gate-lookup 知識を手放した。2026-07-02、`propose_jump` / `propose_auto_jump` を新設し、jump fallback outcome → `TransitOp::Request` 提案の組み立てを `dawn-sector-node`・`dawn-simulation` 双方の重複から集約（282→418、テスト2件追加でimpl 240） |
| `crates/dawn-sector/src/modules.rs` | 211 | 🟢 ADR-0033 で Active 修理モジュール定義を追加 |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 177 | 🟢 ADR-0032 で `ShipSnapshot.inventory` 追加 |
| `crates/dawn-sector/src/dilation.rs` | 164 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 173 | 🟢 |
| `crates/dawn-sector/src/node/approach.rs` | 577（impl 194） | 🟢 R-1 新設（2026-06-23）。approach 系 + ADR-0031 で clear_steering_modes 連携。2026-07-01、独自の検証チェックリストを `orbit.rs` の `begin_maneuver` 呼び出しに置き換え、Orbit/KeepAtRange と完全に同じ経路を通るように統一。同日、`apply_approach_jump_fallback`（1行ラッパー）を `jump.rs` へ移設・削除し、`apply_approach_command_with_auto_jump` を `pub(super)` 化 |
| `crates/dawn-sector/src/node/jump.rs` | 250（impl 88） | 🟢 新設（2026-07-01）。PR #54 で `apply_jump_with_fallback`（3択フォールバック）、PR #55 で `resolve_auto_jump`（auto-jump 判定）を追加。両 PR でテストも追加（186→250）。impl 88 行で健全 |
| `crates/dawn-sector/src/node/tick.rs` | 177 | 🟢 P4-1 + ADR-0031 Step 2.55/2.56 + ADR-0033 Step 6.5 配線 |
| `crates/dawn-sector/src/spawner.rs` | 133 | 🟢 |
| `crates/dawn-sector/src/ship_types.rs` | 91 | 🟢 |
| `crates/dawn-sector/src/node/navigation.rs` | 161 | 🟢 R-1 後。`can_propose_jump` / `can_propose_warp` + ADR-0017 dead-zone テスト |
| `crates/dawn-sector/src/node/ship_registry.rs` | 33 | 🟢 P3-1 |
| `crates/dawn-sector/src/node/sector_map.rs` | 24 | 🟢 P3-1 |

### dawn-actor（クライアント転送境界・M-4 集約先）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-actor/src/protocol.rs` | 633（impl 474） | 🟢 M-4 集約（DomainEvent↔JSON↔ClientCommand）。継続的なコマンド追加（ADR-0031/0032/0035 等）でパース分岐が増加（ADR-0035: `target_ship_id` パース + `reason` シリアライズで 612→633）。18+ variant・変更頻度高だが単一責務 |
| `crates/dawn-actor/src/client_connection.rs` | 259 | 🟢 ClientConnection trait + InProcess/Ws 実装。M-7（Issue #56）で `ClientCommand` 定義を `dawn-core` へ移動し `pub use dawn_core::ClientCommand` 再エクスポートに変更（297→259）。後方互換を維持しつつ定義がDAG下層へ |
| `crates/dawn-actor/src/ws_server.rs` | 263 | 🟢 M-4 集約（WsServer / PlayerSession）+ ADR-0032 `send_raw` |
| `crates/dawn-actor/src/lib.rs` | 28 | 🟢 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 629 | 🟢 Raft クラスター配線（in-process テスト用） |
| `crates/dawn-simulation/src/serve/mod.rs` | 355 | 🟢 P5-1 共通ヘルパー。M-7（Issue #56）で `apply_common_command` と `CommonCommandFollowup` を削除し `node.apply_client_command` 呼び出しに置き換え（437→355、-82行）。PR #34 で AoI delivery を分離済み |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 459 | 🟡 M-3（本番パス外・保留）。PR #30 で tick pipeline を `transit::run_runtime_tick` に寄せた |
| `crates/dawn-simulation/src/bench.rs` | 493 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 225 | 🟢 PR #30 で tick 後処理を `serve/runtime.rs` へ移動。PR #34 後は `AoiDelivery` を持ち、入力処理と runtime 呼び出し中心。2026-07-01、Jump の3択フォールバックを `dawn_sector::node::jump::apply_jump_with_fallback` 呼び出しに置き換え（旧来の自前3分岐を削除）。2026-07-02、Raft への `TransitOp::Request` 提案を `dawn_sector::transit::propose_jump` 呼び出しに置き換え（235→225） |
| `crates/dawn-simulation/src/serve/runtime.rs` | 182 | 🟢 PR #30 新設。auto-jump / ownership handoff / scoped InitialState resend を集約し、AoI delivery は `AoiDelivery` に委譲。2026-07-02、auto-jump の Raft 提案を `dawn_sector::transit::propose_auto_jump` 呼び出しに置き換え（192→182） |
| `crates/dawn-simulation/src/serve/aoi_delivery.rs` | 119 | 🟢 配信ロジック本体を `dawn_sector::aoi::AoiDelivery` へ移動。残りは `CellGrid` 構築・セッション loop・`SessionSink` adapter のみ |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 219 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 200 | 🟢 P5-1。PR #34 後は AoI delivery 詳細を `AoiDelivery` に委譲。2026-07-02、`node.build_handoff_payload` 呼び出しに置き換え（203→200） |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 189 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 69 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 9 | 🟢 P5-2 |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 592 | 🟡 許容範囲（Raft 実装の核） |
| `crates/dawn-sector-node/src/runtime.rs` | 256 | 🟢 production Node の jump fallback / tick stepping / replication publish 呼び出し / Redirect / AoI delivery を集約。M-7（Issue #56）で `collect_player_commands` の13分岐 match を `node.apply_client_command` 1呼び出しに置き換え（317→274）。2026-07-02、jump proposal の Raft 提案（in-range / auto-jump 双方）を `dawn_sector::transit::propose_jump` / `propose_auto_jump` に置き換え、`serve/cluster.rs`・`serve/runtime.rs` と重複していた `TransitOp::Request` 組み立てを解消（274→256）。AoI delivery 本体は `dawn_sector::aoi::AoiDelivery` へ、replication は `dawn_replication::OutboundLogPublisher` へ、command dispatch と jump proposal は `dawn_sector` 側へ移動済みで、本ファイルは orchestration のみ |
| `crates/dawn-sector-node/src/client_admission.rs` | 236 | 🟢 **新規記録**（2026-06-29・PR #41「deepen client admission flow」。これまで本表に未記載だった）。`main.rs` から WebSocket accept / Hello 読み取り / fresh-vs-resume 判定 / Welcome・InitialState 完了までの client admission state machine を集約。`main.rs` はプロセス配線、本ファイルはハンドシェイク状態機械、と責務分離。2026-07-01、`advance_handshakes`/`select_handshake_identity` を `<S: EventStore>` でジェネリック化。2026-07-02、InitialState/PlayerFitting 組み立てを `dawn_sector::node::SimulationNode::build_handoff_payload`（PR #59）呼び出しに置き換え（235→236） |
| `crates/dawn-sector-node/src/main.rs` | 341 | 🟢 8D-4 本番バイナリ。config / TCP transport / accept channel / data loading の配線に縮小。2026-07-01、永続化配線（`build_node` がスナップショット有無で新規/復元を分岐、`CheckpointScheduler` をtickループに配線）で 267→341 |
| `crates/dawn-core/src/events.rs` | 603 | 🟢 ADR-0032 `ShipFitted.inventory`・ADR-0033 `RepairApplied`/`RepairLayer`・ADR-0035 `ModuleActivated.target_ship_id`/`ModuleDeactivated.forced_reason`/`ModuleDeactivationReason` 追加 |
| `crates/dawn-ecs/src/systems/combat.rs` | 580 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 479 | 🟢 ADR-0033 `repair_cycles_started` 収集を並置。ADR-0035 で強制 OFF に `forced_reason: CapacitorExhausted` を付与。PR #63 で flat-index 境界計算と 4-way chain を `FittingComp::slot_at_flat_mut`/`iter_slots` に置換（504→479） |
| `crates/dawn-consensus/src/actor.rs` | 465 | 🟢 8D-5 実機検証で使う Raft role-transition ログ（`eprintln!`）を保持 |
| `crates/dawn-event-store/src/file.rs` | 463 | 🟢 |
| `crates/dawn-ecs/src/systems/movement.rs` | 414 | 🟢 |
| `crates/dawn-ecs/src/systems/lock.rs` | 374 | 🟢 |
| `crates/dawn-core/src/commands.rs` | 401 | 🟢 Command enum 群（継続的に variant 追加）。M-7（Issue #56）で `ClientCommand` enum を `dawn-actor` から移動（359→395）。ADR-0035 で `ActivateModuleCommand.target_ship_id` 追加（395→401） |
| `crates/dawn-consensus/src/rpc.rs` | 371 | 🟢 343→371。Raft RPC 型定義 |
| `crates/dawn-consensus/src/tcp_transport.rs` | 351 | 🟢 337→351。8D-3 TcpRaftTransport |
| `crates/dawn-ecs/src/systems/fitting.rs` | 326 | 🟢 PR #63 で `apply_fitting()` の4-way chain を `FittingComp::iter_slots()` に置換 |
| `crates/dawn-replication/src/tcp.rs` | 287 | 🟢 283→287。8D-2c |
| `crates/dawn-ecs/src/components/movement.rs` | 284 | 🟢 |
| `crates/dawn-ecs/src/world.rs` | 285 | 🟢 270→285。クエリヘルパー |
| `crates/dawn-sector-node/src/data_loader.rs` | 278 | 🟢 178→278（+100）。8D-4/8D-5 のテスト追加が主因。module/ship type TOML ローダー |
| `crates/dawn-ecs/src/components/fitting.rs` | 370 | 🟢 ADR-0035 `FittedSlot.target_ship_id` 追加。PR #63 で `slot_at_flat`/`slot_at_flat_mut`/`iter_slots`/`iter_slots_mut` を新設し、3系統に分散していた flat-index 境界計算と6+箇所の4-way chain の唯一の所有者になった |
| `crates/dawn-ecs/src/components/combat.rs` | 264 | 🟢 |
| `crates/dawn-replication/src/anti_entropy.rs` | 215 | 🟢 211→215。8D-2b |
| `crates/dawn-ecs/src/systems/repair.rs` | 212 | 🟢 ADR-0033 新設（Step 6.5 Repair System・RepairApplied 発行 + テスト） |
| `crates/dawn-replication/src/replica.rs` | 224 | 🟢 M-5（ReplicaSet・複製ログ消費側） |
| `crates/dawn-replication/src/bus.rs` | 236 | 🟢 188→236（+48）。8D-2a。テスト追加が主因 |
| `crates/dawn-core/src/navigation.rs` | 196 | 🟢 184→196。ナビゲーション型定義 |
| `crates/dawn-replication/src/snapshot.rs` | 174 | 🟢 8D-2d SnapshotTransfer（ジェネリック / 256 MiB cap） |
| `crates/dawn-core/src/ship_type.rs` | 177 | 🟢 |
| `crates/dawn-replication/src/outbound.rs` | 141 | 🟢 sender-side `OutboundLogPublisher`。append-log cursor と `LogBatch` suffix 構築を保持 |
| `crates/dawn-replication/src/lib.rs` | 84 | 🟢 8D-2a/2b/2c/2d public API |
| `crates/dawn-ecs/src/components/inventory.rs` | 69 | 🟢 ADR-0032 新設（InventoryComp） |
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

#### ~~M-7~~（新規・2026-07-01・**解消済み** 2026-07-01）: Player Command Dispatch のルーティングが `dawn-sector` の外に漏れている

`runtime.rs::collect_player_commands`（13分岐の match。各分岐は「所有権チェック→
`apply_*_command_owned` 呼び出し」のみでドメイン知識を持たない）と、
`protocol.rs::parse_client_command` のパース分岐が同じ13種類の Command 構造を
ミラーしている。Command を1種追加するたびに、`dawn-core`（enum 追加）・
`protocol.rs`（パース追加）・`runtime.rs`（dispatch 追加）・`node/*.rs`（実装追加）の
4箇所を同じ順で触る。

**根本原因**: dispatch の「ルーティング」と「各 Command の検証・実行」が
`dawn-sector`（実装側）と `dawn-sector-node`/`dawn-actor`（ルーティング側）に
分かれており、ルーティング層がドメイン知識を持たない薄いまま外側に置かれている。

**判断: 解消済み（2026-07-01）。** `ClientCommand` を `dawn-core` へ移動し（DAG ブロッカー解消）、
`SimulationNode::apply_client_command(player_id, cmd, lock_commands) -> Option<ClientCommandFollowup>` を
`dawn-sector::node::commands` に新設。`dawn-sector-node/runtime.rs` の13分岐 match と
`dawn-simulation/serve/mod.rs` の `apply_common_command` を両方この1呼び出しに置き換え。
`ClientCommandFollowup::Jump` / `RefreshFitting` で caller への戻り値を型安全に返す。
Issue #56 参照。詳細は「完了済み」表の M-7 行を参照。

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

#### Steering-mode 排他制御の非対称性を是正（2026-07-01・解消済み）

`/improve-codebase-architecture` で「Orbit/KeepAtRange/Approach/Warp の5ハンドラが
同じ排他制御スカフォールドを重複している」と指摘されたが、実装を確認したところ
前提は不正確だった: `validate_maneuver_target` は元々 Orbit/KeepAtRange でのみ共有され、
Warp はそもそも `clear_steering_modes` に参加していなかった。詳細に検証した結果、
**スタイル上の重複ではなく実害のある非対称性が3件**見つかったため、issue化せず
このまま直接修正した:

- `apply_warp_command` が `clear_steering_modes` を呼んでいなかった。Orbit中の
  Ship が Warp を始めても `OrbitComp` が残り続け、tick順序
  （`process_orbit` → `process_warp`、後者が `ThrustComp` を上書き）に
  依存して見かけ上だけ正しく動いていた。Warp完了後に古い `OrbitComp` が
  残り、操船意図が意図せず復帰し得る状態だった
- `apply_approach_command` に `is_warping` ガードが無かった（Orbit/KeepAtRangeには
  ある）。committed Warping 中の Ship に Approach を送ると拒否されず
  `ApproachComp` が付与されてしまっていた
- `is_warping`（committed フェーズのみ true）を Orbit/KeepAtRange/Approach の
  Warp優先チェックに使っていたため、**Aligning フェーズ中は素通りしていた**。
  `clear_steering_modes` のコメントは「warp はこれらのコマンドを検証ガードで
  完全に拒否する」と書いていたが実態と食い違っていた。`has_active_warp`
  （フェーズ問わず `WarpComp` の有無のみ判定）を新設し、Move/Stop だけが
  Aligning Warp を明示的にキャンセルできる特例として残し、他3コマンドは
  Aligning も含めて拒否するよう統一した

上記を修正したうえで、`apply_orbit_command`/`apply_keep_at_range_command`/
`apply_approach_command` の共通スカフォールド（entity解決・transit/Warp優先チェック・
的中判定・距離デフォルト・`clear_steering_modes`）を `begin_maneuver` ヘルパー
（`orbit.rs`、`pub(super)`）へ完全に集約した（純粋な移動・挙動不変。Approach は
distance に `None` を渡し戻り値の距離を無視するだけ）。結果として、元のレポートが
提案していた「5ハンドラの重複整理」は、3つの実バグ修正の副産物として達成された
（Move/Warp自体は意味的に非対称であるべきなので対象外のまま）。

回帰テスト5本追加（`starting_a_warp_clears_an_active_orbit` /
`approach_command_is_rejected_while_warping` /
`approach_command_is_rejected_while_aligning_to_warp` /
`orbit_command_is_rejected_while_aligning_to_warp` /
`keep_at_range_command_is_rejected_while_aligning_to_warp`）。
`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過。

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

| 作業 | 完了日 | 内容 |
|---|---|---|
| P2-1 node.rs サブモジュール化 | 2026-06-19 | commands / navigation / serialization / sector_map / ship_registry に分割 |
| P2-2 main.rs 分割 | 2026-06-19 | 63行に縮小。実装は serve.rs / bench.rs / cluster.rs へ |
| P2-3 ws JSON 分離 | 2026-06-19 | serialization.rs + protocol.rs に分離 |
| P3-1 SectorMap / ShipRegistry 抽出 | 2026-06-19 | SimulationNode フィールド 17→12 |
| P3-2 persistence/ サブモジュール | 2026-06-19 | snapshot.rs + checkpoint.rs を persistence/ 下に統合 |
| ADR-0026 dawn-sector 新設 | 2026-06-19 | ゲームロジックを dawn-simulation から完全分離 |
| P4-1 tick.rs 抽出 | 2026-06-19 | tick() / tick_with_lock_commands() を node/tick.rs へ（91行）|
| P4-2 spawner_logic.rs 抽出 | 2026-06-19 | spawn/bot メソッド群を node/spawner_logic.rs へ（394行）。node/mod.rs 2,868→2,396行 |
| P4-3 `_owned` 統合 | — | スキップ: `_owned` は3行ラッパーでロジック重複ゼロ。統合コストが効果を上回る |
| P5-1 serve.rs 分割 | 2026-06-19 | serve/{mod,single,cluster}.rs の3ファイルに分割（899行 → 382/177/241）|
| P5-2 data_loader.rs 分割 | 2026-06-19 | data_loader/{mod,ship_types,modules,star_map}.rs に分割（479行 → 12/174/190/98）|
| P6-1 `SimWorld` クエリヘルパー追加 | 2026-06-19 | `find_entity` / `query` / `get` / `get_mut` を追加。combat/capacitor/lock/fitting の `inner()` 脱出を削減（L-2 解消）|
| P7-pre node補助責務抽出 | 2026-06-19 | `tackle.rs` / `snapshot_io.rs` / `apply_event.rs` を分離。node/mod.rs は 1,545行へ縮小 |
| P7-1 Transit flow 境界整理 | 2026-06-19 | `node/transit_flow.rs` を新設。`propose_transit` / `export_transit` / `import_transit` / jump event 追記と対応テストを移動 |
| 8D-2a dawn-replication 基盤 | 2026-06-19 | `InMemoryReplicationBus` / `ReplicationTransport` を dawn-replication へ移動 |
| 8D-2b AntiEntropy | 2026-06-19 | log index gap 検出・重複/overlap 判定・`iter_from` suffix 応答 |
| 8D-2c TcpReplicationTransport | 2026-06-19 | 4-byte length prefix + postcard / LAN plaintext TCP transport |
| 8D-2d SnapshotTransfer | 2026-06-19 | `Serialize+DeserializeOwned` ジェネリック / 256 MiB cap |
| 8D-3 TcpRaftTransport | 2026-06-19 | per-peer 自動再接続 / accept ループ / postcard framing |
| 8D-4 dawn-sector-node | 2026-06-19 | 本番バイナリ（TOML 静的 config / 3 ノードクラスタ / Jump Redirect）|
| navigation.rs / galaxy.rs リネーム | 2026-06-19 | `star_system.rs` → `navigation.rs`（dawn-core）、`star_map.rs`/`StarMap` → `galaxy.rs`/`Galaxy`（dawn-sector）。L-3 解消 |
| P7-2 jump/warp validation 移動 | 2026-06-20 | `can_propose_jump` / `can_propose_warp` を `node/mod.rs` → `node/navigation.rs` へ移動。mod.rs 514行に縮小。Phase 7 完了 |
| AoI テストを serialization.rs へ移動 | 2026-06-20 | `ships_visible_to` / `aoi_enter_json` のテストを実装と同じファイルへ。L-1 解消 |
| P9-2 CelestialBodyDef sector 帰属 | 2026-06-20 | `CelestialBodyDef.sector` を追加し、`Galaxy::bodies_in_sector` の ID 割り当て近似を削除 |
| 8D-5 観測ログ仕込み | 2026-06-20 | Raft role 遷移 / TCP 再接続 / tick オーバーランを stderr 出力（実機検証で症状を切り分けるため）。`docs/process/8d5-hardware-notes.md` 追加・localhost 3 プロセス検証済み |
| 8D-5 Raspberry Pi 実機検証 | 2026-07-01 | 物理 Pi Zero W x3 で reachability / tick-sla / failover の3項目すべて PASS。`scripts/verify-pi-cluster.sh` を新設し合否基準を自動化。実行中に発見した `deploy-pi-cluster.sh` の `set -e` 早期終了バグも修正。詳細は `docs/process/8d5-hardware-notes.md` |
| M-5 replication 消費側 | 2026-06-20 | `dawn-replication::ReplicaSet` 新設。受信 `LogBatch` を peer セクターごとに gap 検出・冪等・順序保持で複製ログに取り込む（ライブ world 適用 / failover は範囲外）|
| M-4 WS 境界の集約 | 2026-06-20 | `ws_server` / `protocol` を `dawn-actor` へ移動し dawn-simulation / dawn-sector-node の手動コピーを解消（506行削除）。`bind` を `ToSocketAddrs` ジェネリック化・不要依存を除去 |
| R-1 navigation.rs 分割 | 2026-06-23 | `node/navigation.rs`（ADR-0029 で 1092行に肥大）を `node/warp.rs`（769行）/ `node/approach.rs`（306行）/ `node/navigation.rs`（62行・バリデーションのみ）へ3分割。`mod warp; mod approach;` 追加 + impl ブロック移設の純粋移動（公開 API・挙動不変）。`cargo test --workspace` 全件ゼロエラー（warp 21件 + approach 10件を新パスで確認） |
| runtime tick pipeline collapse | 2026-06-28 | `transit::run_runtime_tick` / `RuntimeTickOutput` と `serve/runtime.rs` で actor / clustered serve の tick ordering を共有。replication-before-raft ordering と transient drain を一箇所へ集約 |
| AoI delivery deepening | 2026-06-29 | `serve/aoi_delivery.rs` の `AoiDelivery` に visible-set memory / Enter-Leave / event filtering / warp `PositionSnap` delivery を集約。single/cluster serve loop から AoI frame の内部知識を除去 |
| Sector Node runtime deepening | 2026-06-29 | `dawn-sector-node/src/runtime.rs` の `SectorNodeRuntime` に command dispatch / jump fallback / tick stepping / replication publish orchestration / Redirect / AoI delivery を集約。`main.rs` は config・TCP transport・accept channel 配線中心に縮小 |
| production outbound replication publisher deepening | 2026-06-30 | `SectorNodeRuntime` から append-log cursor 管理と `LogBatch` 構築を除去し、`dawn_replication::OutboundLogPublisher` に集約。runtime は frame 後に `publish_new_events(sector_id, node.event_store())` を呼ぶだけになり、sender-side replication の locality が `dawn-replication` に揃った |
| AoI delivery を `dawn-sector` へ集約（M-6 の AoI 重複を解消） | 2026-06-29 | `dawn-simulation::serve::aoi_delivery::AoiDelivery` と `dawn-sector-node::runtime::deliver_aoi_frame` の同型実装を `dawn_sector::aoi::AoiDelivery`（`deliver_frame` + `AoiSink` trait + `Observer`）へ統合。送信先は `dawn-actor::ws_server::PlayerSession` を直接持てない（dawn-sector は dawn-actor に非依存）ため `AoiSink` trait で抽象化し、各バイナリ側にローカルな `SessionSink` ラッパー adapter（orphan rule 回避）を置く。Redirect 判定・セッション retain はそれぞれの呼び出し側に残す。`FakeSink` を使った enter/leave delta・destroyed-ship 抑制・warp snap のユニットテストを3本追加（移動前は AoI delivery のユニットテストが存在しなかった）。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| Client admission deepening（記録漏れ・今回追記） | 2026-06-29 | `dawn-sector-node/src/client_admission.rs` を新設（PR #41）。WebSocket accept / Hello 読み取り / fresh-vs-resume 判定 / Welcome・InitialState 完了までを `ClientAdmission` state machine に集約し、`main.rs` から分離。当時このレビュー文書への記録が漏れていたため、2026-07-01 の再計測で追加 |
| Sector Transit プロトコルを公開面 5→2 に集約 | 2026-06-29 | `node/transit_flow.rs` の `propose_transit`/`export_transit`/`import_transit`/`append_jump_events` を `pub(super)` に格下げし、新設の `prepare_transit_commit`（Request 側：Gate-lookup・`entry_pos`/`entry_pos_abs` 算出・export を集約）と `handle_transit_commit`（Commit 側：import + `JumpGateUsed`/`StarSystemChanged` 追記の条件分岐を集約）の2メソッドへ統合。`transit.rs` の `apply_committed_raft_entries` オーケストレーターはこの2メソッドを呼ぶだけになり、Gate の往復先探索ロジックを二重に持たなくなった（#38 のバグ修正直後の整理）。新規ユニットテスト1本（`the_consolidated_request_commit_pair_reproduces_the_same_arrival`）で集約後の経路が既存の低レベルプリミティブと同じ着地点を再現することを確認。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| `dawn-sector-node` への永続化配線 | 2026-07-01 | `/improve-codebase-architecture` で「`EventStore::append` がinfallibleと嘘をついている」と指摘されたのを調査する過程で、より大きな問題を発見: `dawn-sector-node`（本番バイナリ）は `SimulationNode::new`（デフォルト `InMemoryEventStore`）で動いており、`FileEventStore`/`checkpoint()`/`CheckpointScheduler`/`restore_from`（Phase 3 実装・テスト済み）は本番に一切配線されていなかった（`maybe_checkpoint` の呼び出しは `dawn-simulation/src/bench.rs` のみ）。`NodeConfig` に永続化パス4フィールドを追加し、`build_node` でスナップショットの有無により新規/復元を分岐（`StateSnapshot::load` が `NotFound` なら新規、それ以外のエラーなら panic——サイレントなデータ損失を避ける）。復元時は `spawn_npcs` を呼ばない（NPC重複生成防止、`is_fresh` フラグで判定）。tickループに `CheckpointScheduler::maybe_checkpoint` を配線し、チェックポイント失敗はログのみで継続（ホットログへのappendは別経路で動き続ける）。`SectorNodeRuntime`/`ClientAdmission`/`AoiDelivery::deliver_frame` を `<S: EventStore>` でジェネリック化し `SimulationNode<FileEventStore>` に対応。実機での起動→kill→再起動でtick/log_indexが継続し、NPCが重複生成されないことを手動確認済み。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| M-7 ClientCommand dispatch 統一（Issue #56） | 2026-07-01 | `ClientCommand` enum を `dawn-actor` → `dawn-core` へ移動（`dawn-actor` は `pub use dawn_core::ClientCommand` で後方互換維持）。`dawn-sector::node::SimulationNode::apply_client_command(player_id, cmd, lock_commands) -> Option<ClientCommandFollowup>` を新設し、`dawn-sector-node/src/runtime.rs` の13分岐 match と `dawn-simulation/src/serve/` の `apply_common_command`（両バイナリの重複）を1呼び出しに統一。`ClientCommandFollowup` で Jump と RefreshFitting を呼び出し元に返す。`cargo test --workspace` 全件通過 |
| Client handshake payload の集約（PR #59） | 2026-07-02 | `single.rs`（dawn-simulation）と `client_admission.rs`（dawn-sector-node）が、identity 選択後の InitialState/PlayerFitting JSON 組み立てを同一コードで重複していた（identity 選択自体は resume 対応の有無で別物なので統一対象外）。`dawn_sector::node::SimulationNode::build_handoff_payload(ship_id, aoi_cell_size) -> HandoffPayload` を新設し両呼び出し元から呼ぶ形に統一。ユニットテスト1件追加。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |
| Jump proposal orchestration 統一 | 2026-07-02 | `apply_jump_with_fallback` の outcome（in-range → Raft へ `TransitOp::Request` 提案）と auto-jump 提案パスが、`dawn-sector-node/src/runtime.rs` と `dawn-simulation/src/serve/{cluster,runtime}.rs` の3箇所に重複していた。`dawn_sector::transit::propose_jump` / `propose_auto_jump` を新設し、fallback chain の結果を Raft 提案へ橋渡しする部分を1箇所に集約。呼び出し元は返り値の `JumpOutcome`/`Option<SectorId>` を自分のログ整形にだけ使う。`SimulationNode::set_spawn_anchor_abs` を `pub(super)`→`pub(crate)`（`#[cfg(test)]` のまま）に広げ、新規テスト2件を追加。`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過 |

> Phase 2〜7 の構造リファクタ、Phase 8D の TCP 分散配線、M-4/M-5 の重複/機能ギャップ解消、
> R-1（navigation.rs 分割）、runtime tick pipeline collapse、AoI delivery deepening、
> Sector Node runtime deepening、production outbound replication publisher deepening、
> AoI delivery の dawn-sector への集約、
> Sector Transit プロトコルの公開面集約まですべて完了。

### リファクタロードマップ（2026-06-23 追加・ADR-0029 後の再計測で起票）

機能追加（ADR-0029）で再び閾値を超えたファイルの分割を、過去の P7 系（`transit_flow.rs` /
`tackle.rs` / `snapshot_io.rs` を `node/mod.rs` から切り出した）と同じ「責務ごとに sibling
モジュールへ抽出、テストも実装と同じファイルへ」方式で行う。挙動は変えない（純粋な移動）。

#### ~~R-1~~: `node/navigation.rs` 1092 行の分割（完了・上記「完了済み」参照）

#### R-2（一部着手済み）: クライアント `main.gd` 1161 行

ADR-0029 以降に増加した `main.gd` は、2026-06-30 に `WorldSession` を抽出して
1241→1161 に縮小。InitialState / AoI / HP / lock / tick-cap の live world state は
`client/scripts/world_session.gd` へ移動済み。残りは scene lifecycle / input / node generation /
HUD adapter のオーケストレーション層。さらなる分割は `.tscn` 化コンポーネントへの
シーン参照切れリスクが上回るため引き続き保留（client レビューの「採らない方針」と同根。
C-3 はフェイルファストガードで解消済み・2026-06-23 だが、これはこの判断とは独立——
更なる分割を妨げるのはシーン参照切れリスクそのもので、C-3 の有無は前提条件ではなかった）。

#### R-3（低優先・トリガー保留）: `node/` 系ファイルの再肥大（ADR-0031/0032/0033 後）

2026-07-01 の再計測（2回目）で、`warp.rs`（1094、impl 534）/ `spawner_logic.rs`（881、impl 603）/
`orbit.rs`（854、impl 317）/ `mod.rs`（801、impl 612）が総行数で閾値帯に残っている。
R-1（navigation.rs 分割）後に積まれた Orbit/KeepAtRange（ADR-0031）・Inventory（ADR-0032）・
Repair（ADR-0033）の累積に加え、テストの増加が総行数を押し上げている。
**4ファイルとも impl（テスト除く）は700行未満** で、下記トリガーは未発火。
Sector Node runtime deepening は production binary 側の浅さを解消したが、`dawn-sector/src/node/`
内部の domain module サイズには影響しないため、R-3 は引き続き観察対象として残す。

**根本原因**: 機能追加のたびに `node/` 直下へ impl + テストが積まれる構造。これ自体は
P7 系で確立した「責務ごとに sibling モジュールへ抽出」方式の想定内の蓄積であり、
設計の破綻ではない。

**判断: 保留（トリガー付き）。** 総行数はまだ大きいが、現時点では単一責務を保っている。
**今分割すると純粋移動の差分だけが増え、得が薄い。**

再評価トリガー（いずれかで着手）:
- いずれかの **impl 部分**（テスト除く）が ~700 行を超えたとき。
  - `warp.rs` → `process_warp` / Hermite warp 幾何 / コマンド・drain に3分割。
  - `spawner_logic.rs` → spawn / bot AI / inventory seed の責務で分割。
  - `orbit.rs` → Orbit / KeepAtRange の共有幾何と command application を分離。
  - `mod.rs` → フィールド定義と補助 impl の分離。
- または `node/` のファイル総数が増えて「どこに何があるか」の見通しが実際に悪化したとき。

### 未完了・保留

上記リファクタロードマップ以外で残るのは以下。いずれも現時点では新しい module / crate を
増やすより保留・観察の方が費用対効果が高い、と判断した項目。

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| R-2 client `main.gd` 分割 | 品質・一部着手済み | `WorldSession` 抽出で live world state を移動し、`main.gd` は 1185 行。残る scene lifecycle / input / node generation / HUD adapter は `.tscn` 化コンポーネントへのシーン参照切れリスクが上回るため保留（C-3 とは無関係） |
| R-3 `node/` 系再肥大（warp/spawner/mod/orbit） | 品質・保留 | 総行数は閾値帯だが impl は概ね 700 未満・増分はテスト主体。impl が 700 超でファイル別に分割（トリガー付き・上記 R-3） |
| 8D-5 Raspberry Pi 実機検証 | 完了 → 「完了済み」参照 | 2026-07-01、reachability/tick-sla/failover 3項目とも PASS。詳細は `docs/process/8d5-hardware-notes.md` |
| M-3 `SectorSimulatorActor` 密結合 | 品質・保留 | 本番パス外（in-process テスト/ベンチ専用）。P9-1 撤回。優先度低 |
<<<<<<< HEAD
| M-6 アプリ層 adapter 重複（`data_loader` / `spawn_npcs`） | 許容重複（縮小） | AoI / production runtime / Command dispatch は deep module 化済み（M-7 解消で Command dispatch 項目を削除）。残る data_loader / NPC spawn は低頻度 glue として許容。再評価トリガー付き |
=======
| M-6 アプリ層 adapter 重複（command dispatch / `data_loader` / `spawn_npcs`） | 許容重複 | AoI / production runtime は deep module 化済み。Player Command Dispatch は新 crate 化を検討したが浅い seam と判断し許容。再評価トリガー付き |
>>>>>>> origin/main
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

Phase 9 時点では総合 **A−** で決着とし、M-3（本番パス外）・M-6（許容）は「大きい共有 runtime crate を作らない」と
判断した。その後 ADR-0029（真スケール座標）の機能追加で `node/navigation.rs` が閾値を
超えて再肥大し、構造リファクタが一時再燃したが、R-1（navigation.rs 分割・2026-06-23）で
解消済み。さらに `dawn-simulation` 側 AoI delivery と `dawn-sector-node` 側 runtime は
deep module 化済み（上記「完了済み」参照）。Player Command Dispatch は新 crate 化を見送った。
A− を維持。8D-5 実機検証も 2026-07-01 に完了。同日 M-7（Player Command Dispatch
統一）も完了し、残る前進先は戦闘の深み（ADR-0016 §5）といった機能側で、
R-2（client `main.gd`）は保留のまま
（client レビューの「採らない方針」参照。トリガーは C-3 ではなくシーン参照切れリスクそのもの）。

| 項目 | 状態 |
|---|---|
| P9-1（M-3 解消） | 撤回（下記） |
| P9-2（`CelestialBodyDef.sector`） | 完了 → 「完了済み」へ移動 |

#### ~~P9-1: M-3 解消~~（撤回・保留）

当初は「`SectorSimulatorActor` / `SimulationNode` 境界をコマンド/応答 enum で疎結合化し、
8D-5 実機検証の完了後に着手」としていたが、前提が崩れたため撤回する。

`SectorSimulatorActor` は本番パス外（インプロセス・テスト/ベンチ専用）で、本番バイナリ
`dawn-sector-node` は 8D-4 で独自 main ループに移行しこの Actor を使わない（M-3 参照）。
8D-5 はこの境界を経由しないため「実機検証後に着手」という条件は無意味。優先度を下げて保留する。

残る品質観点は **低頻度 glue 重複**（M-6・許容）と **密結合**（M-3・本番パス外で低優先）のみで、
いずれも本番品質には直結しない（「未完了・保留」参照）。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（ClientConnection 境界）— replication 責務は `dawn-replication` へ移動済み
