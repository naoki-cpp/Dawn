---
scope    : コードベース全体の保守性・設計品質レビュー — 完了済み作業ログ
audience : AI Agent / Human Developer
update   : /architecture-review が issue を解消済みへ移動するたびに追記
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-pending.md（未完項目）
date     : 2026-07-09
---

# Architecture Review — Dawn Codebase（完了済みログ）

[server.md](./server.md) の issue のうち、
解消済み・決着済みのものをここに時系列で記録する。**分析のみ。過去分の削除・改変は行わない
（監査ログとして追記のみ）。**

---

## 改善ロードマップ > 完了済み

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
| PlayerLoadout projection module の deepening（`/improve-codebase-architecture`） | 2026-07-09 | `serialization.rs` から PlayerLoadout / owned ships / station inventory の JSON projection を `player_loadout_projection.rs` へ分離。 |
| Owned ship / Active ship モデル実装（ADR-0037、Phase 9B-5 Assemble の前提） | 2026-07-07 | `owners` と `active_ship` を分離し、操縦系コマンドを active ship 解決に統一。 |
| Phase 9B-5 Assemble コマンド実装 + RefreshFitting player_id 化（`/add-event`） | 2026-07-07 | `AssembleCommand` / `ShipAssembled` を実装し、followup を `PlayerId` 基準に修正。 |
| DisembarkCommand 実装（ADR-0037、船を降りる操作の一級化） | 2026-07-07 | `DisembarkCommand` と `disembark_owned` を追加。 |
| Disembark後のクライアント可視性ギャップ修正 + station近接表示の複数化 | 2026-07-07 | `active_ship_id` を wire に追加し、HUD / station proximity 表示を追従。 |
| 新規プレイヤーへのスターターPackagedShip付与 + 複数所有船切り替えUI | 2026-07-08 | starter packaged ship と owned ship roster UI を追加。 |
| Ship cargo / Station inventory UI分離 + TransferToStationCommand実装 | 2026-07-08 | HUD を cargo/station 分離し、`TransferToStationCommand` を追加。 |
| ItemId→ItemRow JSON変換の重複除去（`/improve-codebase-architecture`） | 2026-07-08 | `item_id_to_row_json` に集約して row schema drift を防止。 |

---

## 個別 issue の解消経緯

#### ~~M-7~~（解消済み 2026-07-01）: Player Command Dispatch のルーティングが `dawn-sector` の外に漏れていた

`runtime.rs`/`protocol.rs` に分かれていたコマンドルーティングを `SimulationNode::apply_client_command`
に統一して解消。`ClientCommand` を `dawn-core` へ移動し DAG ブロッカーを外したうえで集約した
（Issue #56）。

#### ~~Steering-mode 排他制御の非対称性~~（解消済み 2026-07-01）

`/improve-codebase-architecture` の「5ハンドラの重複」指摘を調査する過程で、スタイル上の重複ではなく
実害のある非対称性3件（Warpが`clear_steering_modes`を呼ばない、Approachに`is_warping`ガードがない、
Aligningフェーズ中はWarp優先チェックが素通りする）を発見・修正。

#### ~~R-4~~（新設 2026-07-06・完了 2026-07-07）: `node/mod.rs` の impl が700行トリガーを超過

`node/mod.rs` は2026-07-03時点で impl 641行（総行数829）と R-3の観察対象に含まれ、
「700行超で着手」というトリガー付きで保留されていた。2026-07-06の再計測で総行数936・
impl 748行と判明し、R-3自身が定めたトリガーが発火した。

**根本原因**: `mod.rs` は「フィールド定義（構造体宣言・定数）」と「補助impl（ヘルパーメソッド群）」
が同居する構造になっている。ADR-0031/0032/0035のたびにフィールドと対応する小さなimplメソッドが
両方とも `mod.rs` に積まれ続けた。

