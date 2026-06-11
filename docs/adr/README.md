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
| [ADR-0014](ADR-0014-raft-consensus.md) | 分散コンセンサス — Raft による Sector Transit の整合性保証 | Proposed |

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
| [ADR-0009](ADR-0009-star-system-navigation.md) | 星系間ナビゲーション — StarSystem / JumpGate 設計 | Deferred |

### Combat / Fitting

| ADR | タイトル | ステータス |
|---|---|---|
| [ADR-0006](ADR-0006-fitting-and-combat.md) | Fitting / Combat / Lock-on / Active モジュールシステムの設計 | Accepted |
| [ADR-0010](ADR-0010-ship-loss-and-redispatch.md) | 船の喪失と再出撃 — 脱出ポッド / 拠点帰還 / 新艦派遣 | Deferred |
| [ADR-0011](ADR-0011-capacitor-system.md) | サイクルベース Capacitor システムとクライアント側シミュレーション | Accepted |
| [ADR-0012](ADR-0012-turret-tracking.md) | タレット追跡メカニズム | Accepted |

### UI / クライアント表示

| ADR | タイトル | ステータス |
|---|---|---|
| [ADR-0013](ADR-0013-tactical-overlay.md) | タクティカルオーバーレイ（射程リング） | Accepted |

## 参照

- ADRの作成・更新ルール: CLAUDE.md §9 AI Change Checklist
