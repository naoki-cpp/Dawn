---
scope    : 世界に存在する「もの」の定義。型・フィールド・識別子のスキーマ仕様
audience : AI Agent / Human Developer
update   : 型定義・フィールド定義が変わったとき
related  : event-catalog.md, ownership.md, CLAUDE.md §5
---

# Entity Model

## 1. 識別子の設計

### EntityId

全エンティティに共通する一意識別子。

```
構造: [NodeId: 上位 8 bit | Counter: 下位 56 bit]
型  : u64 の newtype
```

**生成規則:**
- `NodeId` は ID を発行するノードの識別子
- `Counter` は同一 `NodeId` 内で単調増加する符号なし整数
- 2 つの `NodeId` が異なれば、`Counter` が同じでも ID は一意
- 一度発行した ID は **再利用しない**（INV-004）

**再利用禁止の理由:**  
再利用した ID を Event Log でリプレイすると、Despawn 済みの Ship が
再び Spawn したように見える矛盾が生じる。

### ShipId

`EntityId` の newtype。Ship エンティティであることを型で表明する。

```rust
// 型定義のイメージ（実装は dawn-core/src/entity.rs）
struct ShipId(EntityId);
```

将来 `StationId` 等が追加されても `ShipId` と混同されない。

### NodeId

```
型  : u8 の newtype
範囲: 0–255（最大 256 ノード）
現在: Single Process 内の論理識別子として使用
```

### SectorId

```
型  : u8 の newtype
範囲: 0–255
現在: 固定数・固定割り当て
```

---

## 2. 値オブジェクト

### Position

3 次元座標。World Space の単位は任意（現在は抽象的な距離単位）。

| フィールド | 型 | 説明 |
|---|---|---|
| `x` | `f32` | 東西方向 |
| `y` | `f32` | 上下方向 |
| `z` | `f32` | 南北方向 |

**精度の選択（f32）:**  
10,000 エンティティ規模の ECS バッチ処理では SIMD 最適化が効く f32 を採用。
天文学的精度（f64）は現フェーズでは不要。将来の要件変化は ADR で再評価する。

### Velocity

1 Tick あたりの変位ベクトル。単位は「距離単位 / Tick」。

| フィールド | 型 | 説明 |
|---|---|---|
| `dx` | `f32` | X 軸方向の変位 |
| `dy` | `f32` | Y 軸方向の変位 |
| `dz` | `f32` | Z 軸方向の変位 |

`Velocity::ZERO` は速度ゼロを表す定数。速度ゼロの Ship は `VelocityChanged` を発行しない。

### SectorBounds

Sector の空間的範囲を表す軸平行バウンディングボックス（AABB）。

| フィールド | 型 | 説明 |
|---|---|---|
| `min` | `Position` | 範囲の最小座標（原点側） |
| `max` | `Position` | 範囲の最大座標 |

**デフォルト値:** `SectorBounds::centered(DEFAULT_HALF)` — 原点中心・一辺 100,000（DEFAULT_HALF = 50,000）の立方体  
**境界越え時の挙動:** Tick ループでは境界判定を行わない（Phase 4 Cycle 2 で壁を削除 — 宇宙は無限）。
`SectorBounds` は現在スポーン位置の生成範囲としてのみ使用する。  

### Tick

論理時間カウンタ。詳細は [tick-model.md](./tick-model.md) を参照。

```
型  : u64 の newtype
初期値: Tick::ZERO (= 0)
性質 : 単調増加・物理時刻と無関係
```

---

## 3. エンティティ：Ship

現在の MVP における唯一のエンティティ種別。

### ECS Component 一覧（Phase 7 時点）

`SimWorld::spawn_ship()` は以下の Component を全て持つ Ship を生成する。
一部だけを持つ不完全な Ship Entity を生成してはならない。

| Component | 説明 |
|---|---|
| `ShipIdComp` | hecs Entity と domain ShipId の対応付け |
| `PositionComp` | 現在の世界座標 |
| `VelocityComp` | 1 Tick あたりの変位 |
| `ThrustComp` | 推力方向・ブレーキ状態（MoveCommand / StopCommand で更新） |
| `ShipStatsComp` | 集計済み stats（base_stats + Σmodule.delta、apply_fitting() が更新） |
| `FittingComp` | 装備スロット（High / Mid / Low / Rig の `FittedSlot` リスト） |
| `HullComp` | 3層 HP（Shield / Armor / Hull） |
| `WeaponComp` | 武器サイクル状態 |
| `LockComp` | ロックオン状態（ターゲットごとの `LockState`） |
| `IsNpcComp` | NPC マーカー（プレイヤー船は spawn 後に remove される） |
| `TransitComp` | Sector Transit 状態（`None` / `InTransit`、ADR-0014） |