**2026-07-07、一部着手（`/improve-codebase-architecture` 発の deepening）**: 「補助impl」の
中身を精査した結果、`entity_absolute_f64`/`dest_in_ship_frame_abs`/`ship_distance` は単なる
共有アクセサではなく、`AnchorTable`（`anchor.rs`、ADR-0029の座標合成代数）の一部をmod.rs側で
再実装していたと判明。`AnchorTable` に `to_relative()`（`absolute()`の逆変換）を新設し、
`rebase()` をその合成として書き直したうえで、`entity_absolute_f64`/`dest_in_ship_frame_abs` は
`anchor_table.absolute()`/`to_relative()` を呼ぶだけに、`ship_distance` は各Shipを
`(AnchorId, offset)` に解決してから `anchor_table.distance()` に委譲する形に置き換えた
（f32ラッパー `entity_absolute`/`entity_abs_pos` はECS由来のオフセット読み出しが関心事のため
mod.rsに残置）。挙動変更なし・`cargo test --workspace` / `fmt` / `clippy -D warnings` 全件通過。
`CONTEXT.md` にAnchorの語彙を追加済み。

**2026-07-07、R-4本体を完了（`/improve-codebase-architecture` → `/grilling` 経由）**:
座標合成アクセサ群（`entity_absolute`/`entity_abs_pos`/`entity_abs_pos_f64`/
`entity_absolute_f64`/`dest_in_ship_frame_abs`/`ship_distance`/`ship_distance_to_point`/
`ship_anchor_and_offset`）と、その両方が使う `debug_assert_missing_anchor` を新設
`node/coordinates.rs` へ `impl<S: EventStore> SimulationNode<S>` ブロックごと移動（可視性は
`pub(super)`/`pub`のまま完全維持）。`mod.rs` は構造体宣言・定数・コンストラクタ・population
backstop・identity/observation アクセサに絞られ、939→821行、implは700行未満に戻った。
純粋移動で挙動変更なし。`cargo test --workspace`（dawn-sector 229/229）/ `fmt` /
`clippy -D warnings` 全件通過。

#### ~~R-5~~（新設 2026-07-08・完了 2026-07-08）: `dawn-actor/src/protocol.rs` の深分割

前回レビューでは `dawn-actor/src/protocol.rs` が1003行（impl 701）まで膨らみ、
`EventJson` / `ClientCommandJson` / Hello/resume解析 / schema freshness testが単一ファイルへ
積み重なっていた。自然な分割軸は既に見えており、実際のコードでも server->client /
client->server / hello-resume が別の関心事として育っていた。

**根本原因**: wire protocolという単一責務の中で、メッセージfamilyごとの進化速度が異なるのに、
実装上の所有権が1ファイルに閉じ込められていたこと。

**解消済み。** 現在は `protocol/mod.rs`（入口と統合テスト） / `protocol/client_command.rs`
（client -> server変換） / `protocol/server_event.rs`（server -> client変換） /
`protocol/hello_resume.rs`（Hello/resume handshake）へ分割済み。最大ファイルは
`protocol/mod.rs` 710行だが、その大半は統合テストで、変換ロジック本体はfamilyごとの
deep moduleに移っている。

#### Phase 8 — 物理ノード分散の配線（Phase 8D 完了）

`dawn-replication`（ADR-0021/0027・Phase 8D）は8D-2〜8D-4を完了済み。
8D-5（Raspberry Pi実機検証）も2026-07-01に完了。8D全項目が完了。

#### Phase 9 — 評価の総点検（決着）

Phase 9時点で総合A−（現在はB+）で決着。新crateは作らない方針（M-3/M-6）、
ADR-0029後の再肥大はR-1で解消済み。

- P9-2（`CelestialBodyDef.sector`）完了。
- P9-1（M-3解消）は撤回（詳細は pending.md 参照 — `SectorSimulatorActor` は本番パス外で
  8D-5はこの境界を経由しないため前提が崩れた）。
