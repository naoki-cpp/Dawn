---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-08-11
---

# Architecture Review — Dawn Codebase（未完項目）

実装詳細と完了条件は各GitHub Issueに置き、この文書は判断と再評価triggerだけを保持する。

## Medium

### M-9（保留）: `EventStore::append`のinfallible contract

`FileEventStore`はwrite/flush失敗時にpanicするが、1 Sector = 1 processのcrash-only recoveryと整合する。
**再評価:** disk-full crashが運用問題になるか、1 processが複数Sectorを所有する場合。

## リファクタロードマップ

### R-2（保留）: client `main.gd`追加分割

live state、interaction、presentationは分離済み。残るscene lifecycle / node generation / network send / HUD assemblyは凝集している。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

### R-3（保留・warp trigger）: `node/`系ファイルの再肥大

2026-08-05、`node/commands.rs` では flight / module / station / loadout-refresh という
独立した変更理由が一つの入口に混在していたため trigger が発火した。issue #264 で、外側の
網羅的な family 選択と follow-up 射影だけを `commands.rs` に残し、各 policy を
`command_flight.rs` / `command_module.rs` / `command_loadout.rs` / `command_station.rs` へ
移した。wire shape、domain result、event semantics は変更していない（ADR-0047 amendment）。

`node/warp.rs` は1203行だが実装は573行で、geometry kernelとstate machineが凝集している。
**判断: Defer。** テストを除く実装が約700行を超え、かつ独立した変更理由が混在する、または
module間のdriftが実害になるまで分割しない。行数だけでは発火させない。

### R-6（Fix候補）: `RuntimeFrameHost`のFrameInput境界

`RuntimeFrameHost`はproduction / single-sector / cluster / in-processのフレーム実行を一つに
集約できたが、admission、Market、jump fallback、fixture spawnの入口は現在も
`with_node_mut` / `RuntimeNodeMutation`によるclosure-scoped mutation bridgeである。
**根本原因:** runtime frameへ入力を渡すtypedな`FrameInput` surfaceがまだなく、composition adapterが
`SimulationNode`のmutation APIを直接選んでいるため。
**判断: Fix。** live production mutationをprepare→durable→applyへ入れるtyped inputと、commit後の
typed outputへ移し、closure bridgeはbootstrap/fixture専用に縮小する。frame外のmutationが増える、
またはack前のmutation順序を検証できない実装が現れたらこの作業を優先する。

### R-7（Fix候補）: `SectorRepository`のbounded-context分割

`node/repositories.rs`は2104行で、Admission、Identity/ResumeTicket、Station projectionのschema、
typed codec、allocator、transaction boundary、全ての回帰testsを一つのfileに保持している。
`SectorRepository`が一つのSQLite connectionとtransactionを所有する設計自体は正しいが、domainごとの
変更理由が同じmoduleへ蓄積している。**根本原因:** shared connection boundaryと各repository viewの
実装ファイル境界が一致していないため。**判断: Fix。** connection/transactionの薄い共通境界を
維持したまま、admission、identity、station projectionの実装とtestsをmoduleへ分ける。
別SQLite connectionを導入したり、Station authorityをSQLiteへ戻したりはしない。

## 一覧

| 項目 | 状態 |
|---|---|
| R-2 | 保留・trigger付き |
| R-3 | commands slice・transit deepening 完了、warpはtrigger付きで保留 |
| R-6 | Fix候補・FrameInput境界 |
| R-7 | Fix候補・repository bounded-context分割 |
| M-9 | 保留・trigger付き |

採らない方針: CRDT/LWW、protobuf、薄いadapterだけの追加crate、行数削減目的の網羅match・domain型の破壊、初回LAN検証でのTLS/認証。

ADR-0051で、薄いadapterのためではなく二重のcomposition rootを統合する
`dawn-server` packageを採用した。production `sector-node` と local
`simulate` は同じpackage境界にあり、Sectorのtick/reducerは引き続き
`dawn-sector`が所有する。