追加で条件付きで付与される Component:

| Component | 条件 |
|---|---|
| `CapacitorComp` | プレイヤー船・ボット船（cap 管理対象） |
| `IsBotComp` | ボット船（`process_bots()` の対象マーカー） |
| `ApproachComp` | アプローチ中（対象 Ship / Jump Gate へ半自動接近・Move / Stop で除去・ADR-0015） |

### Ship が現在持たないもの（MVP 外）

以下は将来のフェーズで追加するが、現在は存在しない。

```
Cargo（積荷）        ← Economy Context
Name（船名）         ← UI / Social Context
```

※ `TransitState`（`TransitComp`）は Phase 7（ADR-0014）で実装済み（上表参照）。

※ 所有者（PlayerId）は ECS Component ではなく
  `SimulationNode` の `ship_owners: HashMap<ShipId, PlayerId>` で管理する。

### Ship Template（データ駆動設計・実装済み）

各船種の「基本性能」はコードではなくデータとして管理する。
Phase 4 Cycle 4 で `ShipTypeDefinition` + TOML 外部化として実装済み。

```
ShipTypeDefinition（不変・データ）     ShipInstance（可変・ECS）
─────────────────────────────────    ──────────────────────────
id          : ShipTypeId             ship_id       : ShipId
name        : "Magpie"               （ship_type_ids: ShipId → ShipTypeId
class       : ShipClass                は SimulationNode 側で管理）
slot_layout : SlotLayout             position      : Position
base_stats  : ShipBaseStats          velocity      : Velocity
                                     HullComp / CapacitorComp …
```

**実装の現状：**
- `data/ship_types.toml` から起動時に読み込む（DataLoader）
- ファイル不在時は `ship_types.rs` の built-in デフォルトへフォールバック
- 定義はイミュータブル。バランス調整は TOML 編集 + サーバー再起動（リビルド不要）
- `ShipTypeId` は `dawn-core` に定義済み。`ShipSpawned` イベントに含まれる

---

## 4. エンティティ：Node（論理概念）

現在の実装では Node は実行プロセス内の**論理的な分割単位**にすぎない。
将来のフェーズで物理的な分散ノードに対応する概念として設計されている。

| 属性 | 現在の実装 | 将来の実装 |
|---|---|---|
| 実体 | In-Process な論理識別子 | 独立したプロセス / マシン |
| 通信 | In-Memory Channel | ノード間: ネットワーク RaftTransport + ゴシップ（ワイヤ = postcard 再利用）。クライアント境界: WebSocket（ADR-0007）。gRPC/protobuf は不採用 |
| 障害 | 発生しない | Node Crash / Network Partition |

Node の物理的実装が変わっても、`NodeId` の役割（ID 発行の単位）は変わらない。

---

## 5. エンティティ：Sector

Ship を空間的に分割して管理する単位。

| 属性 | 説明 |
|---|---|
| `SectorId` | Sector の識別子 |
| `SectorBounds` | Sector の空間的範囲（AABB） |
| 管理 Node | この Sector を担当する論理 Node（[ownership.md](./ownership.md) 参照） |

**現在の制約:**
- Sector 数は固定（MVP: 3）
- Sector のサイズは固定（一辺 100,000 = DEFAULT_HALF × 2、原点中心）
- 動的分割・統合は未実装

---

## 6. 型の後方互換性ルール

`dawn-core` の型変更は全クレートに波及するため特に慎重に扱う。

```
許可: フィールドの追加（ただし Option<T> として追加し、既存コードを壊さない）
禁止: フィールドの削除
禁止: フィールドの型変更（f32 → f64 等）
禁止: フィールド名の変更（シリアライズキーが変わる）
禁止: newtype のラップ解除
```

型を変更する必要が生じた場合は ADR を起こして人間の承認を得ること。
