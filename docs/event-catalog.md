---
scope    : 存在する全イベントの完全仕様。「何が起きうるか」の唯一の真実
audience : AI Agent / Human Developer
update   : イベントを追加・変更するたびに必ず更新する
related  : entity-model.md, tick-model.md, CLAUDE.md §7
---

# Event Catalog

## 1. このカタログの使い方

### コードとの同期ルール

`dawn-core/src/events.rs` の定義とこのカタログは**常に一致していなければならない**。
イベントを追加・変更した場合は、コードとカタログを同一 PR で更新すること。

### イベント追加の手順

```
1. このカタログに新しいイベントを追記する
2. dawn-core/src/events.rs に型を追加する
3. 対応する Command が必要なら dawn-core/src/commands.rs にも追加する
4. 単体テストを events.rs 内に書く
5. PR 説明に「変更したイベント一覧」を記載する
```

### 後方互換性ルール

```
許可: 新しいフィールドを Option<T> として追加する
禁止: 既存フィールドを削除する
禁止: 既存フィールドの型を変更する
禁止: 既存フィールドの名前を変更する
禁止: イベント名を変更する（代わりに V2 を新設する）
```

破壊的変更が必要な場合は [Upcaster の手順](#5-upcasterカタログ) に従うこと。

---

## 2. イベント設計の原則

### Command と Event の違い

| | Command | Event |
|---|---|---|
| 意味 | 変更の**要求** | 変更が起きた**事実** |
| 拒否 | される可能性がある | されない（既に起きた） |
| 保存 | しない | Append-only で永続化 |
| ファイル | `commands.rs` | `events.rs` |

Command と Event を同じ型・同じ enum で表現してはならない（INV-006）。

### 全イベントが持つ共通フィールド

Movement 系イベントは必ず `tick: Tick` を持つ。
`tick` を省略したイベントは INV-005 違反として拒否する。

### Optional フィールドの方針

- 最初に定義するフィールドは全て必須（`Option` にしない）
- 後から追加するフィールドは全て `Option<T>` とする
- 最初から `Option` にすることは禁止（意図のない省略を許すため）

---

## 3. イベント一覧

### 3.1 Ship ライフサイクル

| イベント名 | 説明 | 発行者 |
|---|---|---|
| `ShipSpawned` | Ship が世界に出現した | `SimulationNode::spawn_ship()` |
| `ShipDespawned` | Ship が世界から消えた | `SimulationNode`（未実装） |

### 3.2 Movement

| イベント名 | 説明 | 発行者 |
|---|---|---|
| `ShipMoved` | Ship の位置が変化した | `MovementSystem::run()` |

### 3.3 Sector Transit（将来予約）

| イベント名 | 説明 | ステータス |
|---|---|---|
| `SectorTransitRequested` | Sector 境界越えの要求 | 未実装 |
| `SectorTransitCompleted` | Sector 境界越えの完了 | 未実装 |
| `SectorTransitRejected` | Sector 境界越えの拒否 | 未実装 |

### 3.4 System（将来予約）

| イベント名 | 説明 | ステータス |
|---|---|---|
| `TickStarted` | Tick の開始 | 未実装 |
| `TickCompleted` | Tick の完了 | 未実装 |

---

## 4. イベント詳細仕様

### `ShipSpawned`

**説明:** Ship が Sector 内に生成された。

**ペイロード:**

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 生成された Ship の一意な識別子 |
| `sector_id` | `SectorId` | ✓ | 生成先の Sector |
| `initial_position` | `Position` | ✓ | 生成時の座標 |
| `tick` | `Tick` | ✓ | 生成された Tick（Tick::ZERO を含む） |

**不変条件:**
- `ship_id` は世界全体で一意であり、再利用されない（INV-004）
- `initial_position` は `sector_id` の SectorBounds 内に収まる

**発行条件:** `spawn_ship()` が呼ばれ、ECS に Entity が追加された後に発行する。

---

### `ShipMoved`

**説明:** Ship が 1 Tick 内に位置を変化させた。

**ペイロード:**

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 移動した Ship の識別子 |
| `from` | `Position` | ✓ | 移動前の座標 |
| `to` | `Position` | ✓ | 移動後の座標 |
| `tick` | `Tick` | ✓ | 移動が確定した Tick |

**不変条件:**
- `tick` は省略不可（INV-005）
- `from != to`（位置が変化していない Ship はこのイベントを発行しない）
- `to` は当該 Ship の Sector の SectorBounds 内に収まる

**発行条件:** `MovementSystem::run()` が実行され、Ship の位置が実際に変化した場合のみ発行する。速度ゼロの Ship はイベントを発行しない。

---

### `ShipDespawned`

**説明:** Ship が世界から永続的に取り除かれた。

**ペイロード:**

| フィールド | 型 | 必須 | 説明 |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | 消滅した Ship の識別子 |
| `tick` | `Tick` | ✓ | 消滅した Tick |

**不変条件:**
- `ship_id` は以降どのイベントにも登場しない（ID は再利用されない）

**発行条件:** Ship が ECS World から除去される前に発行する（INV-002 保証のため）。

**ステータス:** 現在未実装。型定義のみ存在する。

---

## 5. Upcasterカタログ

破壊的変更があった場合にのみここに記録する。

現時点での破壊的変更: **なし**

### Upcaster の実装手順（将来のための記録）

```
1. 旧イベントを Deprecated としてマークする（削除しない）
2. 新イベントを別名（V2）で定義する
3. impl Upcaster for 旧イベント { fn upcast(self) -> 新イベント } を実装する
4. Replay パスで Upcaster を通す
5. このカタログに変更履歴を記録する
6. 新 ADR を作成する
```
