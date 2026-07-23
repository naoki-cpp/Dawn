---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-07-24
---

# Architecture Review — Dawn Codebase（未完項目）

open な issue（root cause・decision・re-evaluation trigger）と、保留・許容判断の一覧。
**分析のみ。ここに書かれた項目を直すときは `/simplify` か手動リファクタで別PRとして行う。**

ID体系（`M-`/`L-`/`R-`/`P-`）は継続番号。解消した項目は
[server-completed.md](./server-completed.md) へ移動し、
ここでは打ち消し線 + 一行ポインタのみ残す。

---

## 問題一覧

### Medium

#### M-3（優先度低・本番パス外）: `sector_simulator_actor.rs` と `SimulationNode` の密結合

`SectorSimulatorActor` は `SimulationNode` の公開メソッドをほぼ全て呼ぶ薄いラッパーで、
`SimulationNode` の変更が即 Actor に波及する。

**ただし本番パス外。** `SectorSimulatorActor` を使うのは `MultiNodeCluster`
（dawn-simulation のインプロセス・テスト/ベンチ用クラスタ）のみ。本番バイナリ
`dawn-sector-node` は 8D-4 で独自の main ループを持ち、この Actor を使わない。

このため当初の「8D-5 実機検証後に境界の揺れが確定してから着手」という前提は無効化した
（8D-5 が動かすのは dawn-sector-node であり、この Actor を一切経由しない）。
加えて各ハンドラ（Tick / SpawnShip / Transit / Jump …）は「メッセージ → node メソッド → 返信」の
薄いアダプタで、sync な node を async メッセージングへ繋ぐ Actor の性質上ある程度は本質的。
コマンド/応答 enum 化しても本番価値は薄く、インプロセス・クラスタテストを壊すリスクが上回る。

**判断: 保留。** 優先度を下げて保留する。

再評価のトリガー: `SectorSimulatorActor` の main ループと `dawn-sector-node` の main ループの
重複（両者とも tick + Raft + replication を駆動）が保守上の実害になったとき、または
in-process クラスタを本番に近づける必要が出たとき。

P9-1（M-3解消の当初計画）は撤回済み: `SectorSimulatorActor` は本番パス外で8D-5はこの境界を
経由しないため前提が崩れた。

#### M-6（縮小・許容）: 2つの serve バイナリに残る adapter 重複

M-4（WS 境界）、PR #34（dawn-simulation 側 AoI delivery deepening）、
Sector Node runtime deepening、AoI delivery の dawn-sector への集約後も、
両バイナリの「アプリケーション層」adapter/glue は一部重複している:

| 重複 | dawn-simulation | dawn-sector-node | 備考 |
|---|---|---|---|
| ~~Player Command Dispatch~~ | ~~`serve/mod.rs::apply_common_command`~~ | ~~`runtime.rs::collect_player_commands`~~ | 解消済み（M-7・Issue #56）: `node.apply_client_command` に統一。詳細は completed.md |
| `data_loader`（`load_modules` / `load_ship_types` / `parse_*`） | `data_loader/*.rs`（実装 ~280行）| `data_loader.rs`（278行）| TOML ローダー |
| `spawn_npcs` / `spawn_npc_frigates` | `serve/mod.rs:278` | `main.rs:298` | 実質同一（~12行）|

現在の実態では、`dawn-simulation` 側は `serve/runtime.rs` と `serve/aoi_delivery.rs` によって
single/cluster の内部知識をかなり集約済みで、`dawn-sector-node` 側も `runtime.rs` によって
production process model 固有の frame orchestration を集約済みである。問題は「同じ大きな serve loop が
二重化している」ではなく、**2つの process model がそれぞれ adapter を持つ**ことに縮小した。
8D-4 で `dawn-sector-node` を `dawn-simulation` の serve 経路からコピーして作った名残はあるが、
WS protocol は `dawn-actor` に、ゲームロジックは `dawn-sector` に、両 runtime の frame policy は
それぞれのローカル module に寄っており、残る重複は低頻度の glue に縮小している。

`data_loader` / NPC spawn は I/O と demo wiring の低頻度 glue で、共有 crate へ
押し込むほどの深さがない。

**判断: 当面は許容する（新規 crate は作らない）。**

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

#### M-8（許容・2026-07-01）: `fit_module` / `fit_module_owned` の共有テール重複

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

~~M-10~~ 解消済み（2026-07-11）— see completed.md

#### M-9（保留・2026-07-01）: `EventStore::append` がinfallibleと偽る

