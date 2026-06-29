---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture/architecture.md
date     : 2026-06-29（Sector Node runtime deepening 後の再計測。dawn-sector-node/main.rs 589→308、runtime.rs 354 追加。R-3 は warp.rs 985 / spawner_logic.rs 804 / orbit.rs 723 / mod.rs 724）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B+**（2026-06-29 維持。PR #34 で `dawn-simulation` 側 AoI delivery policy が `serve/aoi_delivery.rs` に集約され、続いて `dawn-sector-node` 側 production frame orchestration が `runtime.rs` に集約された。大きいファイルは `warp.rs` 985 / `spawner_logic.rs` 804 / `orbit.rs` 723 / `node/mod.rs` 724 に残るが、いずれも単一責務の観察対象として R-3 保留）

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector / dawn-replication が分離済み（ADR-0026/0027）。新規 `node/orbit.rs`・`node/inventory.rs`・`systems/repair.rs` も既存責務分割に沿う |
| ファイルサイズ | B+ | 2026-06-29 再計測で **4ファイルが総行数で閾値帯**: `warp.rs` 985・`spawner_logic.rs` 804・`orbit.rs` 723・`node/mod.rs` 724。`dawn-simulation/serve` は `runtime.rs` / `aoi_delivery.rs` へ、`dawn-sector-node` は `runtime.rs` へ分割され、各起動 loop は小さく維持。実害はまだ無いが観察対象が複数 → R-3 に集約しトリガー保留 |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。`InventoryComp`（ADR-0032）・`RepairLayer`/`RepairApplied`（ADR-0033）も既存型設計に整合 |
| 重複 | A− | WS 境界は dawn-actor へ集約（M-4 解消）。PR #34 で dawn-simulation 側 AoI delivery、続いて sector-node 側 production runtime は deep module 化。残る両バイナリ間グルー重複（M-6）は data loading / NPC spawn など低頻度 glue として許容判断 |
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。`TransitOp::Commit` は ADR-0032 で `Box<ShipSnapshot>` 化しサイズ非対称を解消済み |
| AI開発由来 | A− | 命名汚染なし。残る `SectorSimulatorActor` の密結合（M-3）は本番パス外の in-process 専用で実害小 |

---

## ファイルサイズ一覧（2026-06-29 時点）

