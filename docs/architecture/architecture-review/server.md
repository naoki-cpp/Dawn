---
scope    : コードベース全体の保守性・設計品質レビュー — 構造評価（グレード・ファイルサイズ）
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-07-17（定期再計測 その5。PR #149のStation operation execution seamを反映。`station_operation_execution.rs`を新設し、station lifecycle/materializationから受理済み操作の副作用を集約。Rust実測値とserver-pendingを更新。server総合B+、client側は別途client.md参照）
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

**総合: B+**（2026-07-17 再計測で維持。PR #149で `station_operation_execution.rs` を新設し、
Stationのaccepted-operation副作用（速度停止・イベントappend・snapshot更新・station inventory
連携）を `station_lifecycle.rs` / `station_materialization.rs` から集約した。新モジュールは281行、
implは約152行で単一責務と直接テストを保つ。`station_materialization.rs` は692→645行へ縮小した。
一方、`commands.rs` 1642、`inventory.rs` 931、`ship_cargo.rs` 573、`warp.rs` 1088、
`dawn-market/src/order_book.rs` 1030などのwatch対象は残るが、R-3の着手トリガーである各impl約700行超は未発火。`dawn-wire`への
プロトコル移動とDAGの健全性も維持されている）

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector / dawn-replication が分離済み（ADR-0026/0027）。M-7 解消で `ClientCommand` を `dawn-core` へ移動し DAG が整理された（`dawn-sector` が `dawn-actor` 非依存のまま dispatch を保持できるようになった）。Player Command Dispatch のための新 crate は引き続き不要。2026-07-10、`dawn-client-core`（Godot非依存クライアントドメインモデル、`dawn-core`のみに依存）と `dawn-client-gdext`（GDExtensionバインディング、cdylib、他クレートから依存されない葉ノード）を新設（ADR-0039/0040）。2026-07-11、`dawn-wire`（client<->server wire schema、`dawn-core`+serde+postcardのみ依存、トランスポート/ランタイム依存なし）を新設（ADR-0041/0042）。`dawn-actor`（deserialize）と`dawn-client-gdext`（construct+serialize）の双方が同じ型を、不要な依存を持ち込まずに使える。3クレートともDAGの末端/葉ノードに追加され、既存クレートへの逆依存は発生していない |
| ファイルサイズ | B+ | 2026-07-17再計測。Station operation executionとship cargo ownershipの副作用集約は完了したが、`commands.rs` 1642・`inventory.rs` 931・`warp.rs` 1088・`order_book.rs` 1030などのwatch対象が残る。各implは約700行未満でR-3の着手トリガーは未発火。 |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。`InventoryComp`（ADR-0032）・`RepairLayer`/`RepairApplied`（ADR-0033）・`ItemId`（ADR-0034、`dawn-core/src/item.rs`）も既存型設計に整合 |
| 重複 | A− | WS 境界は dawn-actor へ集約（M-4 解消）。AoI delivery、production runtime、Command dispatch は deep module 化済み（M-7 解消で `apply_client_command` が `SimulationNode` に集約）。2026-07-08、`ItemId -> ItemRow` JSON変換の重複（`serialization.rs` 2箇所）を `item_id_to_row_json` へ集約し解消済み。2026-07-11、M-10解消: postcard encode/decodeの3箇所分散呼び出しを`dawn-wire`の`ServerMessage`/`ClientMessage::encode/decode`へ集約。残る両バイナリ間グルー重複（M-6）・Fit経路のテール重複（M-8）は許容判断のまま |
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。`TransitOp::Commit` は ADR-0032 で `Box<ShipSnapshot>` 化しサイズ非対称を解消済み |
| AI開発由来 | A− | 命名汚染なし。残る `SectorSimulatorActor` の密結合（M-3）は本番パス外の in-process 専用で実害小 |

---

