---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture.md
date     : 2026-06-23（ADR-0029 真スケール座標後にファイルサイズ一覧を再計測）
---

# Architecture Review — Dawn Codebase

Rust シニアアーキテクト視点での現状分析と改善ロードマップ。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: A−**

| 観点 | 評価 | 理由 |
|---|---|---|
| クレート構成 | A− | DAG が設計通り。dawn-sector / dawn-replication が分離済み（ADR-0026/0027） |
| ファイルサイズ | B+ | mod.rs は 646行で安定。ただし ADR-0029（真スケール座標）でワープ遷移を絶対 f64 フレームに書き換え、`node/navigation.rs` が 679→1092行に肥大（700行超は現状ここだけ・要分割候補） |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。近似ロジック解消 |
| 重複 | A− | WS 境界は dawn-actor へ集約（M-4 解消）。残る両バイナリ間グルー重複（M-6）は ~230行・低ドリフトで許容判断（新規クレートは過剰）|
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。TCP transport も trait 境界内に収まる |
| AI開発由来 | A− | 命名汚染なし。残る `SectorSimulatorActor` の密結合（M-3）は本番パス外の in-process 専用で実害小 |

---

## ファイルサイズ一覧（2026-06-23 時点）

> 2026-06-19 の前回計測から、ADR-0029（真スケール座標）の実装でワープ遷移・座標変換・
> シリアライズ周りが増加。M-4（WS 境界集約）で `protocol.rs` / `ws_server.rs` は両バイナリ
> から削除され `dawn-actor` に集約済み（下の dawn-actor 表）。

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/navigation.rs` | 1092 | 🔴 ADR-0029 でワープ遷移を絶対 f64 フレーム（Hermite）に書き換え 679→1092。要分割候補 |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 662 | 🟢 P4-2 + P7-1（実装 + bot テスト）+ ADR-0029 `set_spawn_anchor_abs` |
| `crates/dawn-sector/src/node/mod.rs` | 646 | 🟢 P7-2 jump/warp validation 移動後 |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 442 | 🟢 P7-pre |
| `crates/dawn-sector/src/node/transit_flow.rs` | 407 | 🟢 P7-1（実装 + 近接テスト） |
| `crates/dawn-sector/src/node/commands.rs` | 342 | 🟢 P7-1（実装 262行 + fitting/combat テスト 80行） |
| `crates/dawn-sector/src/node/serialization.rs` | 300 | 🟢 ADR-0029 でアンカー相対座標の WS 直列化を追加（158→300） |
| `crates/dawn-sector/src/galaxy.rs` | 286 | 🟢 ADR-0029 AU→units 変換・ゲート AU 化（203→286） |
| `crates/dawn-sector/src/aoi.rs` | 265 | 🟢 |
| `crates/dawn-sector/src/anchor.rs` | 246 | 🟢 ADR-0029 新設（AnchorTable・静的 f64 アンカー絶対座標） |
| `crates/dawn-sector/src/node/apply_event.rs` | 244 | 🟢 P7-pre |
| `crates/dawn-sector/src/transit.rs` | 216 | 🟢 |
| `crates/dawn-sector/src/node/tackle.rs` | 208 | 🟢 P7-pre |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 173 | 🟢 |
| `crates/dawn-sector/src/dilation.rs` | 160 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 156 | 🟢 |
| `crates/dawn-sector/src/node/tick.rs` | 140 | 🟢 P4-1 + P7-1（実装 + tick テスト） |
| `crates/dawn-sector/src/modules.rs` | 137 | 🟢 |
| `crates/dawn-sector/src/spawner.rs` | 127 | 🟢 |
| `crates/dawn-sector/src/ship_types.rs` | 82 | 🟢 |
| `crates/dawn-sector/src/node/ship_registry.rs` | 33 | 🟢 P3-1 |
| `crates/dawn-sector/src/node/sector_map.rs` | 25 | 🟢 P3-1 |

### dawn-actor（クライアント転送境界・M-4 集約先）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-actor/src/protocol.rs` | 335 | 🟢 M-4 で両バイナリから集約（DomainEvent↔JSON↔ClientCommand） |
| `crates/dawn-actor/src/client_connection.rs` | 254 | 🟢 ClientConnection trait + InProcess/Ws 実装 |
| `crates/dawn-actor/src/ws_server.rs` | 188 | 🟢 M-4 で両バイナリから集約（WsServer / PlayerSession） |
| `crates/dawn-actor/src/lib.rs` | 29 | 🟢 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 538 | 🟢 Raft クラスター配線 |
| `crates/dawn-simulation/src/serve/mod.rs` | 429 | 🟢 P5-1 共通ヘルパー |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 423 | 🟡 M-3 |
| `crates/dawn-simulation/src/bench.rs` | 414 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 248 | 🟢 P5-1 |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 190 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 178 | 🟢 P5-1 |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 174 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 65 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 9 | 🟢 P5-2 |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 573 | 🟡 許容範囲（Raft 実装の核）|
| `crates/dawn-core/src/events.rs` | 535 | 🟢 |
| `crates/dawn-ecs/src/systems/combat.rs` | 469 | 🟢 |
| `crates/dawn-consensus/src/actor.rs` | 441 | 🟢 |
| `crates/dawn-event-store/src/file.rs` | 431 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 412 | 🟢 |
| `crates/dawn-sector-node/src/main.rs` | 396 | 🟢 8D-4 本番バイナリ（TCP 配線・WS・Jump Redirect）|
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