> 2026-06-19 の前回計測から、ADR-0029（真スケール座標）の実装でワープ遷移・座標変換・
> シリアライズ周りが増加。M-4（WS 境界集約）で `protocol.rs` / `ws_server.rs` は両バイナリ
> から削除され `dawn-actor` に集約済み（下の dawn-actor 表）。

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/warp.rs` | 985 | 🟡 R-1 新設（2026-06-23）。warp 幾何の単一責務だが総行数が閾値を超過。R-3 でトリガー保留（impl が 700 を超えたら process_warp / warp 幾何 / コマンドへ分割） |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 804 | 🟡 P4-2 + P7-1 + ADR-0029 + ADR-0032（inventory seeding）。spawn / bot AI / inventory seed が同居。R-3 で観察（impl 700 超 or 責務分岐で分割） |
| `crates/dawn-sector/src/node/orbit.rs` | 723 | 🟡 ADR-0031 新設。Orbit / Keep at Range の操船一式。単一責務で許容、R-3 で観察 |
| `crates/dawn-sector/src/node/mod.rs` | 724 | 🟡 P7-2 後 + ADR-0031/0032 のフィールド・定数追加。R-3 で観察 |
| `crates/dawn-sector/src/node/transit_flow.rs` | 558 | 🟢 P7-1 + ADR-0032（inventory 転送）。impl は小、増分はテスト |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 524 | 🟢 P7-pre + ADR-0032（inventory 永続化）。ほぼテスト |
| `crates/dawn-sector/src/node/inventory.rs` | 422 | 🟢 ADR-0032 新設。fit/unfit_module_owned + seed + テスト |
| `crates/dawn-sector/src/node/commands.rs` | 440 | 🟢 P7-1 + ADR-0032（fit 時 inventory 同梱） |
| `crates/dawn-sector/src/node/serialization.rs` | 400 | 🟢 ADR-0029 + ADR-0032（inventory / slot_capacity を PlayerFitting に追加） |
| `crates/dawn-sector/src/galaxy.rs` | 360 | 🟢 ADR-0029 AU→units 変換・ゲート AU 化 |
| `crates/dawn-sector/src/node/apply_event.rs` | 339 | 🟢 P7-pre + ADR-0032（ShipFitted/ShipSpawned で inventory 復元） |
| `crates/dawn-sector/src/node/tackle.rs` | 324 | 🟢 P7-pre |
| `crates/dawn-sector/src/aoi.rs` | 292 | 🟢 |
| `crates/dawn-sector/src/anchor.rs` | 292 | 🟢 ADR-0029 新設（AnchorTable・静的 f64 アンカー絶対座標） |
| `crates/dawn-sector/src/transit.rs` | 278 | 🟢 PR #30 で `run_runtime_tick` / `RuntimeTickOutput` を追加。actor / clustered serve の tick pipeline 共有入口 |
| `crates/dawn-sector/src/modules.rs` | 211 | 🟢 ADR-0033 で Active 修理モジュール定義を追加 |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 173 | 🟢 ADR-0032 で `ShipSnapshot.inventory` 追加 |
| `crates/dawn-sector/src/dilation.rs` | 160 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 156 | 🟢 |
| `crates/dawn-sector/src/node/approach.rs` | 480 | 🟢 R-1 新設（2026-06-23）。approach 系 + ADR-0031 で clear_steering_modes 連携 |
| `crates/dawn-sector/src/node/tick.rs` | ~160 | 🟢 P4-1 + ADR-0031 Step 2.55/2.56 + ADR-0033 Step 6.5 配線 |
| `crates/dawn-sector/src/spawner.rs` | 127 | 🟢 |
| `crates/dawn-sector/src/ship_types.rs` | 82 | 🟢 |
| `crates/dawn-sector/src/node/navigation.rs` | ~75 | 🟢 R-1 後。`can_propose_jump` / `can_propose_warp` + ADR-0017 dead-zone テスト |
| `crates/dawn-sector/src/node/ship_registry.rs` | 33 | 🟢 P3-1 |
| `crates/dawn-sector/src/node/sector_map.rs` | 25 | 🟢 P3-1 |

### dawn-actor（クライアント転送境界・M-4 集約先）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-actor/src/protocol.rs` | 553 | 🟢 M-4 集約（DomainEvent↔JSON↔ClientCommand）。335→553。ADR-0031（Orbit/KeepAtRange）・ADR-0032（Fit/Unfit）のパース追加。18+ variant・変更頻度高だが単一責務 |
| `crates/dawn-actor/src/client_connection.rs` | 297 | 🟢 ClientConnection trait + InProcess/Ws 実装。ADR-0031/0032 の ClientCommand variant 追加 |
| `crates/dawn-actor/src/ws_server.rs` | 212 | 🟢 M-4 集約（WsServer / PlayerSession）+ ADR-0032 `send_raw` |
| `crates/dawn-actor/src/lib.rs` | 29 | 🟢 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 531 | 🟢 Raft クラスター配線（in-process テスト用） |
| `crates/dawn-simulation/src/serve/mod.rs` | 437 | 🟢 P5-1 共通ヘルパー。ADR-0017 jump dead-zone fallback・ADR-0032 `CommonCommandFollowup` enum 追加。PR #34 で AoI delivery を分離 |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 413 | 🟡 M-3（本番パス外・保留）。PR #30 で tick pipeline を `transit::run_runtime_tick` に寄せた |
| `crates/dawn-simulation/src/bench.rs` | 430 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 241 | 🟢 PR #30 で tick 後処理を `serve/runtime.rs` へ移動。PR #34 後は `AoiDelivery` を持ち、入力処理と runtime 呼び出し中心 |
| `crates/dawn-simulation/src/serve/runtime.rs` | 192 | 🟢 PR #30 新設。auto-jump / ownership handoff / scoped InitialState resend を集約し、AoI delivery は `AoiDelivery` に委譲 |
| `crates/dawn-simulation/src/serve/aoi_delivery.rs` | 174 | 🟢 PR #34 新設。visible-set memory / AoiEnter・AoiLeave / event filtering / warp `PositionSnap` delivery を集約 |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 219 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 203 | 🟢 P5-1。PR #34 後は AoI delivery 詳細を `AoiDelivery` に委譲 |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 189 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 65 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 9 | 🟢 P5-2 |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 592 | 🟡 許容範囲（Raft 実装の核）。`div_ceil` clippy 修正のみ |
| `crates/dawn-sector-node/src/runtime.rs` | 354 | 🟢 2026-06-29 新設。production Node の command dispatch / jump fallback / tick stepping / outbound replication / Redirect / AoI delivery を集約 |
| `crates/dawn-sector-node/src/main.rs` | 308 | 🟢 8D-4 本番バイナリ。config / TCP transport / accept channel / data loading の配線に縮小 |
| `crates/dawn-core/src/events.rs` | 584 | 🟢 535→584。ADR-0032 `ShipFitted.inventory`・ADR-0033 `RepairApplied`/`RepairLayer` 追加 |
| `crates/dawn-ecs/src/systems/combat.rs` | 580 | 🟢 469→580（impl 329 / test 251） |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 504 | 🟢 412→504。ADR-0033 `repair_cycles_started` 収集を並置 |
| `crates/dawn-consensus/src/actor.rs` | 465 | 🟢 |
| `crates/dawn-event-store/src/file.rs` | 463 | 🟢 |
| `crates/dawn-ecs/src/systems/movement.rs` | 414 | 🟢 |
| `crates/dawn-ecs/src/systems/lock.rs` | 374 | 🟢 |
| `crates/dawn-ecs/src/systems/repair.rs` | 212 | 🟢 ADR-0033 新設（Step 6.5 Repair System・RepairApplied 発行 + テスト） |
| `crates/dawn-replication/src/replica.rs` | 224 | 🟢 M-5（ReplicaSet・複製ログ消費側） |
| `crates/dawn-ecs/src/components/inventory.rs` | 69 | 🟢 ADR-0032 新設（InventoryComp） |
| `crates/dawn-consensus/src/rpc.rs` | 343 | 🟢 Raft RPC 型定義 |
| `crates/dawn-consensus/src/tcp_transport.rs` | 337 | 🟢 8D-3 TcpRaftTransport |
| `crates/dawn-replication/src/tcp.rs` | 283 | 🟢 8D-2c |
| `crates/dawn-ecs/src/world.rs` | 270 | 🟢 P6-1 クエリヘルパー追加 |
| `crates/dawn-replication/src/anti_entropy.rs` | 211 | 🟢 8D-2b |
| `crates/dawn-replication/src/bus.rs` | 188 | 🟢 8D-2a |
| `crates/dawn-core/src/navigation.rs` | 184 | 🟢 ナビゲーション型定義（star_system.rs より改名）|
| `crates/dawn-sector-node/src/data_loader.rs` | 178 | 🟢 8D-4 module/ship type TOML ローダー |
| `crates/dawn-replication/src/snapshot.rs` | 164 | 🟢 8D-2d SnapshotTransfer（ジェネリック / 256 MiB cap） |
| `crates/dawn-replication/src/lib.rs` | 78 | 🟢 8D-2a/2b/2c/2d public API |
| `crates/dawn-sector-node/src/config.rs` | 56 | 🟢 8D-4 TOML 静的 config |

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
> 2026-06-20）、dawn-simulation 側の AoI delivery deepening（PR #34）、および
> Sector Node runtime deepening（2026-06-29）は解消済み。
> 詳細は「改善ロードマップ > 完了済み」を参照。