`/improve-codebase-architecture` の指摘: トレイト `EventStore::append` は `u64` を
返すのみで失敗を表現できないが、`FileEventStore::append`（file.rs:232-240）は
書き込み/flush失敗時に `.expect()` で panic する。tickのホットパス上にあるため、
ディスクフル等が起きるとSectorプロセス全体が落ちる。

調査の結果、この経路は2026-07-01の永続化配線まで本番で到達不可能だった
（`dawn-sector-node` は `InMemoryEventStore` のみで稼働していたため）。
配線完了により実際に到達可能になった。

**判断: 保留（トリガー付き）。** トレイトを `Result` 化する案は、戻り値を使う
6箇所以上の `apply_*_command` の戻り値型変更（`bool` → `Result<bool, _>`）に波及し、
かつ「tick処理中に一部のイベントだけappend失敗する」状態はINV-005（tick決定性）的に
中途半端な復旧ができない。1 Sector = 1 プロセス（8D-4）構成では panic = そのプロセスのみ
クラッシュし、再起動時にスナップショット+ホットログから復旧する設計（ADR-0017）なので、
crash-only としての panic 自体は不合理ではない。8D最小化方針に照らし、全面 `Result` 化より
panic メッセージの充実化・意図の明文化（トレイトdocコメントへの追記）の方が費用対効果が
高いと判断し保留する。

再評価トリガー: 実機運用でディスクフルによる予期しないクラッシュが実際に発生したとき、
または `dawn-sector-node` がマルチSector・マルチスレッド構成に変わり panic の影響範囲が
1Sectorを超えるようになったとき。

---

## リファクタロードマップ（2026-06-23 追加・ADR-0029 後の再計測で起票）

機能追加（ADR-0029）で再び閾値を超えたファイルの分割を、過去の P7 系（`transit_flow.rs` /
`tackle.rs` / `snapshot_io.rs` を `node/mod.rs` から切り出した）と同じ「責務ごとに sibling
モジュールへ抽出、テストも実装と同じファイルへ」方式で行う。挙動は変えない（純粋な移動）。

#### R-2（一部着手済み）: クライアント `main.gd` 1127 行

ADR-0029 以降に増加した `main.gd` は、`WorldSession` 抽出に続いて 2026-07-05 に
`WorldInteraction` を抽出し、1165→1127 に縮小。InitialState / AoI / HP / lock / tick-cap の
live world state は `client/scripts/world_session.gd`、selection state / double-click /
click→intent は `client/scripts/world_interaction.gd` へ移動済み。残りは scene lifecycle /
scene node generation / network send / HUD adapter のオーケストレーション層。さらなる分割は `.tscn` 化コンポーネントへの
シーン参照切れリスクが上回るため引き続き保留（詳細は client レビューの pending 参照）。

#### R-3（低優先・トリガー保留）: `node/` 系ファイルの再肥大（ADR-0031/0032/0033 後）

2026-07-24 の再計測では `commands.rs` 1459・`inventory.rs` 931・`ship_cargo.rs` 573・
`warp.rs` 1088・`order_book.rs` 1140・`transit_flow.rs` 940・`apply_event.rs` 860・
`node/mod.rs` 854・`orbit.rs` 836・`snapshot_io.rs` 702 がwatch帯に残った。
Player movement commandは今回再整理され、
`movement_commands.rs`へMove/Stop、docked/transit/warp gating、共有推進ヘルパーを集約した。
`commands.rs`はルーター・所有権アクセサ・残りのcommand validationに縮小した。Station operationsはPR #149で再整理され、
`station_operation_execution.rs` 281へaccepted-operationの副作用を集約、`station_materialization.rs`
は645、`station_lifecycle.rs`は410へ縮小したため、Stationの実装はR-3の観察対象から外す。
さらにship cargo ownershipを`ship_cargo.rs`へ分離し、`inventory.rs`は931行のFit/Unfit/Reorder検証へ縮小した。
`spawner_logic.rs` は `/improve-codebase-architecture` 候補3（PR #69）で `process_bots`（Bot AI 決定ループ）を
`node/bot_ai.rs` へ抽出済みで、下記トリガー一覧から外れたまま。R-1（navigation.rs 分割）後に積まれた
Orbit/KeepAtRange（ADR-0031）・Inventory（ADR-0032）・Repair（ADR-0033）・Station（ADR-0034/9B）の
累積に加え、テストと機能追加が総行数を押し上げ続けている。
**watch対象のimplはいずれも700行未満で、単一責務も保たれている**ため、R-3の追加着手トリガーは未発火。
今回のPlayer movement command分割は完了済みとしてcompleted.mdへ移し、R-3の残りは`warp.rs`・`orbit.rs`・`transit_flow.rs`・
`inventory.rs`・`apply_event.rs`・`snapshot_io.rs`・`node/mod.rs`とMarket settlement周辺に限定する。
ship cargo ownershipの次の設計候補は、`dawn-market/src/order_book.rs` とSector側bridgeの
settlement順序を整理するMarket settlement module。ADR-0034の片側Command方針を維持し、新crateは前提にしない。
`mod.rs` は同じ観察対象だったが、2026-07-06 の再計測で impl が700行を超えたため
R-4 として切り出し着手判断へ格上げし、2026-07-07 に完了した（completed.md 参照）。その後
2026-07-17 の再計測で `mod.rs` は854行、implは約443行。R-4と同じ再蓄積は観察対象として残すが、
現時点で追加分割のトリガーには達していない。

