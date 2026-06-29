---
id      : ADR-0008
title   : 移動イベントの権威的設計：VelocityChanged
status  : accepted
date    : 2026-06-05
deciders: [human, ai-agent]
related : ADR-0001（Event Sourcing）, ADR-0006（Fitting/Combat）
---

# ADR-0008 — 移動イベントの権威的設計：VelocityChanged

## 背景

`ShipMoved`（毎 Tick の位置記録）と `ThrustApplied`（推力入力の記録）は
いずれも Event Sourcing の原則と相容れない問題がある。

```
ShipMoved    = 物理計算の結果（導出値）
ThrustApplied = 物理計算の入力（コマンドに相当）

いずれも「決定済みの事実」ではない。
```

さらに根本的な問題として「物理ルールのバージョン管理問題」がある。

```
ThrustApplied を記録した場合:
  → Replay 時に推力計算ロジックを実行する必要がある
  → 推力方程式が変更されると、過去ログから異なる位置が再計算される
  → 実装の詳細（物理コード）に Replay の正確性が依存する

これは数年後の Replay が保証できないことを意味する。
```

---

## 非交渉的な設計規則（本ドキュメントで確定する）

### 原則

1. **位置（Position）は派生状態である。** イベントに含めない。
2. **物理入力はコマンドである。** イベントに含めない。
3. **イベントは「決定済みの事実」のみを表現する。**
4. **Replay は物理シミュレーションを必要としない。**
5. **Replay の正確性は実装詳細（物理コード）に依存してはならない。**

### 禁止するイベント

```
❌ ThrustApplied   { direction }   ← 物理入力
❌ ForceApplied    { force }       ← 物理入力
❌ ShipMoved       { from, to }    ← 物理計算の結果（導出値）
```

---

## Decision: `VelocityChanged` が唯一の移動イベント

### 設計

```
SetThrustCommand
    ↓
MovementSystem（物理計算）
    ↓
VelocityChanged  ← 物理計算の出力 = 決定済みの事実
    ↓
EventStore に記録

Replay:
  VelocityChanged を順番に適用する
      ↓
  各 Tick で position += velocity を計算する
      ↓
  位置を再構築（物理シミュレーション不要）
```

### イベント定義

```rust
/// 船の速度が変化した。MovementSystem の計算結果として発行する。
/// 速度が前 Tick と同じ場合は発行しない。
pub struct VelocityChanged {
    pub ship_id  : ShipId,
    pub velocity : Velocity,   // 変化後の速度ベクトル（units/tick）
    pub tick     : Tick,
}
```

**なぜ `VelocityChanged` が「決定済みの事実」か:**
- 物理計算の「結果」であり「入力」ではない
- Replay 時に必要なのは「このベクトルで移動する」という事実だけ
- 推力方程式が変わっても、記録済みの `VelocityChanged` の意味は変わらない

### Replay の手順

```
1. VelocityChanged を時系列順に処理し、各 Tick 時点の velocity を確定する
2. tick ごとに position += velocity を適用して位置を再構築する
3. Snapshot がある場合は snapshot から始め、以降のイベントのみ処理する
```

`position += velocity` は物理ロジックではなく**純粋な算術**である。
この式は変わらない。

---

## `ShipMoved` の廃止

`ShipMoved` は `VelocityChanged` で置き換える。

```
廃止: ShipMoved { ship_id, from, to, tick }
導入: VelocityChanged { ship_id, velocity, tick }
```

**移行結果（完了）:**
- `ShipMoved` はコードベースから完全に削除した（新規プロジェクトのため既存ログはない）
- `VelocityChanged` のみを使用する

---

## クライアント（Godot）への影響

`VelocityChanged`（速度）を受け取る。`ShipMoved` は削除済み。

```
クライアント側の位置更新:
  VelocityChanged 受信 → ship.set_velocity(velocity)
  各フレーム          → position += velocity * delta_sec * TICKS_PER_SEC

メリット:
  - 船の動きが滑らかになる（Tick 間補間が自然に正確になる）
  - tick 境界での位置ジャンプがなくなる
```

---

## Snapshot の役割

Snapshot には位置・速度・HP・LockState を含めてよい。
ただし Snapshot は「Replay の高速化のための補助」であり、真実はイベントログである。

```rust
// Snapshot に含めてよいもの
struct ShipSnapshot {
    position   : Position,  // 派生状態だが Snapshot には含めてよい
    velocity   : Velocity,
    current_hp : f32,
    // ...
}
```

---

## ログサイズの見積もり

```
ShipMoved（旧）: 全移動船 × 全 Tick
  例: 5,000 ships × 36,000 ticks/時間 = 1億8千万 events/時間

VelocityChanged（新）: 速度が変化した時のみ
  例: NPC（等速直線運動）は spawn 時に 1 回のみ
      プレイヤー（加速中）はロック完了まで数 Tick
  通常 >> 99% の削減
```

---

## 実装チェックリスト

- [x] `dawn-core`: `VelocityChanged` イベント追加
- [x] `dawn-core`: `ShipMoved` を削除（新規プロジェクトのため Upcaster 不要）
- [x] `dawn-ecs/systems/movement.rs`: 速度が変化した場合のみ `VelocityChanged` を発行
- [x] `dawn-simulation/node.rs`: `apply_event` で `VelocityChanged` を処理
- [x] `dawn-simulation/ws_server.rs`: `VelocityChanged` を JSON で送信
- [x] `client/scripts/main.gd`: `VelocityChanged` ハンドラ追加
- [x] `client/scripts/ship_controller.gd`: フレームごとに velocity で位置更新

---

## 参照

- ADR-0001: Event Sourcing 基本原則
- AI_DEVELOPMENT_GUIDE.md「Project North Star」: 「Event が唯一の真実。State は派生物に過ぎない」
- AI_DEVELOPMENT_GUIDE.md「Architecture Invariants」INV-002: State は Event の Replay で完全再現できなければならない
- docs/architecture/event-schema-evolution.md: Event Schema Evolution Rules（`ShipMoved` の廃止手順）