#### M-4（一部解消）: クライアント配線層が `dawn-simulation` と `dawn-sector-node` で重複

以前は `dawn-sector-node` の WS/プロトコル/データロード層が `dawn-simulation` の手動コピーで、
3ファイルすべてが「Adapted from …」「kept in sync manually」と保守負債を明記していた。

解消（2026-06-20）: **`ws_server` と `protocol` を `dawn-actor` へ集約**した。
`dawn-actor/src/client_connection.rs` のドキュメントが既に `WsClientConnection (ws_server.rs)`
を「本番 WebSocket transport」と記述しており、移動は設計意図の実現（charter 変更ではない）。

- `WsServer` / `WsClientConnection` / `PlayerSession` → `dawn-actor::ws_server`
  （`bind` を `ToSocketAddrs + Display` でジェネリック化し両呼び出し元に対応）
- `parse_client_command` / `domain_event_to_json` / JSON DTO / `redirect_json` → `dawn-actor::protocol`
- 両バイナリは重複ファイルを削除し `use dawn_actor::{protocol, ws_server}` に切替
- 不要になった依存（`tokio-tungstenite` / `futures-util` ほか）を両 Cargo.toml から除去

残課題は M-6 に集約（`data_loader` 以外にも `deliver_aoi_frame` / `spawn_npcs` が重複）。

#### ~~M-5~~（機能ギャップ）: 受信 replication batch が消費されていない（解消済み）

以前は `dawn-sector-node` の tick ループが受信 `LogBatch` をログ出力するだけで破棄しており、
ゴシップが「送るだけ」で複製の消費側が未実装だった。

解消（2026-06-20）: `dawn-replication` に **`ReplicaSet`** を新設し、受信ループに配線した。
既存の `AntiEntropy::plan_batch` を使い、peer セクターごとに **gap 検出・冪等・順序保持**で
追記ログの複製を保持する（ADR-0021 のログシッピング消費側）。

意図的に範囲外とした2点（別機能・別設計が必要）:
- 複製イベントをライブ `SimulationNode` world へ適用すること
  （別セクター座標の艦が自セクターの AoI/衝突を壊すため）
- failover takeover（複製を所有へ昇格）

`ReplicaSet` は順序付き追記ログを保持するところまでで、これは将来の read / failover 経路が
消費する前提データである。誤解を招く「8D-2d scope」コメントも除去した。

#### M-6（許容）: `dawn-sector-node` が `dawn-simulation` の serve 層をフォークしている

M-4（WS 境界）解消後も、両バイナリの「アプリケーション層」グルーが重複している:

