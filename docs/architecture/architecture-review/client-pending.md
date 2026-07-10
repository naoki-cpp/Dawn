---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 未完項目
audience : AI Agent / Human Developer
update   : /architecture-review で状態が変わるたびに更新
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）
date     : 2026-07-10
---

# Architecture Review — Dawn Client（未完項目）

C-1〜C-8 は解消済み（[client.md](./client.md) の Issue ID 登録簿を参照）。
2026-07-10 再計測で `hud_manager.gd` が watch 帯（🟡）に入ったため C-9 として新規記録。

`main.gd` の god object 問題は実質解消し、C-4（PlayerLoadout dict のスキーマ非検証）も
C-8（インベントリ行 Dictionary の stringly-typed 設計）も typed row 化で解消したため、
クライアント側の次の課題は構造リファクタではなく機能側（戦闘の深み、ADR-0016 §5）が妥当。

このファイルに残るのは、意図的に「今は変えない」と判断したものだけ。

---

## R-2 client `main.gd` 分割（サーバー側 pending.md と共通管理）

サーバー側の `server-pending.md` にも記載されている項目。
`WorldSession`・`WorldInteraction`・`WorldPresentation` 抽出で live world state /
world interaction policy / world visual side effect を移動し、`main.gd` は 1217 行
（client.md「ファイルサイズ一覧」参照。2026-07-10 再計測で前回1089から増加——
ドラッグ&ドロップ状態機械・Disembark・SHIPS 列ハンドラの追加）。残る scene lifecycle /
node generation / network send / HUD adapter は `.tscn` 化コンポーネントへのシーン参照切れ
リスクが上回るため保留。

再評価トリガー: 下記「採らない方針」の前提（ヘッドレス実行だけではシーンツリー構成の妥当性を
確認しきれない）が変わったとき、または `main.gd` が再び god object 的に肥大したとき。

## C-9（新規・2026-07-10）: `hud_manager.gd` が watch 帯（850行）に到達

2026-07-10 再計測で `hud_manager.gd` が 729→850 行に増加し、grading の watch 目安
（~500+行で成長中）に該当した。

**根本原因**: 全 HUD パネル（status/target/module bar/duel result/4列インベントリ&フィッティング）
の構築・更新を1つの static class に集約する設計（過去レビューで意図的に採用）が、
インベントリパネルへのドラッグ&ドロップ hit-test（`column_at`/`inventory_panel_consumes`/
`inventory_panel_row_at`/`module_slot_at`）追加のたびに線形に伸びている。

**判断: 保留（トリガー付き）。** 現時点では「HUD 構築」という単一責務は保たれており、
`build_*`/`update_*`のペア構造は崩れていない。ヒットテスト関数群だけを
`hud_hit_test.gd`のような sibling moduleへ抽出する余地はあるが、抽出しても
呼び出し側（`main.gd`）は変わらず、現時点では純粋な行数減らし以上の効果が薄い。

再評価トリガー: 総行数が1000行を超えたとき、または「HUD構築」と「ヒットテスト判定」という
2つの責務が実際に混線して見通しが悪化したとき（例: 新しいパネル種別の追加でbuild/hit-testの
対応関係を追うのが困難になったとき）。

---

## 採らない方針

- main.gd を複数の `.tscn` 化されたコンポーネント（個別シーン+スクリプト）に分割することは、
  シーン参照切れのリスクが高い。pin 済み Godot CLI で構文・実行エラーは検出できるようになったが、
  シーンツリー構成の妥当性（ノードパスの解決・レイアウト）はヘッドレス実行だけでは確認しきれない。
  GDScript ファイル内の `class_name` 抽出（同一シーンに留める）の方が安全で、C-1 では
  実際にこの方式で4クラスを抽出し、GdUnit4 で検証できた。
- raw `InputEvent` をそのまま deep module に飲ませることは当面行わない。`WorldInteraction` は
  正規化された input facts を受けて intent を返す形に留め、Godot の scene-tree / `InputEvent`
  依存を抱え込まない。これにより GdUnit4 での scene-tree なしテスト可能性を保っている。
