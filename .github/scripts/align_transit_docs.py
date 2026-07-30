from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    old_count = text.count(old)
    new_count = text.count(new)
    if old_count == 1 and new_count == 0:
        path.write_text(text.replace(old, new, 1), encoding="utf-8")
        return
    if old_count == 0 and new_count == 1:
        return
    raise SystemExit(
        f"{path}: expected exactly one old or new occurrence, "
        f"found old={old_count}, new={new_count}"
    )


replace_once(
    Path("README.md"),
    "- **Docs** — decisions in `docs/adr/`; contributor entry point is `AI_AGENT_BRIEF.md`.\n",
    "- **Docs** — design decisions in `docs/adr/`, architecture in `docs/architecture/`, and development process in `docs/process/`.\n",
)

replace_once(
    Path("crates/dawn-sector/src/node/transit.rs"),
    "    /// anchor, and `entry_pos` alone does not carry the destination anchor\n"
    "    /// identity needed to use it as a raw offset. `entry_pos_abs` is the precise f64\n"
    "    /// Sector-frame arrival point (the destination Gate's `abs_m`, or the\n"
    "    /// origin for a non-Gate Transit); `rebase_after_transit` re-anchors\n"
    "    /// against it (appending the authoritative `AnchorRebased` event, ADR-0029)\n"
    "    /// so the Ship can immediately jump back out (ADR-0009).\n",
    "    /// anchor, and `entry_pos` alone does not carry the destination anchor\n"
    "    /// identity needed to use it as a raw offset. `entry_pos_abs` is the precise f64\n"
    "    /// Sector-frame arrival point (the destination Gate's `abs_m`, or the\n"
    "    /// origin for a non-Gate Transit); `rebase_after_transit` re-anchors\n"
    "    /// against it and returns the authoritative `AnchorRebased` event for\n"
    "    /// `import_transit` to append (ADR-0029), so the Ship can immediately\n"
    "    /// jump back out (ADR-0009).\n",
)

replace_once(
    Path("docs/architecture/architecture-review/server.md"),
    "**総合: B+。** crate DAGとdeep module境界は健全。直近では、protocol shim削除（#183）、\n"
    "操船step decisionのpure core化（#190）、snapshot read/write seam（#194）、\n"
    "ClientCommand入口でのactive ship一回解決（#196 / ADR-0047）が完了した。\n",
    "**総合: B+。** crate DAGとdeep module境界は健全。直近では、live/replayのShip materialization、\n"
    "Station runtime apply、SectorMap projection、client read APIのtyped化、\n"
    "Transit state mutation deepeningが完了した。\n",
)

Path("docs/architecture/architecture-review/server-pending.md").write_text(
    """---
scope    : コードベース全体の保守性・設計品質レビュー — 未完項目・issue一覧
audience : AI Agent / Human Developer
update   : /architecture-review で issue を起票・状態更新するたびに更新
related  : docs/architecture/architecture-review/server.md（構造評価）,
           docs/architecture/architecture-review/server-completed.md（完了済みログ）
date     : 2026-07-30
---

# Architecture Review — Dawn Codebase（未完項目）

実装詳細と完了条件は各GitHub Issueに置き、この文書は判断と再評価triggerだけを保持する。

## Medium

### M-3（保留）: `SectorSimulatorActor`と`SimulationNode`の密結合

本番パス外のin-process test/bench adapterで、handlerもmessage → node method → replyの薄い変換である。
**再評価:** production runtimeとのdriftが不具合化するか、in-process clusterを本番構成へ近づける場合。

### M-9（保留）: `EventStore::append`のinfallible contract

`FileEventStore`はwrite/flush失敗時にpanicするが、1 Sector = 1 processのcrash-only recoveryと整合する。
**再評価:** disk-full crashが運用問題になるか、1 processが複数Sectorを所有する場合。

## リファクタロードマップ

### R-2（保留）: client `main.gd`追加分割

live state、interaction、presentationは分離済み。残るscene lifecycle / node generation / network send / HUD assemblyは凝集している。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

### R-3（保留）: `node/`系ファイルの再肥大

総行数だけでは分割しない。実装部分約700行超、独立した変更理由の混在、またはdriftの実害をtriggerとする。

## 一覧

| 項目 | 状態 |
|---|---|
| R-2 / R-3 | 保留・trigger付き |
| M-3 / M-9 | 保留・trigger付き |

採らない方針: CRDT/LWW、protobuf、薄いadapterのための共有runtime crate、行数削減目的の網羅match・domain型の破壊、初回LAN検証でのTLS/認証。
""",
    encoding="utf-8",
)