## 最新ファイルサイズ一覧（2026-07-17 再計測）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-sector/src/node/commands.rs` | 1642 | 🟡 R-3 watch。command dispatch と検証を保持するが、implは約593行でトリガー未発火 |
| `crates/dawn-sector/src/node/inventory.rs` | 931 | 🟢 Fit/Unfit/Reorderの検証とFittingComp変更に専念。ship cargo ownershipを`ship_cargo.rs`へ分離 |
| `crates/dawn-sector/src/node/ship_cargo.rs` | 573 | 🟢 ship cargo ownership、Station transfer、Market bridge、初期seedを集約。直接回帰テスト付き |
| `crates/dawn-sector/src/node/warp.rs` | 1088 | 🟡 R-1系のwatch。implは約503行で、warp幾何・drain・proposalの境界を維持 |
| `crates/dawn-market/src/order_book.rs` | 1030 | 🟡 Market settlement候補。order matching / persistence / bridge境界を次回判断 |
| `crates/dawn-sector/src/node/transit_flow.rs` | 940 | 🟡 Request/Commitの責務は分離済み。implは約326行 |
| `crates/dawn-actor/src/protocol/mod.rs` | 922 | 🟡 R-5完了後の公開入口。実装本体は`dawn-wire`へ移動済みだが、統合テストを含む総量をwatch |
| `crates/dawn-sector/src/node/apply_event.rs` | 860 | 🟡 replay applyの単一責務を維持。implは約310行 |
| `crates/dawn-sector/src/node/mod.rs` | 854 | 🟡 R-4完了後の再蓄積をwatch。implは約443行 |
| `crates/dawn-sector/src/node/orbit.rs` | 836 | 🟡 Orbit / KeepAtRangeを保持。implは約294行 |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 702 | 🟡 snapshot / inventory persistenceの境界を保持。watch下限に近い |
| `crates/dawn-sector/src/node/station_materialization.rs` | 645 | 🟢 build / assemble / disassembleの検証・計画に専念。PR #149で692→645 |
| `crates/dawn-sector/src/node/station_operation_execution.rs` | 281 | 🟢 Station accepted-operationの副作用を専有。直接テストを含むdeep module（PR #149） |
| `crates/dawn-sector/src/node/station_lifecycle.rs` | 410 | 🟢 dock / undock / active ship等の検証・計画に専念 |
| `crates/dawn-sector/src/node/station_inventory.rs` | 378 | 🟢 bounded cacheの責務に限定 |
| `crates/dawn-sector/src/node/station_inventory_db.rs` | 328 | 🟢 SQLite永続化の責務に限定 |
| `crates/dawn-sector/src/node/player_loadout_projection.rs` | 506 | 🟢 PlayerLoadout / owned ship / station inventory projectionを専有 |
| `crates/dawn-sector/src/node/serialization.rs` | 470 | 🟢 InitialState / ship state / AoIの組み立てに限定 |
| `crates/dawn-sector/src/node/aoi.rs` | 636 | 🟢 AoI delivery / observer境界を保持 |
| `crates/dawn-wire/src/client_command.rs` | 475 | 🟢 wire schemaの型定義に限定 |
| `crates/dawn-client-gdext/src/client_command_gd.rs` | 332 | 🟢 GDExtension adapterに限定 |

## 前回ファイルサイズ一覧（2026-07-10 時点・履歴）

