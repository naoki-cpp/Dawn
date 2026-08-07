# ADR Index

List of all ADRs (Architecture Decision Records). Numbers are chronological
by decision; categorization happens only in this index. Never move or rename
ADR files (it would break existing links and references from
AI_DEVELOPMENT_GUIDE.md).

When adding a new ADR, register it here too (the `/new-adr` skill does both).

## By category

### Architecture / Foundations

| ADR | Title | Status |
|---|---|---|
| [ADR-0001](ADR-0001-event-sourcing.md) | Event Sourcing の採用 | Accepted |
| [ADR-0002](ADR-0002-actor-model.md) | Actor モデルによるノード内並行制御 | Accepted |
| [ADR-0003](ADR-0003-local-first-development.md) | Local-First Development（段階的分散化） | Accepted |
| [ADR-0014](ADR-0014-raft-consensus.md) | 分散コンセンサス — Raft による Sector Transit の整合性保証 | Accepted |
| [ADR-0016](ADR-0016-game-vision.md) | プロジェクトの再定義 — 「EVE を超えるゲーム」をゴールに据える | Accepted |
| [ADR-0017](ADR-0017-snapshot-compaction.md) | イベントログのスナップショット圧縮と2層ログ（INV-002 改訂） | Accepted |
| [ADR-0018](ADR-0018-tidi-graceful-degradation.md) | Time Dilation を境界つき局所的最終手段として採用（INV-TiDi 改訂） | Accepted |
| [ADR-0019](ADR-0019-spatial-index-and-aoi.md) | AoI のための静的セルグリッド（3×3×3 隣接可視） | Accepted |
| [ADR-0020](ADR-0020-simulation-lod.md) | Simulation LoD — 非交戦エンティティの休眠化（近似ゼロ・交差閉包） | Deferred |
| [ADR-0021](ADR-0021-sector-local-replication.md) | Sector-local 複製は単一所有 + 追記ログのゴシップ配布（CRDT/LWW は採らない） | Accepted |
| [ADR-0026](ADR-0026-dawn-sector-crate.md) | dawn-sector クレートの新設（ゲームロジックを dawn-simulation から分離） | Accepted |
| [ADR-0027](ADR-0027-dawn-replication-crate.md) | dawn-replication クレートの新設（ゴシップ配布 + アンチエントロピー + スナップショット転送） | Accepted |
| [ADR-0028](ADR-0028-large-world-coordinates.md) | 大規模座標系 — 真スケール座標の方式比較（スパイク GO：B＋C2） | Proposed |
| [ADR-0029](ADR-0029-true-scale-coordinates-implementation.md) | 真スケール座標の実装 — アンカー相対 f32（サーバ B）＋ 浮動原点（クライアント C2） | Accepted |
| [ADR-0030](ADR-0030-steering-files-restructure.md) | ステアリング系ファイルの再構成（常時ロード文脈の軽量化 — Hook + ガイド分割） | Accepted |
| [ADR-0044](ADR-0044-absolute-f64-coordinate-authority.md) | サーバー権威座標を絶対 f64 に統一する方針 | Accepted |
| [ADR-0049](ADR-0049-sector-recovery-state-delta-wal.md) | Exact Sector recovery with a versioned state-delta journal | Accepted |

### Client / Communication

| ADR | Title | Status |
|---|---|---|
| [ADR-0004](ADR-0004-client-technology.md) | クライアント技術選択（Godot 4 + godot-rust） | Accepted |
| [ADR-0005](ADR-0005-client-connection.md) | ClientConnection — サーバー／クライアント通信の抽象化 | Accepted |
| [ADR-0007](ADR-0007-multiplayer-session.md) | マルチプレイヤー対応設計（Phase 5） | Accepted |
| [ADR-0039](ADR-0039-dawn-client-core-crate.md) | dawn-client-core クレート新設 — Godot非依存クライアントドメインモデルのRust抽出（Phase 1: Loadout） | Accepted |
| [ADR-0040](ADR-0040-dawn-client-gdext-binding.md) | dawn-client-gdext — GDExtension バインディングで dawn-client-core を Godot へ公開 | Accepted |
| [ADR-0041](ADR-0041-dawn-wire-command-send.md) | dawn-wire クレート新設 + コマンド送信の GDExtension 化 | Accepted |
| [ADR-0042](ADR-0042-wire-postcard-protocol.md) | ワイヤプロトコルを WebSocket + postcard バイナリへ移行（段階1: Event/Command） | Accepted |
| [ADR-0048](ADR-0048-resume-ticket-client-admission.md) | Server-issued ResumeTicket for client admission | Accepted |

