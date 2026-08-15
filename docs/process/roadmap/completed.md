---
scope    : 完了済みロードマップ項目の短い記録
audience : AI Agent / Human Developer
update   : フェーズまたは大きなマイルストーンの完了時
related  : ./README.md, ../roadmap-history.md, ../../adr/
---

# Roadmap — Completed

完了済みの詳細を常時読む必要はない。過去の判断・計測値は
[roadmap-history.md](../roadmap-history.md)、ADR、各architecture documentへ分散している。

## 4. 完了済み（短縮）

### 基盤と分散実行

- Phase 0〜3: workspace、Single Node、Multi-Node、Snapshot / Replay 完了。
- Phase 4〜7.5: マルチプレイヤー、ゲームループ、Raft、Jump Gate 完了。
- 戦闘の基礎: Warp、Propulsion、Tackle、Signature、Orbit、Keep at Range、Local Repair、Remote Repair / Logistics 完了。
- Phase 8A / 8D: 永続化、Replication、物理ノード、ネットワークRaft、Piクラスタ検証完了。

### Phase 9 の基盤

- 9A: `ItemId` 一般化、Scrap Metalの撃破者加算、Snapshot対応完了。
- 9B: NPC Station、Dock/Undock、Station-local inventory、Assemble / Disassemble、Packaged Ship建造、GodotのStation UI完了。Station inventoryはSQLite projection（SimulationNode内cacheなし）。
- 9D: `dawn-market`、SQLite order book、Currency台帳、エスクロー、Station限定Market UI、純粋matching policy、SettlementIntent outboxとstable ID配送（#279）完了。
- 9E: 9E-1のプレイテスト以外は、Phase 9基盤の自動検証を完了。

### Phase 10 の基盤

- GDExtension crate、`dawn-client-core`、PlayerLoadout projection、WorldSession純粋状態、Command送信、postcardの主要移行、`ShipMotion`のcommand/frame境界完了（ADR-0045/0046）。
- 10-2/10-4: `InitialState` / `PlayerLoadout` / `AoiEnter` 固定型移行とpostcard WebSocket移行の段階2を完了（ADR-0042）。`register_ship`のJSON往復排除（#178）、`connection.gd`のdead text-frame fallback削除（#179）。
- 10-6: 絶対座標・floating origin・client motionの再評価と `ShipMotion` への統合完了。Godot editorでの実機確認は pending に残す。
- Godotの自動テストは通過。実行DLL生成と手動プレイテストは [pending.md](./pending.md) に残す。
- Phase 11 celestial presentation: topology-derived fresh spawn, physical body radii,
  synchronized star lighting, deterministic planet surfaces, and bright-star sky
  landmarks (ADR-0053).

## 記録先

完了項目の詳細な経緯は、既存の [roadmap-history.md](../roadmap-history.md) とGit履歴を参照する。
完了済み項目を再実装候補として扱わず、変更が必要な場合は新しいTODOまたはADRを起票する。

- #282: Workspace boundaries consolidated. `dawn-protocol`, `dawn-storage`, and `dawn-distributed` are the final protocol/storage/distributed packages, obsolete package names are deleted, and `dawn-server` is the only server composition root.
