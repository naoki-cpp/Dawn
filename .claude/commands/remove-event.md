# /remove-event — 廃止イベントの完全削除

**引数:** 削除するイベント名（例: `/remove-event ShipMoved`）

このスキルは指定された `DomainEvent` バリアントとその構造体を
コードベース・テスト・ドキュメントから完全に削除する。

**前提:** プレリリース段階（永続化された外部ユーザーのイベントログが存在しない）。
リリース後に実行する場合は CLAUDE.md §7「リリース以降に破壊的変更が必要な場合の手順」に従うこと。

---

## 手順

### Step 1: 削除対象の確認

`crates/dawn-core/src/events.rs` を読み、以下を確認する:
- `pub struct <EventName> { ... }` が存在するか
- `DomainEvent::<EventName>(...)` バリアントが存在するか
- `#[deprecated]` または `@deprecated` が付いているか（付いていない場合は削除の意図を確認する）

### Step 2: 全参照箇所の洗い出し

以下のパターンで grep して参照箇所を全て列挙する:

```
<EventName>
```

対象範囲: `crates/`, `docs/`, `client/`, `CLAUDE.md`

典型的な参照場所:
- `crates/dawn-core/src/events.rs` — struct 定義、enum バリアント、`ship_id()`/`tick()` メソッドの match アーム
- `crates/dawn-simulation/src/node.rs` — `apply_event()` の match アーム
- `crates/dawn-simulation/src/ws_server.rs` — JSON 変換の match アーム
- `crates/dawn-simulation/src/spawner.rs` — テスト・アサーションのコメント
- `crates/dawn-actor/src/event_store_actor.rs` — テストヘルパー
- `crates/dawn-actor/src/replication_bus.rs` — テストヘルパー
- `crates/dawn-event-store/src/file.rs` — テストヘルパー
- `crates/dawn-event-store/src/memory.rs` — テストヘルパー
- `docs/event-catalog.md` — イベント一覧・詳細セクション
- `docs/tick-model.md` — ステップ説明・例示コード
- `docs/adr/` — 廃止手順の記述
- `CLAUDE.md` — 例示コード・注記

### Step 3: コアコードの削除

`crates/dawn-core/src/events.rs` を編集する:

1. `pub struct <EventName> { ... }` を削除
2. `DomainEvent::<EventName>(<EventName>)` バリアントを削除
3. `ship_id()` メソッドの `DomainEvent::<EventName>(e) => e.ship_id` アームを削除
4. `tick()` メソッドの `DomainEvent::<EventName>(e) => e.tick` アームを削除
5. `#[allow(deprecated)]` アノテーションが残っていれば削除

### Step 4: シミュレーション層の削除

`crates/dawn-simulation/src/node.rs`:
- `apply_event()` の `DomainEvent::<EventName>(_) => { ... }` アームを削除
- コメント内の参照を適切な表現に修正

`crates/dawn-simulation/src/ws_server.rs`:
- `DomainEvent::<EventName>(_) => return None` などのアームを削除

`crates/dawn-simulation/src/spawner.rs`:
- テストのアサーションメッセージ内の参照を修正

### Step 5: テストヘルパーの置き換え

以下の各ファイルで `<EventName>` を使っているテストヘルパーを
既存の代替イベント（通常は `VelocityChanged` または `ShipSpawned`）で置き換える:

- `crates/dawn-actor/src/event_store_actor.rs`
- `crates/dawn-actor/src/replication_bus.rs`
- `crates/dawn-event-store/src/file.rs`
- `crates/dawn-event-store/src/memory.rs`

置き換えパターン:
```rust
// Before
use dawn_core::events::<EventName>;
DomainEvent::<EventName>(<EventName> { ship_id: ..., ... })

// After (VelocityChanged で代替する場合)
use dawn_core::{events::VelocityChanged, Velocity};
DomainEvent::VelocityChanged(VelocityChanged {
    ship_id : ShipId::new(NodeId(0), n),
    velocity: Velocity::new(1.0, 0.0, 0.0),
    tick    : Tick(tick),
})
```

### Step 6: ドキュメントの更新

`docs/event-catalog.md`:
- イベント一覧テーブルから `<EventName>` の行を削除
- `@deprecated` 注記ブロックを削除
- `### <EventName>` 詳細セクションを削除（存在する場合）

`docs/tick-model.md`:
- `@deprecated` 注記・例示コードを削除または修正
- §4「tick フィールドの必須化」の例示コードが削除したイベントを使っていれば修正

`docs/entity-model.md`:
- 削除したイベント名を参照している箇所を修正

対応 ADR（`docs/adr/ADR-XXXX-*.md`）:
- 「廃止手順」セクションを「削除済み」に書き換え
- 実装チェックリストを `[x]` に更新

`CLAUDE.md`:
- 例示コードに削除したイベント名が残っていれば修正

### Step 7: ビルド検証

```bash
cargo test --workspace
```

エラーが出た場合は Step 2 の grep 漏れがある。エラーメッセージの参照先を修正して再実行する。

### Step 8: コミット

```bash
git add -p   # 変更を確認しながらステージング
git commit -m "refactor(dawn-core): remove deprecated <EventName> event (ADR-XXXX)"
```

---

## 注意事項

- Step 2 の grep で見つかった参照を 1 件でも見落とすとコンパイルエラーになる
- `#[allow(deprecated)]` アノテーションが残ると警告が出続ける
- テストヘルパーの置き換えは「イベントのセマンティクス」ではなく「ログ操作のテスト」が目的なので、どのイベントで代替してもよい