| [ADR-0046](ADR-0046-world-session-state-ownership.md) | WorldSession pure state ownership in dawn-client-core | Accepted |
| [ADR-0047](ADR-0047-client-command-dispatch-shape.md) | ClientCommand ディスパッチャの明示的 match を維持する | Accepted |

### Movement / Navigation

| ADR | Title | Status |
|---|---|---|
| [ADR-0008](ADR-0008-ship-movement-events.md) | 移動イベントの権威的設計：VelocityChanged | Accepted |
| [ADR-0009](ADR-0009-star-system-navigation.md) | 星系間ナビゲーション — StarSystem / JumpGate 設計 | Accepted |
| [ADR-0015](ADR-0015-approach-piloting.md) | アプローチ（半自動操船） | Accepted |
| [ADR-0031](ADR-0031-orbit-keep-at-range.md) | Orbit / Keep at Range — 距離を能動的に管理する持続的操船 | Accepted |

### Combat / Fitting

| ADR | Title | Status |
|---|---|---|
| [ADR-0006](ADR-0006-fitting-and-combat.md) | Fitting / Combat / Lock-on / Active モジュールシステムの設計 | Accepted |
| [ADR-0010](ADR-0010-ship-loss-and-redispatch.md) | 船の喪失と再出撃 — 脱出ポッド / 拠点帰還 / 新艦派遣 | Deferred |
| [ADR-0011](ADR-0011-capacitor-system.md) | サイクルベース Capacitor システムとクライアント側シミュレーション | Accepted |
| [ADR-0012](ADR-0012-turret-tracking.md) | タレット追跡メカニズム | Accepted |
| [ADR-0032](ADR-0032-inventory-and-runtime-fitting.md) | インベントリとランタイム換装 — InventoryComp / Fit/UnfitModuleCommand | Accepted |
| [ADR-0033](ADR-0033-local-repair-module.md) | ローカルリペアモジュール — アクティブ自己修理 / RepairApplied / Repair System | Accepted |
| [ADR-0035](ADR-0035-per-slot-module-targeting.md) | Per-Slot Module Targeting — Weapon/Tackle/Logistics 共通のモジュール起動/ロック基盤 | Accepted |
| [ADR-0036](ADR-0036-remote-repair.md) | Remote Repair — Logistics (targeted ally repair) | Accepted |

### Economy (Phase 9)

| ADR | Title | Status |
|---|---|---|
| [ADR-0034](ADR-0034-economy-foundations.md) | Economy Foundations — Item一般化 / Packaged Ship / Scrap Metal / Market・DBの境界 | Accepted |
| [ADR-0037](ADR-0037-docked-ship-ownership.md) | Docked Ship Ownership — owned ship / active ship / docked station context を分離する | Accepted |
| [ADR-0038](ADR-0038-station-inventory-sqlite.md) | Station Inventory — SQLite as the durable authority, lazy-loaded in memory | Accepted |

### UI / Client Presentation

| ADR | Title | Status |
|---|---|---|
| [ADR-0013](ADR-0013-tactical-overlay.md) | タクティカルオーバーレイ（射程リング） | Accepted |

### Phase 10 additions

| ADR | Title | Status |
|---|---|---|
| [ADR-0043](ADR-0043-client-side-prediction.md) | Client-Side Prediction and Motion Reconciliation | Proposed |
| [ADR-0045](ADR-0045-unified-client-motion-state.md) | クライアント移動状態と表示積分の統合 | Accepted |

## References

- Creating a new ADR: `/new-adr` skill (`.agents/commands/new-adr.md`)
- Pre-change checks that decide when an ADR is required: `/ai-change-checklist`
  skill (`.agents/commands/ai-change-checklist.md`)