> **2026-07-10、全ファイル再計測（`/architecture-review`）。** 既存クレートの行数は前回
> （同日、`/doc-sync` 経由の部分計測）から変化なし——`commands.rs` 1573・`warp.rs` 1093・
> `transit_flow.rs` 949・`apply_event.rs` 887・`orbit.rs` 862・`mod.rs` 859・`inventory.rs` 851・
> `snapshot_io.rs` 710・`spawner_logic.rs` 671・`player_loadout_projection.rs` 559・
> `serialization.rs` 485。今回の差分は ADR-0039/0040 で新設された `dawn-client-core`
> （5ファイル・652行、Godot非依存クライアントドメインモデル）と `dawn-client-gdext`
> （4ファイル・658行、GDExtensionバインディング）を表へ追加したことのみ。
> `dawn-actor` 側は `protocol/mod.rs` 798 / `client_command.rs` 398 / `server_event.rs` 257 /
> `hello_resume.rs` 34 で、R-5 完了後の分割構造は維持されている。PR #129 で
> `dawn-client-core/src/loadout.rs` 337→373（`apply_module_activation` 委譲先 + テスト2件）・
> `dawn-client-gdext/src/loadout_gd.rs` 271→267（薄い委譲のみへ縮小）を反映。

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/warp.rs` | 1093 | 🟡 R-1 新設（2026-06-23）。warp 幾何の単一責務だが総行数が閾値を超過。2026-07-09 の 1024 から再び増加（テスト蓄積）、impl 自体は約534で700行未満 |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 671 | 🟢 P4-2 + P7-1 + ADR-0029 + ADR-0032。残るのは spawn mechanics（spawn / inventory seed）のみで、R-3 の観察対象から外れたまま |
| `crates/dawn-sector/src/node/bot_ai.rs` | 347 | 🟢 `spawner_logic.rs` から `process_bots` を抽出した Bot AI 決定ループ。純粋移動、挙動変更なし |
| `crates/dawn-sector/src/node/orbit.rs` | 862 | 🟡 ADR-0031 新設。Orbit / Keep at Range の操船一式。単一責務で許容だが総行数は watch 帯（impl 約326で700行未満） |
| `crates/dawn-sector/src/node/mod.rs` | 859 | 🟡 R-4 完了（2026-07-07）後、`coordinates.rs` 抽出時の821から再び38行増加（9B station関連のフィールド/アクセサ追加）。impl 約671で700行未満だが、R-4 と同じ蓄積パターンが再発しつつあるため watch へ格上げ |
| `crates/dawn-sector/src/node/coordinates.rs` | 190 | 🟢 R-4（2026-07-07新設）。`AnchorTable`（ADR-0029）呼び出し側の座標合成アクセサを一元化した deep module |
| `crates/dawn-sector/src/node/transit_flow.rs` | 949 | 🟢 `prepare_transit_commit`/`handle_transit_commit`（公開面 5→2 に集約）+ `rebase_after_transit`。大きいが責務は cohesive（impl 約368で700行未満） |
| `crates/dawn-sector/src/node/station.rs` | 53 | 🟢 2026-07-09、Station operations deepening 後の shared vocabulary module。`StationOperationOutcome` / `StationOperationRejection` だけを持ち、実装は sibling module へ移動 |
| `crates/dawn-sector/src/node/station_lifecycle.rs` | 407 | 🟢 2026-07-09 新設。dock / undock / active-ship selection / disembark / docked lock cleanup を所有する deep module |
| `crates/dawn-sector/src/node/station_materialization.rs` | 692 | 🟢 2026-07-09 新設。build / assemble / disassemble を所有する deep module。Ship materialization の検証と event append がここに集約。2026-07-10、カーゴ salvage 修正 + 回帰テスト3件追加（432→692、テスト増分が大半）。次回 `/architecture-review` で全体再計測時に確定grading |
| `crates/dawn-sector/src/node/station_inventory.rs` | 378 | 🟢 **新規記録**（本表に未記載だった）。ADR-0038（2026-07-08）新設。Station inventory の bounded in-memory cache + SQLite write-through seam。永続化の権威は `station_inventory_db.rs`、本ファイルは直近アクセスした player だけのキャッシュ層と `SimulationNode` 向けアクセサを所有 |
| `crates/dawn-sector/src/node/command_station.rs` | 164 | 🟢 **新規記録**（本表に未記載だった）。2026-07-09 新設。Station family（dock/undock/build/disassemble/select-active/assemble/disembark/transfer-to-station）の command dispatch を専有し、`commands.rs` から分離済み |
| `crates/dawn-sector/src/node/station_inventory_db.rs` | 328 | 🟢 ADR-0038（2026-07-08）新設。SQLite（rusqlite）による Station inventory の永続化権威 |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 710 | 🟡 P7-pre + ADR-0032（inventory 永続化）。ほぼテストだが総行数が閾値を超えたため watch へ |
| `crates/dawn-sector/src/node/inventory.rs` | 903 | 🟡 ADR-0032 新設。fit/unfit_module_owned + transfer + seed + テスト。2026-07-08〜09、PR #119/#121/#131（drag-and-drop reorder・per-station inventory分割・SEC-3/4/5修正）で851→903。大きいが責務は単一（impl 約292で700行未満）、総行数は watch 帯 |
| `crates/dawn-sector/src/node/commands.rs` | 1573 | 🟡 P7-1 + ADR-0032 + M-7（Issue #56）+ ADR-0035 + 9B station commands。command dispatch と操作検証の責務は保っているが、総行数は最大ファイルへ再肥大し続けている。impl（テスト除く）は約687で、R-3 のトリガー（700行超）に最も近い |
| `crates/dawn-sector/src/node/player_loadout_projection.rs` | 559 | 🟢 2026-07-09 新設。PlayerLoadout / owned ships / station inventory の JSON projection を一元化した deep module。`item_id_to_row_json` もここで唯一化され、row schema drift を防ぐ |
| `crates/dawn-sector/src/node/serialization.rs` | 485 | 🟢 InitialState / ship state / AoI / handoff payload の組み立てへ責務を縮小。PlayerLoadout projection を sibling module へ分離済み |
| `crates/dawn-sector/src/galaxy.rs` | 459 | 🟢 ADR-0029 AU→units 変換・ゲート AU 化 |
| `crates/dawn-sector/src/node/apply_event.rs` | 887 | 🟡 P7-pre + ADR-0032 + ADR-0035。replay apply の責務は単一。サイズは伸び続けており watch 帯だが、履歴再生の owner として一貫している（impl 約328で700行未満） |
| `crates/dawn-sector/src/node/tackle.rs` | 345 | 🟢 P7-pre。ADR-0035（PR #62）で距離判定を `entity_absolute_f64` の f64 差分に修正（真 AU スケールでの f32 丸め対策・ADR-0029 パターン準拠）。PR #66 で手組みの delta 計算を `SimulationNode::ship_distance` 呼び出しに置換し未使用 `PositionComp` import を削除（358→345） |
| `crates/dawn-sector/src/node/range_gate.rs` | 478（impl 149） | 🟢 ADR-0035 新設（PR #62）。Range Gate System（Step 5.5）— Weapon/Tackle/Remote Repair のターゲットが射程外に出たら強制 OFF（`ModuleDeactivated { forced_reason: OutOfRange }`）。PR #63 で flat-index 解決を `FittingComp::slot_at_flat_mut` に置換（403→382）。PR #66 で距離判定を `SimulationNode::ship_distance` 呼び出しに置換（382→362）。ADR-0036 で `effective_range_for_kind`/`process_range_gate` に Remote Repair 2 kind を追加 + 活性化/Range Gate/回復のテスト3件を追加（362→469） |
| `crates/dawn-sector/src/aoi.rs` | 629 | 🟢 `AoiDelivery`/`AoiSink`/`Observer`（旧 dawn-simulation・dawn-sector-node 重複の集約先）。半分弱はテスト。2026-07-01、`deliver_frame` を `<S: EventStore>` でジェネリック化 |
| `crates/dawn-sector/src/anchor.rs` | 311 | 🟢 ADR-0029 新設（AnchorTable・静的 f64 アンカー絶対座標）。2026-07-07、`/improve-codebase-architecture` で `node/mod.rs` が再実装していた逆変換を `to_relative()` として新設し、`rebase()` をその合成に書き直し。座標合成代数の唯一の所有者になった（292→311） |
| `crates/dawn-sector/src/transit.rs` | 419 | 🟢 PR #30 で `run_runtime_tick` / `RuntimeTickOutput` を追加。Request/Commit ハンドラが `prepare_transit_commit`/`handle_transit_commit` に委譲し Gate-lookup 知識を手放した。2026-07-02、`propose_jump` / `propose_auto_jump` を新設し、jump fallback outcome → `TransitOp::Request` 提案の組み立てを `dawn-sector-node`・`dawn-simulation` 双方の重複から集約 |
| `crates/dawn-sector/src/modules.rs` | 246 | 🟢 ADR-0033 で Active 修理モジュール定義を追加。ADR-0036 で Remote Shield Booster / Remote Armor Repairer を追加（211→246） |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 201 | 🟢 ADR-0032 で `ShipSnapshot.inventory` 追加 |
| `crates/dawn-sector/src/dilation.rs` | 164 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 174 | 🟢 |
| `crates/dawn-sector/src/node/approach.rs` | 566（impl 182） | 🟢 R-1 新設（2026-06-23）。approach 系 + ADR-0031 で clear_steering_modes 連携。2026-07-01、独自の検証チェックリストを `orbit.rs` の `begin_maneuver` 呼び出しに置き換え、Orbit/KeepAtRange と完全に同じ経路を通るように統一。同日、`apply_approach_jump_fallback`（1行ラッパー）を `jump.rs` へ移設・削除し、`apply_approach_command_with_auto_jump` を `pub(super)` 化。PR #68（候補2）で `dest_in_ship_frame_abs` を `node/mod.rs` へ移設（4サブモジュールから呼ばれる共有アクセサのため、577→562） |
| `crates/dawn-sector/src/node/jump.rs` | 250（impl 89） | 🟢 新設（2026-07-01）。PR #54 で `apply_jump_with_fallback`（3択フォールバック）、PR #55 で `resolve_auto_jump`（auto-jump 判定）を追加。両 PR でテストも追加（186→250）。impl 88 行で健全 |
| `crates/dawn-sector/src/node/tick.rs` | 267 | 🟢 P4-1 + ADR-0031 Step 2.55/2.56 + ADR-0033 Step 6.5 配線。PR #67 で cap-refit ループを `reapply_fitting` に、destroyed-ship 削除ループを `remove_ship` に置換（177→171） |
| `crates/dawn-sector/src/spawner.rs` | 133 | 🟢 |
| `crates/dawn-sector/src/ship_types.rs` | 108 | 🟢 |
| `crates/dawn-sector/src/node/navigation.rs` | 161 | 🟢 R-1 後。`can_propose_jump` / `can_propose_warp` + ADR-0017 dead-zone テスト |
| `crates/dawn-sector/src/node/ship_registry.rs` | 76 | 🟢 P3-1。PR #67（アーキテクチャレビュー候補1）で `remove(ship_id, world)` を新設し、index/type_ids/owners/by_player の削除と ECS despawn を1メソッドに集約（33→56）。従来は各削除元（tick.rs/apply_event.rs/transit_flow.rs）が個別に4-6行を手組みしており、`transit_flow.rs` の1箇所は owners/by_player の削除を欠落させていた。2026-07-07、ADR-0037 で `by_player` を `active_ship` に改名し、`remove()` は削除される船が実際に active だった場合のみ `active_ship` を消すよう修正（複数所有時に別の所有船削除でactiveポインタが誤って消える潜在バグの修正、56→76） |
| `crates/dawn-sector/src/node/sector_map.rs` | 28 | 🟢 P3-1 |

### dawn-actor（クライアント転送境界・M-4 集約先）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-actor/src/protocol/mod.rs` | 825 | 🟢 R-5 完了（2026-07-08）。wire protocol の公開面と統合テスト・schema freshness test を束ねる薄い入口に縮小。2026-07-11、`ClientCommandWire`/`EventWire`/Hello関連の実体は`dawn-wire`（下記）へ全面移動、ここは再エクスポートのみ（ADR-0041/0042）。同日、命名規則を`*Json`→`*Wire`へ統一し、本番未使用だった`parse_client_command`/`parse_hello`/`redirect_json`/`domain_event_to_json`を削除（ADR-0042追記） |
| `crates/dawn-actor/src/client_connection.rs` | 260 | 🟢 ClientConnection trait + InProcess/Ws 実装 |
| `crates/dawn-actor/src/ws_server.rs` | 317 | 🟢 M-4 集約（WsServer / PlayerSession）。2026-07-11、Welcome/Redirect/Event/Hello/Commandをpostcardバイナリ化（ADR-0042） |
| `crates/dawn-actor/src/lib.rs` | 41 | 🟢 |

