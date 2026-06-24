# ADR Index

ADR (Architecture Decision Record) の一覧。番号は採番順（決定の時系列）であり、
カテゴリ分けはこのインデックスのみで行う。ファイルの移動・リネームは行わない
（既存リンク・CLAUDE.md からの参照を壊さないため）。

新しい ADR を追加したら、このインデックスにも追記すること。

## カテゴリ別一覧

### Architecture / 基盤方針

| ADR | タイトル | ステータス |
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

### Client / 通信

| ADR | タイトル | ステータス |
|---|---|---|
| [ADR-0004](ADR-0004-client-technology.md) | クライアント技術選択（Godot 4 + godot-rust） | Accepted |
| [ADR-0005](ADR-0005-client-connection.md) | ClientConnection — サーバー／クライアント通信の抽象化 | Accepted |
| [ADR-0007](ADR-0007-multiplayer-session.md) | マルチプレイヤー対応設計（Phase 5） | Accepted |

### Movement / Navigation

| ADR | タイトル | ステータス |
|---|---|---|
| [ADR-0008](ADR-0008-ship-movement-events.md) | 移動イベントの権威的設計：VelocityChanged | Accepted |
| [ADR-0009](ADR-0009-star-system-navigation.md) | 星系間ナビゲーション — StarSystem / JumpGate 設計 | Accepted |
| [ADR-0015](ADR-0015-approach-piloting.md) | アプローチ（半自動操船） | Accepted |
| [ADR-0031](ADR-0031-orbit-keep-at-range.md) | Orbit / Keep at Range — 距離を能動的に管理する持続的操船 | Accepted |

### Combat / Fitting

| ADR | タイトル | ステータス |
|---|---|---|
| [ADR-0006](ADR-0006-fitting-and-combat.md) | Fitting / Combat / Lock-on / Active モジュールシステムの設計 | Accepted |
| [ADR-0010](ADR-0010-ship-loss-and-redispatch.md) | 船の喪失と再出撃 — 脱出ポッド / 拠点帰還 / 新艦派遣 | Deferred |
| [ADR-0011](ADR-0011-capacitor-system.md) | サイクルベース Capacitor システムとクライアント側シミュレーション | Accepted |
| [ADR-0012](ADR-0012-turret-tracking.md) | タレット追跡メカニズム | Accepted |
| [ADR-0032](ADR-0032-inventory-and-runtime-fitting.md) | インベントリとランタイム換装 — InventoryComp / Fit/UnfitModuleCommand | Accepted |
| [ADR-0033](ADR-0033-local-repair-module.md) | ローカルリペアモジュール — アクティブ自己修理 / RepairApplied / Repair System | Proposed |

### UI / クライアント表示

| ADR | タイトル | ステータス |
|---|---|---|
| [ADR-0013](ADR-0013-tactical-overlay.md) | タクティカルオーバーレイ（射程リング） | Accepted |

## 参照

- ADRの作成・更新ルール: CLAUDE.md §9 AI Change Checklist
