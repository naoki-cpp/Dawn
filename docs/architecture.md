---
scope    : システム全体の地図。「何が存在し、どう繋がっているか」を俯瞰する
audience : AI Agent / Human Developer
update   : クレート構成が変わったとき / フェーズが進んだとき
related  : entity-model.md, event-catalog.md, ownership.md, tick-model.md, roadmap.md, CLAUDE.md
---

# Dawn Architecture

## 1. このドキュメントの読み方

このファイルはプロジェクトへの**最初の入口**である。
詳細は必ず専用ドキュメントへのリンクを辿ること。このファイルに詳細を書き足さない。

### AIエージェントへの注意

- コードを書く前に **CLAUDE.md** を読むこと（不変条件・禁止事項を含む）
- 設計判断の根拠は **adr/** を参照すること
- 「何を実装すべきか」は **docs/roadmap.md** を参照すること

### ドキュメント責務早見表

| ファイル | 答える問い |
|---|---|
| `CLAUDE.md` | 何をしてはならないか / 変更前に何を確認するか |
| `docs/architecture.md` | 全体はどう構成されているか（このファイル） |
| `docs/entity-model.md` | 何が存在するか（型・フィールド定義） |
| `docs/event-catalog.md` | 何が起きるか（イベント仕様） |
| `docs/ownership.md` | 誰が何を管理するか（所有権・状態遷移） |
| `docs/tick-model.md` | いつ・どの順番で処理されるか |
| `docs/roadmap.md` | 何を・どの順番で作るか |
| `adr/` | なぜそう決めたか（変更不可の判断記録） |

---

## 2. プロジェクトの本質

### 目的

EVE Online に着想を得た**研究用の分散シミュレーション基盤**。
ゲームを作ることが目的ではない。以下の技術的命題を実証するためのプラットフォームである。

- Single Shard における数万エンティティのリアルタイム同期
- Event Sourcing による完全な因果追跡と世界の再現性
- Actor モデルと ECS の責務分離による高スループット処理

### 現在のスコープ（Local-First Phase）

```
動作環境 : Single Process
通信手段 : In-Memory Channel のみ
ノード   : 論理的な概念として存在（物理分散なし）
ネットワーク: 不使用（gRPC / QUIC / Raft / CRDT は将来フェーズ）
```

現在の制約は「できないから」ではなく「今は必要ないから」である。
→ 詳細は [ADR-0003](../adr/ADR-0003-local-first-development.md) を参照。

### 将来のスコープ（方向性のみ、未実装）

- 物理的に分離したノードへの分散
- ネットワーク層（gRPC / QUIC）
- 分散コンセンサス（Raft）
- CRDT による最終一貫性

**将来のスコープを前提にしたコードを現在のフェーズで書かないこと。**

---

## 3. Cargo Workspace 構成

### クレート一覧

| クレート | 種別 | 責務 |
|---|---|---|
| `dawn-core` | ライブラリ | 純粋ドメインモデル定義。外部依存ゼロ |
| `dawn-ecs` | ライブラリ | ECS World ラッパー。Component / System 定義 |
| `dawn-event-store` | ライブラリ | Append-only Event Log の永続化 |
| `dawn-simulation` | バイナリ | 全体を結合するシミュレーション実行基盤 |

### 依存 DAG

```
dawn-core
    ↑
    ├── dawn-ecs
    └── dawn-event-store
                ↑
                └── dawn-simulation  (バイナリ)
```

依存は**下から上への一方向のみ**。逆方向・循環は設計の失敗を意味する。
→ 詳細ルールは [CLAUDE.md §3](../CLAUDE.md) を参照。

### クレートへの依存追加ルール

`dawn-core` に追加してよい依存は以下のみ。追加前に ADR を作成すること。

```
serde / thiserror のみ
ネットワーク・ファイル I/O・非同期ランタイムは禁止
```

---

## 4. 主要概念の定義

各概念の**詳細は専用ドキュメントを参照**すること。ここでは1行定義のみ記載する。

| 概念 | 定義 | 詳細 |
|---|---|---|
| **World** | シミュレーション世界全体。全 Sector の集合 | — |
| **Sector** | 空間的分割単位。Ship エンティティの管理範囲 | [ownership.md](./ownership.md) |
| **Node** | 論理的処理単位。現在は In-Process な概念 | [ownership.md](./ownership.md) |
| **Ship** | 唯一の Entity 種別（MVP） | [entity-model.md](./entity-model.md) |
| **Tick** | 論理時間単位。物理時刻と無関係 | [tick-model.md](./tick-model.md) |
| **Event** | 世界で起きた不変の事実 | [event-catalog.md](./event-catalog.md) |
| **Command** | 変更要求。拒否される可能性がある | [event-catalog.md](./event-catalog.md) |

---

## 5. データフローの概観

```
Command 受信
    │
    ▼
Validation（拒否 → CommandRejected を返す）
    │
    ▼
Domain Logic 実行（ECS World を更新）
    │
    ▼
Event 生成 → EventStore に Append
    │
    ▼
（将来）ノード間 Replication
```

Command と Event は別の型として完全に分離する。
→ フローの詳細は [CLAUDE.md §4](../CLAUDE.md) / イベント仕様は [event-catalog.md](./event-catalog.md) を参照。

---

## 6. 現在の制約（変更には ADR が必要）

| 制約 | 理由 | 根拠 ADR |
|---|---|---|
| Single Process のみ | ドメインロジックの正しさを先に確立する | [ADR-0003](../adr/ADR-0003-local-first-development.md) |
| ネットワーク不使用 | In-Memory Channel で十分な段階 | [ADR-0003](../adr/ADR-0003-local-first-development.md) |
| Ship / Position のみ | MVP スコープの制限 | [CLAUDE.md §1](../CLAUDE.md) |

---

## 7. このドキュメントの更新ルール

以下の場合のみ更新する。それ以外は専用ドキュメントを更新すること。

- クレートの追加・削除
- 主要概念の追加
- フェーズの進行によるスコープ変更