`dawn-wire`（ADR-0041/0042、client<->server wire schema、`dawn-core`+serde+postcardのみ依存）:

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-wire/src/client_command.rs` | 483 | 🟢 client -> server wire translation の deep module。`ClientCommandWire` / `client_command_from_wire` / schema 出力を集約 |
| `crates/dawn-wire/src/server_event.rs` | 273 | 🟢 server -> client wire translation の deep module。`EventWire` / `domain_event_to_event_wire` を集約 |
| `crates/dawn-wire/src/hello_resume.rs` | 47 | 🟢 Hello / resume handshake の小さな補助モジュール |
| `crates/dawn-wire/src/lib.rs` | 97 | 🟢 crate doc + `ServerMessage`/`ClientMessage` 統合 enum |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 630 | 🟢 Raft クラスター配線（in-process テスト用） |
| `crates/dawn-simulation/src/serve/mod.rs` | 332 | 🟢 P5-1 共通ヘルパー。`node.apply_client_command` 呼び出しに統一済み |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 470 | 🟡 M-3（本番パス外・保留） |
| `crates/dawn-simulation/src/bench.rs` | 493 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 237 | 🟢 `AoiDelivery` を持ち、入力処理と runtime 呼び出し中心 |
| `crates/dawn-simulation/src/serve/runtime.rs` | 183 | 🟢 auto-jump / ownership handoff / scoped InitialState resend を集約 |
| `crates/dawn-simulation/src/serve/aoi_delivery.rs` | 119 | 🟢 配信ロジック本体を `dawn_sector::aoi::AoiDelivery` へ移動。残りは adapter のみ |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 224 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 235 | 🟢 P5-1。AoI delivery 詳細を `AoiDelivery` に委譲 |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 192 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 77 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 9 | 🟢 P5-2 |

### dawn-client-core（Godot非依存クライアントドメインモデル・ADR-0039）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-client-core/src/loadout.rs` | 373 | 🟢 2026-07-10 新設。`PlayerLoadoutMsg`（旧 `player_loadout.gd`）+ capacitor シミュレーション/武器射程/activation toggle の純粋関数。ユニットテスト含む。同日PR #129で `apply_module_activation`（`dawn-client-gdext` から委譲されたモジュール活性化状態の更新）を追加 |
| `crates/dawn-client-core/src/module_row.rs` | 126 | 🟢 2026-07-10 新設。`ModuleRow`/`ModuleKind`/`StatDelta`（旧 `module_row.gd`）。サーバーの `player_loadout_projection.rs` が送る wire 形状のミラー |
| `crates/dawn-client-core/src/item_row.rs` | 61 | 🟢 2026-07-10 新設。`ItemRow`/`ItemType`（旧 `item_row.gd`） |
| `crates/dawn-client-core/src/lib.rs` | 44 | 🟢 crate doc + re-export のみ。doctest 1件（C-EXAMPLE） |
| `crates/dawn-client-core/tests/server_contract_test.rs` | 84 | 🟢 `dawn-sector` を dev-dependency に取り、実サーバーの `build_player_loadout_json()` 出力をこの crate の型でパースできることを確認する契約テスト（DAGには影響しない） |

