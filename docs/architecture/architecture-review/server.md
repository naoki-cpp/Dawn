---
scope    : コードベース全体の保守性・設計品質レビュー — 現行構造評価
audience : AI Agent / Human Developer
update   : 大規模リファクタ実施後 / 新クレート追加時 / architecture issue更新時
related  : AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」, docs/architecture/architecture.md,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）,
           docs/architecture/architecture-review/server-pending.md（未完項目・issue一覧）
date     : 2026-07-29（冗長性レビュー後の部分再計測。全ファイル再計測ではない）
---

# Architecture Review — Dawn Codebase（現行構造評価）

詳細な判断とtriggerは[server-pending.md](./server-pending.md)、完了履歴は
[server-completed.md](./server-completed.md)を参照する。

## 現状評価

**総合: B+。** crate DAGとdeep module境界は健全。直近では、protocol shim削除（#183）、
操船step decisionのpure core化（#190）、snapshot read/write seam（#194）、
ClientCommand入口でのactive ship一回解決（#196 / ADR-0047）が完了した。

2026-07-29の調査では、行数よりも**同じ状態・projection・authorityの二重所有**を優先課題とした。

| 観点 | 評価 | 現在の判断 |
|---|---|---|
| クレート構成 | A− | `dawn-core` / `dawn-sector` / `dawn-wire` / client 2 crateのDAGは健全。共有runtime crateは不要 |
| 型設計 | A− | domain固有のResult/Outcomeを維持。dispatcher都合で共通型へ潰さない（ADR-0047） |
| 重複 | B+ | 重要な残件はlive/replay materialization（#197/#198）とGalaxy projection（#199） |
| 永続化 | B+ | snapshot seamは改善。post-snapshot tail replayとの同値性を#197で固定する |
| Rust固有 | A− | 網羅matchとexhaustive destructuringを変更検出器として利用 |

## 冗長性

解消済み:

- protocol、ClientCommand dispatch、AoI、data loader、NPC spawn loop
- fitting再計算と`ShipFitted` emission tail
- postcard encode/decode
- snapshot constructor/read field list

Open:

1. **#197 P0** Ship生成のlive/replay materialization統一
2. **#198 P1** Station runtime-state applyのlive/replay共有
3. **#199 P1** `SectorMap` projectionの一元化

`ClientCommand`外側matchと`StationDispatchCommand`、domain固有の戻り値、process model固有の薄いadapterは
意図的に維持する。

## ファイルサイズ（部分再計測）

| ファイル | 行数 | 判定 |
|---|---:|---|
| `crates/dawn-sector/src/node/commands.rs` | 1544 | 🟡 網羅dispatcher・module command・tests |
| `crates/dawn-sector/src/node/command_station.rs` | 264 | 🟢 station family private dispatch |
| `crates/dawn-sector/src/node/snapshot_io.rs` | 865 | 🟡 snapshot/checkpointと復旧tests |
| `client/scripts/main.gd` | 1338 | client側orchestration |

全体表は2026-07-24計測を基準とし、次回の定期reviewで更新する。総行数だけでは分割せず、
実装部分約700行超、独立した変更理由の混在、またはdriftの実害をtriggerとする。
