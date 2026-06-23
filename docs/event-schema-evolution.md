# Event Schema Evolution Rules

> AI_DEVELOPMENT_GUIDE.md §7 の詳細の正典。ガイド本体には「現在プレリリース＝破壊的変更可」
> の注記とこのファイルへのリンクのみを残す（ADR-0030）。

## フェーズによる適用範囲

このルールには **プレリリース（現在）** と **リリース以降** の 2 段階がある。

```
プレリリース（Phase 1〜リリース前）:
  永続化されたイベントログを持つ外部ユーザーが存在しない。
  → 破壊的変更（フィールド削除・型変更・イベント削除）を直接行ってよい。
  → Upcaster・V2 命名・Deprecated マークは不要。
  → ただし docs/event-catalog.md と AI_DEVELOPMENT_GUIDE.md は常に実態と合わせること。

リリース以降（本番ログが存在する段階）:
  外部ユーザーのイベントログが存在する。
  → 既存フィールドの変更・削除は Upcaster なしに行ってはならない。
  → 以下「リリース以降の制約」が完全に適用される。
```

**現在は Phase 6（プレリリース）。破壊的変更は許可されている。**

---

## リリース以降の基本原則

**既存の Event フィールドを変更・削除してはならない。**
**新しいフィールドの追加のみが許可される。**

### リリース以降に許可される変更

```rust
// 変更前
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
    pub damage   : f32,
    pub tick     : Tick,
}

// 変更後: 新フィールドの追加は許可（必ず Option にする）
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: ShipId,
    pub damage   : f32,
    pub tick     : Tick,
    pub hit_chance: Option<f32>,  // ← 新フィールドは Option<T> で追加
}
```

### リリース以降に禁止される変更

```rust
// 禁止1: フィールドの削除
pub struct WeaponFired {
    pub ship_id  : ShipId,
    // target_id を削除 ← 禁止。過去のEventのReplayでデシリアライズが失敗する
    pub damage   : f32,
    pub tick     : Tick,
}

// 禁止2: フィールドの型変更
pub struct WeaponFired {
    pub ship_id  : ShipId,
    pub target_id: u64,   // ShipId → u64 に変更 ← 禁止
    pub damage   : f32,
    pub tick     : Tick,
}

// 禁止3: フィールド名の変更（シリアライゼーションのキーが変わる）
pub struct WeaponFired {
    pub attacker_id: ShipId,  // ship_id → attacker_id に変更 ← 禁止
    pub target_id  : ShipId,
    pub damage     : f32,
    pub tick       : Tick,
}
```

### リリース以降に破壊的変更が必要な場合の手順

```
1. 新しい Event を別名で定義する
   例: WeaponFired → WeaponFiredV2

2. 古い Event を Deprecated としてマークする（削除しない）
   /// @deprecated WeaponFiredV2 を使用すること
   pub struct WeaponFired { ... }

3. Upcaster を実装する
   impl Upcaster for WeaponFired {
       fn upcast(self) -> WeaponFiredV2 { ... }
   }

4. Replay 時に Upcaster を通して新形式に変換する

5. docs/event-catalog.md を更新する

6. 対応する ADR を作成する（既存 ADR の更新ではなく新規作成）
```

## Event Catalog との同期

`docs/event-catalog.md` が Event の唯一の仕様書である。
フェーズにかかわらず、コードの変更と同時に更新すること。

```bash
# Event定義とカタログの整合をCIで検証する
cargo run --bin check-event-catalog

# このコマンドが失敗する場合、以下のいずれかが発生している:
# - コードにあってカタログにないEvent
# - カタログにあってコードにないEvent
# - フィールド定義の不一致
```
