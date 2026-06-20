---
scope    : コードベース全体の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時
related  : CLAUDE.md §11, docs/architecture.md
date     : 2026-06-20（P9-2 完了後に更新）
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
| ファイルサイズ | A− | P7-1/P7-2 + AoI テスト移動で node/mod.rs 514行に縮小。全ファイル 700行以下 |
| 型設計 | A− | SectorMap・ShipRegistry 抽出 + P9-2 で `CelestialBodyDef.sector` 追加。近似ロジック解消 |
| 重複 | A− | WS 境界（ws_server / protocol）を dawn-actor へ集約（M-4 解消）。残るは data_loader 重複のみ |
| Rust固有 | A− | Box\<dyn\> ゼロ・Mutex 最小。TCP transport も trait 境界内に収まる |
| AI開発由来 | B+ | 命名汚染なし。残る密結合は `SectorSimulatorActor` と `SimulationNode` 境界 |

---

## ファイルサイズ一覧（2026-06-19 時点）

### dawn-sector（ゲームロジック）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-sector/src/node/mod.rs` | 575 | 🟢 P7-2 jump/warp validation 移動後 |
| `crates/dawn-sector/src/node/navigation.rs` | 679 | 🟢 P7-2（validation + 実装 + approach/warp テスト） |
| `crates/dawn-sector/src/node/spawner_logic.rs` | 485 | 🟢 P4-2 + P7-1（実装 394行 + bot テスト 91行） |
| `crates/dawn-sector/src/node/transit_flow.rs` | 402 | 🟢 P7-1（実装 + 近接テスト） |
| `crates/dawn-sector/src/node/commands.rs` | 342 | 🟢 P7-1（実装 262行 + fitting/combat テスト 80行） |
| `crates/dawn-sector/src/galaxy.rs` | 203 | 🟢 TOML schema parser 統合 |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 391 | 🟢 P7-pre |
| `crates/dawn-sector/src/node/apply_event.rs` | 226 | 🟢 P7-pre |
| `crates/dawn-sector/src/node/tackle.rs` | 204 | 🟢 P7-pre |
| `crates/dawn-sector/src/aoi.rs` | 246 | 🟢 |
| `crates/dawn-sector/src/transit.rs` | 215 | 🟢 |
| `crates/dawn-sector/src/persistence/snapshot.rs` | 168 | 🟢 |
| `crates/dawn-sector/src/dilation.rs` | 160 | 🟢 |
| `crates/dawn-sector/src/node/serialization.rs` | 158 | 🟢 |
| `crates/dawn-sector/src/persistence/checkpoint.rs` | 156 | 🟢 |
| `crates/dawn-sector/src/modules.rs` | 137 | 🟢 |
| `crates/dawn-sector/src/spawner.rs` | 127 | 🟢 |
| `crates/dawn-sector/src/node/tick.rs` | 139 | 🟢 P4-1 + P7-1（実装 91行 + tick テスト 48行） |
| `crates/dawn-sector/src/ship_types.rs` | 82 | 🟢 |
| `crates/dawn-sector/src/node/ship_registry.rs` | 33 | 🟢 P3-1 |
| `crates/dawn-sector/src/node/sector_map.rs` | 25 | 🟢 P3-1 |