### dawn-client-gdext（GDExtensionバインディング・ADR-0040）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-client-gdext/src/loadout_gd.rs` | 267 | 🟢 2026-07-10 新設。`PlayerLoadout` GDExtension クラス。`dawn-client-core::PlayerLoadoutMsg` の薄いラッパー、Variant/GString ⇄ Rust 型変換のみでドメインロジックは持たない。同日PR #129で `apply_module_activation` の状態変更ロジックを `dawn-client-core` へ委譲し、ADR-0040 の thin-adapter 方針に完全準拠 |
| `crates/dawn-client-gdext/src/module_row_gd.rs` | 253 | 🟢 2026-07-10 新設。`ModuleRow` GDExtension クラス。旧 GDScript の `equals()`/`clone()` API を維持し `hud_surface.gd` の diffing 実装が無改修で動くようにしている |
| `crates/dawn-client-gdext/src/item_row_gd.rs` | 115 | 🟢 2026-07-10 新設。`ItemRow` GDExtension クラス |
| `crates/dawn-client-gdext/src/client_command_gd.rs` | 332 | 🟢 ADR-0041/0042。`ClientCommand` GDExtension クラス（コマンド送信、schema駆動`build()`）+ `ClientMessageDecoder`（テスト専用） |
| `crates/dawn-client-gdext/src/server_message_gd.rs` | 71 | 🟢 2026-07-11新設（ADR-0042）。`ServerMessageDecoder`（postcardバイト列→Dictionary） |
| `crates/dawn-client-gdext/src/json_variant.rs` | 55 | 🟢 2026-07-11新設（ADR-0042）。`ServerMessageDecoder`/`ClientMessageDecoder`共有のJSON⇄Variant変換ヘルパー |
| `crates/dawn-client-gdext/src/lib.rs` | 24 | 🟢 crate doc + `#[gdextension]` エントリポイントのみ |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 593 | 🟡 許容範囲（Raft 実装の核） |
| `crates/dawn-sector-node/src/runtime.rs` | 259 | 🟢 production Node の jump fallback / tick stepping / replication publish 呼び出し / Redirect / AoI delivery を集約。本ファイルは orchestration のみ。2026-07-11、Redirect送信を`ServerMessage::Redirect`+`send_message`（postcardバイナリ）に置換（ADR-0042） |
| `crates/dawn-sector-node/src/client_admission.rs` | 236 | 🟢 client admission state machine |
| `crates/dawn-sector-node/src/main.rs` | 338 | 🟢 8D-4 本番バイナリ。config / TCP transport / accept channel / data loading の配線に縮小 |
| `crates/dawn-core/src/events.rs` | 694 | 🟡 domain event 定義の中核。大きいが責務は単一で、wire/schema 変換や apply は持ち込まない。継続的な variant 追加で700行に近づいており次回計測で watch 候補 |
| `crates/dawn-core/src/item.rs` | 36 | 🟢 ADR-0034 `ItemId`（`Module`/`PackagedShip`/`ScrapMetal`）— 経済系機能全体が参照する小さく安定した型定義 |
| `crates/dawn-ecs/src/systems/combat.rs` | 578 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 485 | 🟢 capacitor tick と module force-off の owner。大きいが責務は単一 |
| `crates/dawn-consensus/src/actor.rs` | 476 | 🟢 8D-5 実機検証で使う Raft role-transition ログ（`eprintln!`）を保持 |
| `crates/dawn-event-store/src/file.rs` | 464 | 🟢 |
| `crates/dawn-core/src/fitting.rs` | 430 | 🟢 `ModuleDefinition`/`ModuleKind`/`SlotKind`/`StatDelta`/`FittingSnapshot` 等、Fitting ドメイン型の定義一式 |
| `crates/dawn-consensus/src/transport.rs` | 204 | 🟢 `RaftTransport` trait 定義 + in-process 実装 |
| `crates/dawn-event-store/src/memory.rs` | 184 | 🟢 `InMemoryEventStore` |
| `crates/dawn-ecs/src/systems/movement.rs` | 415 | 🟢 |
| `crates/dawn-ecs/src/systems/lock.rs` | 375 | 🟢 |
| `crates/dawn-core/src/commands.rs` | 584 | 🟢 Command enum 群（継続的に variant 追加）。M-7（Issue #56）で `ClientCommand` enum を `dawn-actor` から移動。ADR-0037 系の active ship / station 操作コマンド群もここに収まる |
| `crates/dawn-consensus/src/rpc.rs` | 371 | 🟢 Raft RPC 型定義 |
| `crates/dawn-consensus/src/tcp_transport.rs` | 353 | 🟢 8D-3 TcpRaftTransport |
| `crates/dawn-ecs/src/systems/fitting.rs` | 315 | 🟢 fitting の適用ロジック |
| `crates/dawn-replication/src/tcp.rs` | 288 | 🟢 8D-2c |
| `crates/dawn-ecs/src/components/movement.rs` | 291 | 🟢 movement component 群 |
| `crates/dawn-ecs/src/world.rs` | 294 | 🟢 クエリヘルパー |
| `crates/dawn-sector-node/src/data_loader.rs` | 286 | 🟢 module/ship type TOML ローダー |
| `crates/dawn-ecs/src/components/fitting.rs` | 384 | 🟢 `FittedSlot` と fitting component 群の owner |
| `crates/dawn-ecs/src/components/combat.rs` | 376 | 🟢 |
| `crates/dawn-replication/src/anti_entropy.rs` | 216 | 🟢 8D-2b |
| `crates/dawn-ecs/src/systems/repair.rs` | 257 | 🟢 ADR-0033 の repair system |
| `crates/dawn-replication/src/replica.rs` | 225 | 🟢 M-5（ReplicaSet・複製ログ消費側） |
| `crates/dawn-replication/src/bus.rs` | 237 | 🟢 8D-2a |
| `crates/dawn-core/src/navigation.rs` | 253 | 🟢 ナビゲーション型定義 |
| `crates/dawn-replication/src/snapshot.rs` | 175 | 🟢 8D-2d SnapshotTransfer（ジェネリック / 256 MiB cap） |
| `crates/dawn-core/src/ship_type.rs` | 181 | 🟢 |
| `crates/dawn-replication/src/outbound.rs` | 142 | 🟢 sender-side `OutboundLogPublisher`。append-log cursor と `LogBatch` suffix 構築を保持 |
| `crates/dawn-replication/src/lib.rs` | 110 | 🟢 8D-2a/2b/2c/2d public API |
| `crates/dawn-ecs/src/components/inventory.rs` | 127 | 🟢 ADR-0032 新設（InventoryComp）。2026-07-08、ADR-0034 9B `take_all`（whole-stack removal、`TransferToStationCommand`向け）+ テスト2件を追加（104→127） |
| `crates/dawn-sector-node/src/config.rs` | 98 | 🟢 8D-4 TOML 静的 config。2026-07-01、永続化パス（`event_log_path`/`snapshot_path`/`cold_path`/`checkpoint_interval_ticks`）を追加（全て `#[serde(default)]` 付きで後方互換） |
| `crates/dawn-core/src/entity.rs` | 140 | 🟢 **新規記録**（本表に未記載だった）。`EntityId`/hecs 変換ヘルパー |
| `crates/dawn-core/src/position.rs` | 125 | 🟢 **新規記録**（本表に未記載だった）。`Position`/`Velocity` 型定義 |
| `crates/dawn-core/src/sector.rs` | 105 | 🟢 **新規記録**（本表に未記載だった）。`SectorId`/`SectorBounds` 型定義 |

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