#### M-6（許容）: 2つの serve バイナリに残るアプリ層 adapter 重複

M-4（WS 境界）、PR #34（dawn-simulation 側 AoI delivery deepening）、および
Sector Node runtime deepening 後も、
両バイナリの「アプリケーション層」adapter/glue は一部重複している:

| 重複 | dawn-simulation | dawn-sector-node | 備考 |
|---|---|---|---|
| `data_loader`（`load_modules` / `load_ship_types` / `parse_*`） | `data_loader/*.rs`（実装 ~280行）| `data_loader.rs`（178行）| TOML ローダー |
| AoI フレーム配信 | `serve/aoi_delivery.rs`（`AoiDelivery`） | `runtime.rs`（`SectorNodeRuntime` 内の delivery policy） | **責務は同型**だが、両側とも起動 loop から deep module へ移動済み |
| `spawn_npcs` / `spawn_npc_frigates` | `serve/mod.rs:278` | `main.rs:298` | **実質同一**（~12行）|

現在の実態では、`dawn-simulation` 側は `serve/runtime.rs` と `serve/aoi_delivery.rs` によって
single/cluster の内部知識をかなり集約済みで、`dawn-sector-node` 側も `runtime.rs` によって
production process model 固有の frame orchestration を集約済みである。問題は「同じ大きな serve loop が
二重化している」ではなく、**2つの process model がそれぞれ自分の adapter を持つ**ことに縮小した。
8D-4 で `dawn-sector-node` を `dawn-simulation` の serve 経路からコピーして作った名残はあるが、
WS protocol は `dawn-actor` に、ゲームロジックは `dawn-sector` に、両 runtime の frame policy は
それぞれのローカル module に寄っており、残る重複は低頻度の glue に縮小している。