### dawn-simulation（配線・起動）

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-simulation/src/cluster.rs` | 528 | 🟢 Raft クラスター配線 |
| `crates/dawn-simulation/src/sector_simulator_actor.rs` | 423 | 🟡 M-3 |
| `crates/dawn-simulation/src/bench.rs` | 411 | 🟢 |
| `crates/dawn-simulation/src/serve/mod.rs` | 402 | 🟢 P5-1 共通ヘルパー |
| `crates/dawn-simulation/src/protocol.rs` | 309 | 🟢 |
| `crates/dawn-simulation/src/serve/cluster.rs` | 241 | 🟢 P5-1 |
| `crates/dawn-simulation/src/ws_server.rs` | 199 | 🟢 |
| `crates/dawn-simulation/src/data_loader/modules.rs` | 190 | 🟢 P5-2 |
| `crates/dawn-simulation/src/serve/single.rs` | 177 | 🟢 P5-1 |
| `crates/dawn-simulation/src/data_loader/ship_types.rs` | 174 | 🟢 P5-2 |
| `crates/dawn-simulation/src/main.rs` | 63 | 🟢 |
| `crates/dawn-simulation/src/data_loader/mod.rs` | 9 | 🟢 P5-2 |

### その他クレート

| ファイル | 行数 | 判定 |
|---|---|---|
| `crates/dawn-consensus/src/state.rs` | 573 | 🟡 許容範囲（Raft 実装の核）|
| `crates/dawn-core/src/events.rs` | 495 | 🟢 |
| `crates/dawn-event-store/src/file.rs` | 431 | 🟢 |
| `crates/dawn-ecs/src/systems/combat.rs` | 430 | 🟢 |
| `crates/dawn-consensus/src/actor.rs` | 430 | 🟢 |
| `crates/dawn-ecs/src/systems/capacitor.rs` | 412 | 🟢 |
| `crates/dawn-consensus/src/rpc.rs` | 343 | 🟢 Raft RPC 型定義 |
| `crates/dawn-consensus/src/tcp_transport.rs` | 330 | 🟢 8D-3 TcpRaftTransport |
| `crates/dawn-sector-node/src/main.rs` | 337 | 🟢 8D-4 本番バイナリ（TCP 配線・WS・Jump Redirect）|
| `crates/dawn-sector-node/src/protocol.rs` | 243 | 🟢 8D-4 WS プロトコル |
| `crates/dawn-sector-node/src/data_loader.rs` | 178 | 🟢 8D-4 module/ship type TOML ローダー |
| `crates/dawn-replication/src/tcp.rs` | 263 | 🟢 8D-2c |
| `crates/dawn-core/src/navigation.rs` | 121 | 🟢 ナビゲーション型定義（star_system.rs より改名）|
| `crates/dawn-sector-node/src/ws_server.rs` | 153 | 🟢 8D-4 WebSocket サーバー |
| `crates/dawn-ecs/src/world.rs` | 252 | 🟢 P6-1 クエリヘルパー追加 |
| `crates/dawn-replication/src/anti_entropy.rs` | 211 | 🟢 8D-2b |
| `crates/dawn-replication/src/bus.rs` | 188 | 🟢 8D-2a |
| `crates/dawn-replication/src/snapshot.rs` | 164 | 🟢 8D-2d SnapshotTransfer（ジェネリック / 256 MiB cap） |
| `crates/dawn-replication/src/lib.rs` | 71 | 🟢 8D-2a/2b/2c/2d public API |

---

## 問題一覧

### Medium

#### M-3: `sector_simulator_actor.rs` と `SimulationNode` の密結合

`SectorSimulatorActor` は `SimulationNode` の公開メソッドをほぼ全て呼ぶ薄いラッパー。
`SimulationNode` の変更が即 Actor に波及する。
8D-2a/2b/2c で `dawn-replication` 配線が入り、イベント flush 境界と TCP transport 境界は明確になった。
ただし `SimulationNode` の公開メソッド変更が Actor に波及しやすい構造は残る。

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

残課題: `data_loader`（`load_modules` / `load_ship_types`）の重複は未解消。
ファイル I/O を伴うためファイル I/O 禁止の `dawn-core` / `dawn-sector` にも、転送境界の
`dawn-actor` にも意味的に合わず、置き場の判断（新規 `dawn-data` クレート等）が要る。別途検討。

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

---

### Phase 8 — 物理ノード分散の配線（Phase 8D 完了）

`dawn-replication`（ADR-0021/0027・Phase 8D）は全ステップ完了済み。

次の自然な前進先:

- **8D-5: Raspberry Pi 実機検証**（3 物理ノード / LAN 平文）

採らない方針も維持する:

- CRDT / LWW-Register は採らない（単一所有 + append-only log gossip）
- protobuf / `dawn-proto` は採らない（wire は postcard 再利用）
- TLS / 認証は第1次 LAN 検証では扱わない

---

### Phase 9 — 評価 A への引き上げ

現在の総合評価は **A−**。A に上げるための残タスク（優先度順）:

#### P9-1: M-3 解消 — `SectorSimulatorActor` / `SimulationNode` 境界の明確化

`SectorSimulatorActor`（423行）は `SimulationNode` の公開メソッドをほぼ全て呼ぶ薄いラッパーで、
`SimulationNode` の変更が即 Actor に波及する。

方針:
- `SimulationNode` の外部インタフェースをコマンド/応答の enum に絞り込む
- Actor は「何を呼ぶか」ではなく「何を送るか」に依存する形に変える
- ADR を起票して境界を明文化する

着手条件: 8D-5 実機検証の完了後（分散配線で境界の揺れが確定してから）

#### P9-2: `CelestialBodyDef` へのセクター帰属フィールド追加（完了）

`CelestialBodyDef.sector` を追加し、`Galaxy::bodies_in_sector` は ID 割り当て規約ではなく
明示フィールドで絞り込むようになった。`data/galaxy.toml` / `data/galaxy.demo.toml`
も `sector` フィールドを持つ。
型設計の残り違和感は解消済み。

---

## 触らない箇所（安定・枯れている）

以下は正しく動作しており、リファクタの対象にしない:

- `dawn-ecs` systems（combat / movement / lock / capacitor）— 凝集度高・純粋関数的
- `dawn-consensus`（Raft 合意層）— 正しいアルゴリズム、変更リスク高
- `dawn-core` / `dawn-event-store`（Event sourcing 基盤）— 設計の核、INV-001 維持
- `dawn-actor`（ClientConnection 境界）— replication 責務は `dawn-replication` へ移動済み