| 重複 | dawn-simulation | dawn-sector-node | 備考 |
|---|---|---|---|
| `data_loader`（`load_modules` / `load_ship_types` / `parse_*`） | `data_loader/*.rs`（実装 ~280行）| `data_loader.rs`（178行）| TOML ローダー |
| `deliver_aoi_frame`（AoI フレーム配信） | `serve/mod.rs:207` | `main.rs:338` | **実質同一**（~40行）|
| `spawn_npcs` / `spawn_npc_frigates` | `serve/mod.rs:278` | `main.rs:325` | **実質同一**（~12行）|

根本原因: **`dawn-actor`（転送境界）と `dawn-sector`（ゲームロジック）の両方に依存する
共有ライブラリが存在しない**。両方に依存するのは2バイナリだけなので、両者を組み合わせる
グルー（セッション AoI 配信・NPC スポーン・データロード・serve ループ）は共有の置き場がなく、
各バイナリにコピーされる。8D-4 で `dawn-sector-node` を `dawn-simulation` の serve 経路から
コピーして作ったことで顕在化した。

これは M-4 で `data_loader` を `dawn-actor` に置けなかった理由（I/O 禁止）と同根で、
個別ファイルの置き場問題ではなく**共有アプリ層クレートの欠如**である。

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
  「統合コストが効果を上回る」とスキップ。~230行の安定したグルーの重複も同じ費用対効果で許容が妥当。
- **ドリフトの実害が小さい**: M-4 で直した `protocol`（18 variant・変更頻度高）と違い、
  `data_loader` / `deliver_aoi_frame` は変更頻度が低く無言バグ化のリスクは限定的。

再評価トリガー（このいずれかが起きたら設計し直す）:
- `data_loader` / `deliver_aoi_frame` が実際にドリフトしてバグを生んだとき
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
| 8D-5 観測ログ仕込み | 2026-06-20 | Raft role 遷移 / TCP 再接続 / tick オーバーランを stderr 出力（実機検証で症状を切り分けるため）。`docs/8d5-hardware-notes.md` 追加・localhost 3 プロセス検証済み |
| M-5 replication 消費側 | 2026-06-20 | `dawn-replication::ReplicaSet` 新設。受信 `LogBatch` を peer セクターごとに gap 検出・冪等・順序保持で複製ログに取り込む（ライブ world 適用 / failover は範囲外）|
| M-4 WS 境界の集約 | 2026-06-20 | `ws_server` / `protocol` を `dawn-actor` へ移動し dawn-simulation / dawn-sector-node の手動コピーを解消（506行削除）。`bind` を `ToSocketAddrs` ジェネリック化・不要依存を除去 |

> Phase 2〜7 の構造リファクタ、Phase 8D の TCP 分散配線、M-4/M-5 の重複/機能ギャップ解消は
> すべて完了。コードベースの品質リファクタは一区切り。

### 未完了・保留

残るのは以下のみ。いずれも本番品質には直結せず、意識的に「今はやらない」と判断した項目。

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| 8D-5 Raspberry Pi 実機検証 | 機能・外部依存待ち | ハードウェア未購入。観測ログ・config・localhost 検証は済み（完了済み参照）。Pi 入手後に着手 |
| M-3 `SectorSimulatorActor` 密結合 | 品質・保留 | 本番パス外（in-process テスト/ベンチ専用）。P9-1 撤回。優先度低 |
| M-6 アプリ層グルー重複（`data_loader` / `deliver_aoi_frame` / `spawn_npcs`） | 品質・許容 | ~230行・低ドリフト。新規クレートは過剰と判断。再評価トリガー付き |

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

タスクは全て決着済み。総合評価は **A−** に据え、A への無理な引き上げは行わない。
残る M-3（本番パス外）・M-6（許容）は「やらない」と意識的に判断したもので、
本番品質には直結しない。これ以上の構造リファクタは費用対効果が見合わないため、
A− を適正な落としどころとする。次の前進先は品質リファクタではなく
**8D-5 実機検証**（roadmap）や戦闘の深み（ADR-0016 §5）といった機能側。

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

残る品質観点は **重複**（M-6・許容）と **密結合**（M-3・本番パス外で低優先）のみで、
いずれも本番品質には直結しない（「未完了・保留」参照）。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（ClientConnection 境界）— replication 責務は `dawn-replication` へ移動済み
