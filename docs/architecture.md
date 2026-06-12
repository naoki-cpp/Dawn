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
- 設計判断の根拠は **docs/adr/** を参照すること
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
| `docs/game-design.md` | なぜその機能を作るか / EVE からの教訓・将来機能候補 |
| `docs/adr/` | なぜそう決めたか（変更不可の判断記録） |

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
ノード間通信 : In-Memory Channel のみ（物理分散なし）
クライアント接続 : WebSocket + JSON（Godot ⇔ WsServer、ADR-0007）
ノード   : 論理的な概念として存在（物理分散なし）
ノード間ネットワーク: 不使用（gRPC / QUIC / Raft / CRDT は将来フェーズ）
```

現在の制約は「できないから」ではなく「今は必要ないから」である。
→ 詳細は [ADR-0003](./adr/ADR-0003-local-first-development.md) を参照。

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
| `dawn-consensus` | ライブラリ | Raft 実装（Leader 選出 / Log Replication / RaftActor、ADR-0014） |
| `dawn-actor` | ライブラリ | Actor 基盤（EventStoreActor / ReplicationBus / ClientConnection trait） |
| `dawn-simulation` | バイナリ | 全体を結合するシミュレーション実行基盤・WsServer（Godot 接続） |

### 依存 DAG

```
dawn-core
    ↑
    ├── dawn-ecs
    ├── dawn-consensus
    └── dawn-event-store
            ↑
            └── dawn-actor
                    ↑
                    └── dawn-simulation  (バイナリ・dawn-consensus にも依存)
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

## 5-A. ClientConnection 抽象化（Phase 4 で実装）

クライアント（Godot）とサーバー（Rust）の接続を trait で抽象化する。
実装を差し替えることでネットワーク化時に Godot 側のコードを変更しない。

```
Phase 4（テスト用）:            Phase 5 以降（現在）:
  InProcessConnection             WsClientConnection
  ↓ In-Memory Channel で直結      ↓ WebSocket + JSON（ADR-0007）

  どちらも同じ ClientConnection trait を実装する

※ gRPC への移行は行わないことを ADR-0007 で決定済み。
  再検討するとしても Phase 9 以降（分散ノード間通信が必要になったとき）。
```

### trait の責務（この 2 方向のみ）

```
サーバー → クライアント : DomainEvent のストリーム配信
クライアント → サーバー : Command の送信
```

これ以外の責務をこの trait に混入してはならない。
接続状態管理・認証・再接続は上位レイヤーが担う。

### データフロー（Phase 4 以降）

```
SectorSimulatorActor
    ↓ events
ReplicationBus
    ↓
ClientConnection（InProcess / WebSocket）
    ↓ DomainEvent stream
Godot クライアント（GDScript）
    ↑ Command
```

→ 詳細設計は ADR-0005（trait）/ ADR-0007（WebSocket セッション）を参照

---

## 5-B. ゲーム化に向けた設計方向性（未実装・設計のみ）

現在の技術基盤をEVEライクな3Dゲームに育てるために、
以下の概念を**今から設計の前提として持つ**。実装はPhaseに従う。

### Interest Management（観測範囲）

最重要。これなしでは実際のゲームにならない。

```
問題: 10万隻が存在する場合、全Eventを全クライアントに送ることは不可能
解法: 各クライアントは「自分の周囲 R km 以内のエンティティ」の
     Eventのみを受信する（Bubble / Area of Interest）

              World
           ┌──────────────┐
           │  C           │
           │     ┌──────┐ │
           │  A  │[you] │ │  ← Bubble内のA,Bのみ受信
           │     │  B   │ │     Cは受信しない
           │     └──────┘ │
           └──────────────┘
```

**設計への影響（将来）：**
- Event は EventStore に書いた後、Bubble フィルタリングを経てクライアントへ配信する
- Bubble の計算には空間インデックス（Sector内の3D近傍クエリ）が必要
- クライアントが移動するにつれて Bubble が更新される

### Projection / Read Model 層

現在のCQRSはWrite側のみ設計済み。Read側を明示化する。

```
Write側（現在実装済み）:
  Command → Validation → Event → EventStore

Read側（将来実装）:
  EventStore → Projection → Read Model
                                ├── SpatialIndex（近傍クエリ）
                                ├── ShipStateView（Ship現在状態）
                                └── SectorOccupancyView（Sector人口）
```

Projectionは**EventのReplayで再構築できる**（INV-002の延長）。
Read Modelが破損しても、EventLogから再生成できる。

### クライアント接続モデル

```
Server（Authoritative）        Client（表示）
─────────────────────          ──────────────
真の状態を持つ                   表示用の状態を持つ
     │                               │
     │ ① Commandを受信               │ Client-Side Prediction
     │ ② 検証・Event生成             │ （レイテンシを隠すための先読み）
     │ ③ EventをClientへ配信    →    │ Reconciliation
     │                               │ （Eventで先読みを補正）
```

ServerはAuthoritative（現在の設計のまま）。
Clientは「仮の状態」を先行表示し、Serverからのイベントで補正する。

### Bounded Context 拡張順序

```
現在実装済み:
  Spatial + Movement + Combat（Fitting / Lock-on / Capacitor 含む）
  Navigation（Jump Gate / 星系間移動、ADR-0009・Phase 7.5 実装済み）

推奨追加順序（依存関係による）:
  Resource    ← 資源（放置型採掘は FBD-009 で禁止。争奪型のみ検討）
      ↓
  Economy     ← Market / Trade / Manufacturing
      ↓
  Social      ← Corporation / Alliance / Chat

原則: 上位ContextはSpatialを使うが、SpatialはContextを知らない
     （依存は常に下向き）
```

---

## 6. 現在の制約（変更には ADR が必要）

| 制約 | 理由 | 根拠 ADR |
|---|---|---|
| Single Process のみ | ドメインロジックの正しさを先に確立する | [ADR-0003](./adr/ADR-0003-local-first-development.md) |
| ノード間ネットワーク不使用 | In-Memory Channel で十分な段階 | [ADR-0003](./adr/ADR-0003-local-first-development.md) |
| エンティティは Ship のみ（Fitting / Combat / Capacitor 含む） | MVP スコープの制限 | [CLAUDE.md §1](../CLAUDE.md) |

---

## 7. このドキュメントの更新ルール

以下の場合のみ更新する。それ以外は専用ドキュメントを更新すること。

- クレートの追加・削除
- 主要概念の追加
- フェーズの進行によるスコープ変更
