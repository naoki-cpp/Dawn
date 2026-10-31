---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-09-01
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

### R-9（#346）: 実装済み・マージ待ち

FileJournalの読み出しは1バッチずつ検証するiteratorへ、圧縮とarchive重複比較は
固定サイズの範囲コピーへ変更済み。保存形式・復旧保証は維持する。
実装記録は[server-completed.md](./server-completed.md)を参照。Issueはマージ後に閉じる。

## 一覧

| 項目 | 状態 |
|---|---|
| R-2 | 保留・trigger付き |
| R-3 | commands slice・transit deepening 完了、warpはtrigger付きで保留 |
| R-9 | #346・実装済み、マージ待ち |
採らない方針: CRDT/LWW、protobuf、薄いadapterだけの追加crate、行数削減目的の網羅match・domain型の破壊、初回LAN検証でのTLS/認証。

ADR-0051で、薄いadapterのためではなく二重のcomposition rootを統合する
`dawn-server` packageを採用した。production `sector-node` と local
`simulate` は同じpackage境界にあり、Sectorのtick/reducerは引き続き
`dawn-sector`が所有する。
