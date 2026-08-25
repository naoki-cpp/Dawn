---
scope    : Roadmap entry point, current phase, and reading order
audience : AI Agent / Human Developer
update   : Update when the current task or priority changes
related  : ./pending.md, ./completed.md, ./deferred.md, ../roadmap.md, ../roadmap-history.md
---

# Roadmap

このディレクトリがロードマップの正典である。architecture review と同じく、
現在地・未完了・完了済み・条件待ちを別ファイルに分け、AIが毎回読む範囲を小さくする。

## 読み方

1. まずこのファイルの「現在地」と「次の1件」を読む。
2. 実装対象は [pending.md](./pending.md) から選ぶ。
3. 条件が整うまで着手しない項目は [deferred.md](./deferred.md) に置く。
4. 完了済みの確認が必要なときだけ [completed.md](./completed.md) を読む。
5. 過去の判断根拠・計測値は [../roadmap-history.md](../roadmap-history.md)、ADR、architecture docsを参照する。

## 現在地

現在のフェーズ: **Phase 9 基盤実装完了・9E 経済ループ検証中**

- Rust workspace はグリーン。Godot実行前に `cargo build -p dawn-client-gdext` でDLLを生成する。
- 9A（Scrap Metal）、9B（Station / Ship操作）、9D（Market / Currency）は実装済み。
- 9C（プレイヤー設置インフラ）は設計未着手。新規ADRが必要。
- 9E-1 は手順準備済み、人間によるプレイテストと結果記録が未完了。
- Phase 10 はGDExtension・wire移行・`ShipMotion`によるClient-Side Prediction /
  reconciliationまで実装済み。Godotエディタでの手動プレイテストのみ残る。
- Phase 11 は天体表示、ワープ演出、Filmic/Bloom、星野geometry、
  起動時ネビュラベイクまで実装済み。船モデルと戦闘フィードバックが残る。

### 次の1件

**9E-1 — 経済ループのプレイテスト**

`docs/process/playtest-guide.md` の「Phase 9 — 経済ループ検証」を実施し、
Scrap Metal獲得、Station操作、Market取引が判断や対立を生むかを記録する。
自動テストだけではこの完了条件を判定しない。

## 参照入口

- [未完了タスク](./pending.md)
- [完了済み（短縮）](./completed.md)
- [条件待ちバックログ](./deferred.md)
- [互換性用の旧入口](../roadmap.md)
