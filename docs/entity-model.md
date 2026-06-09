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

**デフォルト値:** `SectorBounds::cube(10_000.0)` — 一辺 10,000 の立方体  
**境界越え時の挙動:** 速度の該当軸を反転（弾性壁バウンス）  

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

### ECS Component 一覧

| Component | フィールド | 説明 |
|---|---|---|
| `ShipIdComp` | `ShipId` | hecs Entity と domain ShipId の対応付け |
| `PositionComp` | `Position` | 現在の世界座標 |
| `VelocityComp` | `Velocity` | 1 Tick あたりの変位 |

Ship エンティティは必ず上記 3 Component を全て持つ。
一部だけを持つ不完全な Ship Entity を生成してはならない。

### Ship が現在持たないもの（MVP 外）

以下は将来のフェーズで追加するが、現在は存在しない。

```
Hull（船体耐久度）    ← Combat Context
Cargo（積荷）        ← Economy Context
Name（船名）         ← UI / Social Context
OwnerId（所有者）    ← Character Context
TransitState        ← Sector Transit 実装時に追加
```

### Ship Template（データ駆動設計・将来）

EVE Online には数百種類の船体がある。
各種類の「基本性能」をコードではなくデータとして管理する設計が必要。

```
ShipTemplate（不変・データファイル）   ShipInstance（可変・ECS）
─────────────────────────────────    ──────────────────────────
template_id: ShipTypeId              ship_id       : ShipId
name        : "Rifter"               template_id   : ShipTypeId  ← 参照
mass        : 1_067_000.0 kg         position      : Position
max_speed   : 380.0 m/s              velocity      : Velocity
turn_rate   : 3.28 deg/s             current_hp    : f32  （将来）
base_hull_hp: 563.0                  …
…
```

**設計原則：**
- Template は起動時にファイル（TOML / JSON）から読み込む
- Template はイミュータブル。変更はデプロイを伴う
- ECS Component は Instance の状態のみを保持し、基本パラメータは Template を参照する
- `ShipTypeId` は `dawn-core` に追加する（将来）

---

## 4. エンティティ：Node（論理概念）

現在の実装では Node は実行プロセス内の**論理的な分割単位**にすぎない。
将来のフェーズで物理的な分散ノードに対応する概念として設計されている。

| 属性 | 現在の実装 | 将来の実装 |
|---|---|---|
| 実体 | In-Process な論理識別子 | 独立したプロセス / マシン |
| 通信 | In-Memory Channel | gRPC / QUIC |
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
- Sector のサイズは固定（DEFAULT_SIZE = 10,000）
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