これは M-4 で `data_loader` を `dawn-actor` に置けなかった理由（I/O 禁止）と同根で、
個別ファイルの置き場問題ではなく**共有アプリ層クレートの欠如**である。ただし、現時点では
そのクレートを作るほどの深さや adapter 数には達していない。

#### 判断: 当面は許容する（新規クレートは作らない）

`dawn-server`（仮称）共有クレートを新設する案もあるが、文書全体に照らして
**過剰**と判断し採らない。理由:

- **ガイド §「新Crate追加チェック」第1項目**「既存Crateの責務分割で対応できないことを確認」を
  満たさない。両バイナリの差は process モデル（N-in-1 vs 1-per-process）と
  **transport の選択だけ**で、その transport は既に trait 抽象化済み
  （`RaftTransport` / `ReplicationTransport`）。重複はこの2バイナリ構成の副産物で、
  クレート新設は不均衡。
- **8D 最小化方針**（roadmap「巨大基盤の一括建設をしない・薄いスライス」）に逆行する。
- **前例との整合**: `dawn-proto` は「見返りが乏しい」と却下、P4-3 は `_owned` 統合を
  「統合コストが効果を上回る」とスキップ。現在残る安定したグルーの重複も同じ費用対効果で許容が妥当。
- **ドリフトの実害が小さい**: M-4 で直した `protocol`（18 variant・変更頻度高）と違い、
  `data_loader` / NPC spawn / 各 process model 固有 runtime は変更頻度が低く無言バグ化のリスクは限定的。

再評価トリガー（このいずれかが起きたら設計し直す）:
- `data_loader` / NPC spawn / 各 runtime adapter が実際にドリフトしてバグを生んだとき
- 3つ目の serve バイナリが必要になったとき
- 2バイナリの process モデル差を解消し1バイナリ化できる見込みが立ったとき
  （その場合は新規クレートではなくバイナリ統合を優先検討する）

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
| M-5 replication 消費側 | 2026-06-20 | `dawn-replication::ReplicaSet` 新設。受信 `LogBatch` を peer セクターごとに gap 検出・冪等・順序保持で複製ログに取り込む（ライブ world 適用 / failover は範囲外）|
| M-4 WS 境界の集約 | 2026-06-20 | `ws_server` / `protocol` を `dawn-actor` へ移動し dawn-simulation / dawn-sector-node の手動コピーを解消（506行削除）。`bind` を `ToSocketAddrs` ジェネリック化・不要依存を除去 |
| R-1 navigation.rs 分割 | 2026-06-23 | `node/navigation.rs`（ADR-0029 で 1092行に肥大）を `node/warp.rs`（769行）/ `node/approach.rs`（306行）/ `node/navigation.rs`（62行・バリデーションのみ）へ3分割。`mod warp; mod approach;` 追加 + impl ブロック移設の純粋移動（公開 API・挙動不変）。`cargo test --workspace` 全件ゼロエラー（warp 21件 + approach 10件を新パスで確認） |
| runtime tick pipeline collapse | 2026-06-28 | `transit::run_runtime_tick` / `RuntimeTickOutput` と `serve/runtime.rs` で actor / clustered serve の tick ordering を共有。replication-before-raft ordering と transient drain を一箇所へ集約 |
| AoI delivery deepening | 2026-06-29 | `serve/aoi_delivery.rs` の `AoiDelivery` に visible-set memory / Enter-Leave / event filtering / warp `PositionSnap` delivery を集約。single/cluster serve loop から AoI frame の内部知識を除去 |
| Sector Node runtime deepening | 2026-06-29 | `dawn-sector-node/src/runtime.rs` の `SectorNodeRuntime` に command dispatch / jump fallback / tick stepping / outbound replication / Redirect / AoI delivery を集約。`main.rs` は config・TCP transport・accept channel 配線中心に縮小 |

> Phase 2〜7 の構造リファクタ、Phase 8D の TCP 分散配線、M-4/M-5 の重複/機能ギャップ解消、
> R-1（navigation.rs 分割）、runtime tick pipeline collapse、AoI delivery deepening、
> Sector Node runtime deepening まですべて完了。

