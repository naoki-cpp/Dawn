---
scope    : 「誰が何を管理するか」の完全な規則。所有権・状態遷移・責任範囲
audience : AI Agent / Human Developer
update   : Actor 構成が変わったとき / Sector 管理ルールが変わったとき
related  : entity-model.md, event-catalog.md, CLAUDE.md §5
---

> **実装状況の注意**
>
> このドキュメントは将来フェーズを含む設計全体を記述している。
> 現在（Phase 7 完了時点）実装済みの範囲は以下のとおり。
>
> | セクション | 内容 | 実装状況 |
> |---|---|---|
> | §1〜2 | Ship の所有権・基本状態遷移 | ✅ 実装済み |
> | §2 Sector Transit / §3 Node 障害時 | Sector 間移動の排他制御・Raft フェイルオーバー | ✅ Phase 7 実装済み（ADR-0014） |
> | §4 Actor の所有権 | Actor 間のデータ分離 | ✅ Phase 2 実装済み |
> | §5 ID 生成 | NodeId + 単調増加カウンタ | ✅ 実装済み |
>
> Sector Transit は必ず Raft を経由すること（CLAUDE.md FBD-006）。

# Ownership Rules

## 1. 所有権とは何か

このシステムにおける「所有権」とは、**あるエンティティの状態変更を行う権限を持つ主体が一意に定まること**を意味する。

### なぜ所有権が必要か

同一エンティティを複数の主体が同時に変更すると、競合が発生し整合性が壊れる。  
現在（Single Process）では技術的に競合を防げるが、  
**将来の分散化に備えて、今から所有権ルールをコードで表現する**。

### 現在（Single Process）と将来（分散）の違い

| | 現在 | 将来 |
|---|---|---|
| 競合の発生 | プロセス内なので理論上ない | ネットワーク遅延・障害で発生する |
| 所有権の強制 | コードの規約として強制 | Raft / CRDT で物理的に強制 |
| 違反時の影響 | テストで検出可能 | 本番データの不整合 |

---

## 2. Ship の所有権

### 基本ルール

```
Ship は必ず 1 つの Sector に所有される。
複数の Sector が同一の Ship を同時に所有してはならない。
```

### 所有権の状態遷移

```
[存在しない]
      │
      │ ShipSpawned { sector_id }
      ▼
[Sector A が所有]  ←──────────────────────────┐
      │                                        │
      │ SectorTransitRequested（未実装）         │
      ▼                                        │
[Transit 中]                                   │
  所有権: Sector A のまま維持                   │
      │                                        │
      │ SectorTransitCompleted（未実装）         │
      ▼                                        │
[Sector B が所有] ──────────────────────────────┘
      │
      │ ShipDespawned（未実装）
      ▼
[存在しない]
```

**Transit 中の状態について（未実装、設計のみ）:**  
Sector 間の移動中は所有権が宙に浮いた状態になるが、
論理的には「元の Sector がまだ所有している」とみなす。
これにより、Transit 完了前に別の操作が割り込むことを防ぐ。

### Transit 中の操作制限（未実装）

Transit 中の Ship に対して以下の操作を受理してはならない。

```
- MoveCommand の受理
- 別の SectorTransit の開始
- ShipDespawn
```

### 所有権の確認責務

| 操作の種類 | 確認責任者 | 現在の実装 |
|---|---|---|
| Sector-local な変更 | Sector Node 自身 | SimulationNode |
| Sector 境界越え | Consensus Layer（未実装） | 未実装 |
| Read（参照のみ） | 確認不要 | — |

---

## 3. Sector の管理責任

### 基本ルール

```
各 Sector は必ず 1 つの Node が管理する。
同一 Sector を複数の Node が同時に管理してはならない。
```

### Sector → Node のマッピング

```
現在: SimulationNode が全 Sector を管理（Single Process）
将来: Consensus Layer が Sector → Node マッピングを管理
```

現在の実装では `SectorId(0)` を `SimulationNode` が直接担当する。  
将来の分散化でこのマッピングが変わっても、`SectorId` の意味は変わらない。

### Node 障害時（将来設計）

現在は考慮不要。将来フェーズで以下を設計する。

```
- Node Crash → Raft によるリーダー再選出
- 新 Node が旧 Node の Sector を引き継ぐ
- Event Log から状態を再構築して引き継ぎ完了
```

---

## 4. Actor の所有権

Actor モデルは Phase 2 で導入済み（`dawn-actor` クレート、ADR-0002）。

### Actor と担当データの対応

| Actor | 所有するデータ | 受け付けるメッセージ |
|---|---|---|
| `SectorSimulatorActor` | ECS World（該当 Sector 分） | `Tick`, `MoveCommand`, `SpawnShip` |
| `EventStoreActor` | Event Log（該当 Sector 分） | `Append`, `IterFrom` |

### Actor 間データ共有の禁止ルール

```
禁止: Arc<Mutex<T>> でデータを共有する
禁止: Actor が別 Actor の内部状態を直接参照する
許可: Mailbox（mpsc チャンネル）経由のメッセージコピーのみ
```

この制約は将来の物理分散化で Actor 間通信が
ネットワーク越しになることを見越した設計である。

---

## 5. ID 生成の所有権

### 基本ルール

```
EntityId の生成は NodeId を持つ Node の排他的権限である。
ID 生成に調整（ロック・コンセンサス）は不要。
```

### なぜ調整不要か

```
EntityId = NodeId（8 bit） + Counter（56 bit）

同一 NodeId 内で Counter が単調増加すれば、
異なる Node 間で Counter が重複しても EntityId は一意になる。
```

例:
```
Node(0), Counter(100) → EntityId: 0x00_00000000000064
Node(1), Counter(100) → EntityId: 0x01_00000000000064  ← 異なる
```

この設計は分散化後も変更不要。

---

## 6. 不変条件一覧（所有権に関するもの）

CLAUDE.md の INV と重複する場合はリンクのみ記載。

| 条件 | 内容 | 参照 |
|---|---|---|
| Ship は常に 1 Sector に帰属 | 複数 Sector による同時所有禁止 | §2 |
| EntityId は再利用しない | Despawn 後も同 ID を割り当てない | [CLAUDE.md INV-004](../CLAUDE.md) |
| Transit 中の Ship への操作制限 | MoveCommand 等を拒否する | §2 |
| Sector は 1 Node が管理 | 複数 Node による同時管理禁止 | §3 |
| Actor はデータを直接共有しない | Mailbox 経由のみ | [CLAUDE.md INV-005](../CLAUDE.md) |
