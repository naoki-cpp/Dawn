---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-10-31
---

# Architecture Review — Dawn Codebase（未完項目）

実装詳細と完了条件は各GitHub Issueに置き、この文書は判断と再評価triggerだけを保持する。

## リファクタロードマップ

### R-2（保留）: client `main.gd`追加分割

live state、interaction、presentationは分離済み。残るscene lifecycle / node generation / network send / HUD assemblyは凝集している。
**根本原因:** 残る処理はscene-tree構築とsession lifecycleを共有し、独立した変更境界をまだ持たないため。
**判断: Defer。** pass-through surfaceを増やさず、下記triggerが発火するまで現在のcomposition rootを維持する。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

### R-3（保留・warp trigger）: `node/`系ファイルの再肥大

2026-08-05、`node/commands.rs` では flight / module / station / loadout-refresh という
独立した変更理由が一つの入口に混在していたため trigger が発火した。issue #264 で、外側の
網羅的な family 選択と follow-up 射影だけを `commands.rs` に残し、各 policy を
`command_flight.rs` / `command_module.rs` / `command_loadout.rs` / `command_station.rs` へ
移した。wire shape、domain result、event semantics は変更していない（ADR-0047 amendment）。

`node/warp.rs`は1281行だがproduction実装は592行で、geometry kernelとstate machineが凝集している。
**判断: Defer。** テストを除く実装が約700行を超え、かつ独立した変更理由が混在する、または
module間のdriftが実害になるまで分割しない。行数だけでは発火させない。

### ~~R-9（#346）~~ 解消済み

PR #369をマージ済み、Issue #346はclosed。実装記録は[server-completed.md](./server-completed.md)を参照。

### R-10（Fix）: Godotテスト環境準備の分散

**根本原因:** `setup-godot.ps1`/`.sh`はGodot取得・互換patch・importを持つ一方、
GdUnit取得は`install-gdunit.sh`、DLL buildはCI/利用者の別手順であり、準備順序をcallerが所有する。
両setupに同じpatch意図も重複する。DLL欠損時はimport後に多数の未定義classへ波及する。
**判断: Fix。** 既存platform adapterを維持し、前提検証と順序を一つの準備contractへ集約する。
patchの正本を共有し、fresh/repeat setup、DLL欠損、addon版違い、CRLF/LF、import失敗を検証する。
新crateや委譲だけのmanagerは作らない。新しいGitHub Issueはまだ起票していない。

### R-11（Defer）: Market決済のframe間bookkeeping

**根本原因:** page進行とroutingは集約済みだが、`serve/single.rs`と`serve/runtime.rs`は
queued input、commit outcome、ledger ack、次frameのguard retirementをそれぞれ接続する。
**判断: Defer。** 現在のretry/guard/shared-page testsで保証され、正しさの欠陥は確認していない。
**再評価:** 第三のserve mode追加、二つのmodeでack/retirement順序がずれる回帰、または同じ
bookkeeping修正が両方で再発した場合。抽出時もMarket/Sector authorityとdurable-before-ackは維持する。

### その他の黄判定（Defer）

| 対象 | 根本原因・現在の判断 | 再評価trigger |
|---|---|---|
| `repositories/identity.rs`（681行） | ResumeTicket/allocator/reconciliationは一つのprivate owner。現状維持 | 独立したschema更新が互いに波及、または状態所有者が重複した場合 |
| `peer_transport.rs`（963行） | control/bulk framingと接続lifecycleの同時所有は意図的 | 独立protocol追加、priority isolationの回帰、adapter surfaceの継続増加 |
| `runtime_frame.rs`（1338行） | production部分596行で、一つのdurable frame owner | deployment固有policyや別authorityの混入、または呼び出し側との順序drift |

## 一覧

| 項目 | 状態 |
|---|---|
| R-2 | 保留・trigger付き |
| R-3 | commands slice・transit deepening 完了、warpはtrigger付きで保留 |
| R-9 | #346・PR #369で解消済み |
| R-10 | Godot準備contract・Fix（未起票） |
| R-11 | 決済bookkeeping・Defer（trigger付き） |
採らない方針: CRDT/LWW、protobuf、薄いadapterだけの追加crate、行数削減目的の網羅match・domain型の破壊、初回LAN検証でのTLS/認証。

ADR-0051で、薄いadapterのためではなく二重のcomposition rootを統合する
`dawn-server` packageを採用した。production `sector-node` と local
`simulate` は同じpackage境界にあり、Sectorのtick/reducerは引き続き
`dawn-sector`が所有する。
