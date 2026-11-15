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

現在のフェーズ: **Phase 9 基盤実装完了・経済ループ検証前に Item / Station のゲームプレイを深化中**

- Godot実行前は `scripts/setup-godot.ps1` または `bash scripts/setup-godot.sh` で
  固定版の導入・拡張ビルド・importまで準備する（Python 3.10+とCargoが必要）。
  Rust変更後に拡張だけ更新する場合は `cargo build -p dawn-client-gdext` を使う。
- 9A（Scrap Metal）、9B（Station / Ship操作）、9D（Market / Currency）は基盤実装済み。
- 現状の Item / Station は最小構成であり、経済ループ全体を評価するには用途・供給/消費・サービスの幅が不足している。
- 9E-0で Item の役割と Station のサービスを設計・実装し、その後に9E-1のプレイテストを行う。
- 9C（プレイヤー設置インフラ）は別系統のTODO。関連ADR-0057 / ADR-0058は提案中で、採取ルールやFBD-009の変更は承認前提とする。
- 9E-1 は現行MVP向けの手順があるが、9E-0完了後に実装内容へ合わせて更新してから実施する。
- Phase 10 はGDExtension・wire移行・`ShipMotion`によるClient-Side Prediction /
  reconciliationまで実装済み。Godotエディタでの手動プレイテストのみ残る。
- Phase 11 は天体・恒星表現、レンズ応答、ワープ演出、Filmic/Bloom、星野geometry、
  銀河面の視線積分と起動時ベイクまで実装済み。船モデルと戦闘フィードバックが残る。

### 次の1件

**9E-0 — Item / Station のゲームプレイを深化する**

Item の用途・供給・消費と Station のサービス構成を決め、実際にプレイ可能な内容へ反映する。
プレイヤー設置構造物（9C）や未承認のルール変更を前提にせず、完了後に9E-1のシナリオを
更新して人間による経済ループ検証を行う。

## 参照入口

- [未完了タスク](./pending.md)
- [完了済み（短縮）](./completed.md)
- [条件待ちバックログ](./deferred.md)
- [互換性用の旧入口](../roadmap.md)