**根本原因**: 機能追加のたびに `node/` 直下へ impl + テストが積まれる構造。これ自体は
P7 系で確立した「責務ごとに sibling モジュールへ抽出」方式の想定内の蓄積であり、
設計の破綻ではない。

**判断: 保留（トリガー付き）。** 総行数はまだ大きいが、現時点では単一責務を保っている。
今分割すると純粋移動の差分だけが増え、得が薄い。

再評価トリガー（いずれかで着手）:
- いずれかの **impl 部分**（テスト除く）が ~700 行を超えたとき。
  - `warp.rs` → `process_warp` / Hermite warp 幾何 / コマンド・drain に3分割。
  - `orbit.rs` → Orbit / KeepAtRange の共有幾何と command application を分離。
  - `commands.rs` → command dispatch とバリデーション本体、肥大化した test cluster を分離。
  - `transit_flow.rs` → Request 側と Commit 側のハンドラを分離。
- または `node/` のファイル総数が増えて「どこに何があるか」の見通しが実際に悪化したとき。

---

## 未完了・保留 一覧

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| R-2 client `main.gd` 分割 | 品質・一部着手済み | `WorldSession`・`WorldInteraction`・`WorldPresentation` 抽出で live world state / world interaction policy / world visual side effect を移動し、`main.gd` は 1219 行（詳細・最新値は client.md）。残る scene lifecycle / node generation / network send / HUD adapter は `.tscn` 化コンポーネントへのシーン参照切れリスクが上回るため保留 |
| R-3 `node/` 系再肥大（warp/orbit/transit_flow/inventory/apply_event/snapshot_io/mod） | 品質・保留 | 2026-07-24再計測で `commands.rs` は1459行、Move/Stopと共有推進ヘルパーは`movement_commands.rs`へ分離済み。残るwatch帯は `inventory.rs` 931・`warp.rs` 1088・`transit_flow.rs` 940・`apply_event.rs` 860・`node/mod.rs` 854・`orbit.rs` 836・`snapshot_io.rs` 702。各implは700行未満で責務単位も保たれるため保留し、次候補はMarket settlement。implが700超、またはtest clusterを含む見通し悪化が実害化した時点で分割 |
| M-3 `SectorSimulatorActor` 密結合 | 品質・保留 | 本番パス外（in-process テスト/ベンチ専用）。P9-1 撤回。優先度低 |
| M-6 アプリ層 adapter 重複（`data_loader` / `spawn_npcs`） | 許容重複（縮小） | AoI / production runtime / Command dispatch は deep module 化済み（M-7 解消で Command dispatch 項目を削除）。残る data_loader / NPC spawn は低頻度 glue として許容。再評価トリガー付き |
| M-8 `fit_module`/`fit_module_owned` 共有テール重複 | 許容（2026-07-01） | `inventory.rs` のモジュールコメントで意図的な分離と明記済み。テールのみの軽微な重複で優先度なし |
| M-9 `EventStore::append` がinfallibleと偽る | 品質・保留（2026-07-01） | 永続化配線完了で実際に到達可能になったpanic経路。1プロセス1Sector構成ではcrash-only設計として不合理ではないため、全面Result化は見送り保留。実機クラッシュ発生 or マルチSectorプロセス化がトリガー |
| ~~M-10~~ postcard encode/decode の呼び出し側分散 | 解消済み | 2026-07-11解消。詳細は completed.md 参照 |

採らない方針（恒久）:

- CRDT / LWW-Register は採らない（単一所有 + append-only log gossip）
- protobuf / `dawn-proto` は採らない（wire は postcard 再利用）
- TLS / 認証は第1次 LAN 検証では扱わない