### リファクタロードマップ（2026-06-23 追加・ADR-0029 後の再計測で起票）

機能追加（ADR-0029）で再び閾値を超えたファイルの分割を、過去の P7 系（`transit_flow.rs` /
`tackle.rs` / `snapshot_io.rs` を `node/mod.rs` から切り出した）と同じ「責務ごとに sibling
モジュールへ抽出、テストも実装と同じファイルへ」方式で行う。挙動は変えない（純粋な移動）。

#### ~~R-1~~: `node/navigation.rs` 1092 行の分割（完了・上記「完了済み」参照）

#### R-2（低優先・トリガー待ち）: クライアント `main.gd` 1210 行

ADR-0029 でワープ演出・単位整形・原点リベースが加わり 1094→1210 に増加（client レビュー参照）。
ただし god object は C-1 で解消済みで、残りはオーケストレーション層。`world_space` /
`unit_format` は既に static class に分離済み。さらなる分割は `.tscn` 化コンポーネントへの
シーン参照切れリスクが上回るため保留（client レビューの「採らない方針」と同根。
C-3 はフェイルファストガードで解消済み・2026-06-23 だが、これはこの判断とは独立——
更なる分割を妨げるのはシーン参照切れリスクそのもので、C-3 の有無は前提条件ではなかった）。

#### R-3（低優先・トリガー保留）: `node/` 系ファイルの再肥大（ADR-0031/0032/0033 後）

2026-06-29 の再計測で、`warp.rs`（985）/ `spawner_logic.rs`（804）/ `orbit.rs`（723）/
`mod.rs`（724）が総行数で閾値帯に残っている。R-1（navigation.rs 分割）後に積まれた
Orbit/KeepAtRange（ADR-0031）・Inventory（ADR-0032）・Repair（ADR-0033）の累積。
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
| R-2 client `main.gd` 分割 | 品質・保留 | 1151 行だが god object 解消済み。`.tscn` 化コンポーネントへのシーン参照切れリスクが上回るため保留（C-3 とは無関係） |
| R-3 `node/` 系再肥大（warp/spawner/mod/orbit） | 品質・保留 | 総行数は閾値帯だが impl は概ね 700 未満・増分はテスト主体。impl が 700 超でファイル別に分割（トリガー付き・上記 R-3） |
| 8D-5 Raspberry Pi 実機検証 | 機能・外部依存待ち | ハードウェア未購入。観測ログ・config・localhost 検証は済み（完了済み参照）。Pi 入手後に着手 |
| M-3 `SectorSimulatorActor` 密結合 | 品質・保留 | 本番パス外（in-process テスト/ベンチ専用）。P9-1 撤回。優先度低 |
| M-6 アプリ層 adapter 重複（`data_loader` / runtime adapter / `spawn_npcs`） | 許容重複 | dawn-simulation 側 AoI と sector-node 側 production runtime は deep module 化済み。残る重複は低頻度 glue として許容。新規クレートは過剰と判断。再評価トリガー付き |

採らない方針（恒久）:

- CRDT / LWW-Register は採らない（単一所有 + append-only log gossip）
- protobuf / `dawn-proto` は採らない（wire は postcard 再利用）
- TLS / 認証は第1次 LAN 検証では扱わない

---

### Phase 8 — 物理ノード分散の配線（Phase 8D 完了）

`dawn-replication`（ADR-0021/0027・Phase 8D）は 8D-2〜8D-4 を完了済み。
残る 8D-5（Raspberry Pi 実機検証）は上の「未完了・保留」を参照。

---

### Phase 9 — 評価の総点検（決着）

Phase 9 時点では総合 **A−** で決着とし、M-3（本番パス外）・M-6（許容）は「共有クレートを作らない」と
判断した。その後 ADR-0029（真スケール座標）の機能追加で `node/navigation.rs` が閾値を
超えて再肥大し、構造リファクタが一時再燃したが、R-1（navigation.rs 分割・2026-06-23）で
解消済み。さらに `dawn-simulation` 側 AoI delivery と `dawn-sector-node` 側 runtime は
それぞれローカル deep module 化済み（上記「完了済み」参照）。A− を維持。残る前進先は引き続き **8D-5 実機検証** や
戦闘の深み（ADR-0016 §5）といった機能側で、R-2（client `main.gd`）は保留のまま
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
