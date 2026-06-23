# /ai-change-checklist — コード変更前チェックリスト

このスキルはコードを変更する前に確認すべき項目を順に点検する。
AI_DEVELOPMENT_GUIDE.md §9 の正典（手続き）であり、ガイド本体にはこのスキルへの
ポインタのみを残す（ADR-0030）。

**全項目に「問題なし」と判断できない場合は変更を止め、確認を求めること。**

---

## 変更前の確認

```
□ 変更するCrateを特定した
□ そのCrateの責務を Crate別責務早見表（AI_DEVELOPMENT_GUIDE.md §11）で確認した
□ 変更によって影響を受けるCrateを Dependency DAG（§3）で特定した
□ 変更が現在のスコープ内であることを確認した（§1）
□ 変更が Architecture Invariants（§2）のいずれかを破らないことを確認した
□ 変更が Forbidden Changes（docs/forbidden-changes.md / FBD-001〜009）に
  該当しないことを確認した
```

## イベントを追加・変更する場合の追加確認

```
□ docs/event-catalog.md の更新を計画した
□ 新Eventは dawn-core/src/events.rs に追加した（他のCrateに追加していない）
□ 新Eventに tick: Tick フィールドが含まれる（ShipMoveカテゴリのEvent）
□ 新Eventのフィールドは全て Option ではなく必須フィールドで設計した
  （Optional フィールドは後から追加、最初から Optional にしない）
□ 対応する Command が dawn-core/src/commands.rs に存在する
□ 既存 Event を変更する場合: リリース済みか確認した
  - プレリリース（現在）→ 破壊的変更を直接行ってよい（Upcaster 不要）
  - リリース以降       → docs/event-schema-evolution.md
    「リリース以降に破壊的変更が必要な場合の手順」に従う
```

## 新しいCrateを追加する場合の追加確認

```
□ 新Crateの追加が既存Crateの責務分割で対応できないことを確認した
□ 新Crateの Dependency DAG 上の位置を決定した
□ 循環依存が発生しないことを確認した（cargo tree で検証）
□ AI_DEVELOPMENT_GUIDE.md §11（Crate別責務早見表）を更新した
□ 対応するADRを docs/adr/ に作成した
```

## テストの確認

```
□ 変更した全ての pub fn に対応するテストが存在する
□ テスト関数名が「何が保証されるか」を説明している
□ cargo test --workspace がゼロエラーで通過することを確認した
□ 変更したADRが存在する場合、そのADRに記載された不変条件のテストが存在する
□ client/scripts/ を変更した場合: シーンツリー無依存の純粋関数なら
  client/test/ にGdUnit4テストを追加した（§8「Godot クライアントのテスト方針」参照）
□ client/scripts/ のシーンツリー依存部分を変更した場合: テストの代わりに
  Godot エディタでの確認内容（または確認できなかった旨）をPR説明に明記した
```

## PR説明の確認

```
□ 変更の動機を記載した（なぜこの変更が必要か）
□ 変更・参照したADRを記載した（例: ADR-0003 参照）
□ 変更したCrateの一覧を記載した
□ 影響を受けるEventの一覧を記載した（あれば）
□ テスト方法を記載した
```
