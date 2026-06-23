---
status: Accepted
date: 2026-06-23
---

# ADR-0030 — ステアリング系ファイルの再構成（常時ロード文脈の軽量化）

## 背景

Anthropic の "Steering Claude Code: Skills, Hooks, Rules, Subagents and more"
（https://claude.com/ja/blog/steering-claude-code-skills-hooks-rules-subagents-and-more）
が示すベストプラクティスに照らして、本リポジトリのステアリング系ファイル群
（CLAUDE.md / AI_DEVELOPMENT_GUIDE.md / .claude/）を点検した。

記事の核心は **「仕組みごとに役割を分け、常時ロードされる指示（CLAUDE.md）は薄く
保つ」** ことにある。具体的には:

- CLAUDE.md は索引・規約・不変条件に絞り、目安 200 行以下に保つ。
- 30 行を超える手続き的ワークフローは **Skill** に出す（呼び出し時のみロード）。
- 特定パスにだけ効く制約は **Rules**（`paths:` スコープ）に出す。
- 「常に X する」「絶対に X しない」のような決定論的強制は **Hook** に出す。

現状の乖離:

| 項目 | 現状 | 記事の推奨 |
|---|---|---|
| CLAUDE.md | 5 行（委譲のみ） | OK（薄い） |
| AI_DEVELOPMENT_GUIDE.md | **1325 行**・CLAUDE.md から毎セッション "Read and follow" | 200 行目標に大幅超過 |
| Skills/Commands | doc-sync / hotspot / remove-event | OK（適切な手続き型） |
| Rules（path-scoped） | 無し | crate 固有制約を絞れる |
| Hooks | 無し → **本 ADR と同時期に 2 本導入**（ADR-0030 着手ブランチ） | 決定論的強制に活用 |

最大の問題は **1325 行のガイドが毎セッション常時ロードされる** こと。記事の言う
「コンテキストを圧迫すると指示の忠実度が落ちる」状態に当たる。なお本ガイドは
2026-06-19 に一度「肥大化解消」を実施済みであり、肥大化は継続課題である。

### 制約: 番号体系とリンクを壊さない

AI_DEVELOPMENT_GUIDE.md は INV-001〜006 / FBD-001〜009 や §番号で他ドキュメント
（docs/adr/ 各 ADR、doc-sync スキル、本 ADR を含む）から参照される「正典」である。
docs/adr/README.md も「ファイルの移動・リネームは行わない（既存リンク・CLAUDE.md
からの参照を壊さないため）」と定める。再構成はこの参照整合を最優先で守る。

## 決定

ステアリング系ファイルを記事の分業に沿って再構成する。**本 ADR は方針合意のための
提案（Proposed）であり、AI_DEVELOPMENT_GUIDE.md 本体の変更は人間承認後に別作業で
行う**（ガイド付録「このファイル自体の更新ルール」: ADR + 人間承認が必須）。

### 1. Hook（決定論的強制） — 着手済み

ADR-0030 着手ブランチ（chore/steering-hooks）で 2 本導入済み:

- `block-cjk-commit.py`（PreToolUse / Bash・PowerShell）: コミットメッセージに CJK
  が含まれる場合にブロックし、§8「コミットメッセージは英語」を機械的に強制する。
- `check-rust-fmt.py`（PostToolUse / Edit・Write）: 編集した `.rs` が rustfmt 未整形
  の場合に通知する（非ブロックの advisory）。

これにより §8 の英語コミット規約は「モデルの記憶頼み」から「ツール境界での強制」に
移る。

### 2. 手続き的ワークフローの Skill 化

着手前チェックの手続きである §9 AI Change Checklist を Skill（.claude/commands/）へ
抽出し、ガイド本体には「正典の所在を指す短い節」だけを残す。remove-event スキルが
既にこのパターンの実証例である。

> 当初案では §4 Event Workflow / §7 Schema Evolution も Skill 化を検討したが、これらは
> 「呼び出して実行する手続き」ではなく多数の変更に常時効く **参照ルール** であるため、
> Skill 化すると見落としリスクが高い。§4（正規イベントフロー）は常時ロード核に残し、
> §7 の詳細（リリース以降の Upcaster 手順・コード例。現在はプレリリースで非適用）は
> 参照 doc に降格してプレリリース注記＋リンクのみ残す（下記 3 に統合）。

### 3. 詳細カタログ・非適用ルールの参照ドキュメント降格

常時は不要だが必要時に参照する詳細は、独立 doc に出して「必要時に Read」する:

- §10 Forbidden Changes の詳細列挙 → `docs/forbidden-changes.md`。ガイドには
  FBD-00x の一覧（ID・一行要約）と参照リンクのみ残す。
- §12 よくある設計違反パターン → `docs/design-violations.md`。ガイドには参照リンクのみ。
- §7 Event Schema Evolution の詳細 → `docs/event-schema-evolution.md`。ガイドには
  「現在プレリリース＝破壊的変更可」の注記とリンクのみ残す。

### 4. crate 固有制約の path-scoped Rule 化（任意・将来）

§3 の「dawn-core はネットワーク・I/O・非同期ランタイム禁止」のような crate 固有
制約は、`paths: ["crates/dawn-core/**"]` を持つ Rule に出して、その crate を触る
ときだけ提示する余地がある。CI（cargo-deny / 循環依存検出）が既に最終防衛線である
ため優先度は低く、本 ADR では将来オプションとして記録するに留める。

### 5. ガイド本体に残すもの（常時ロードの最小核）

- §1 プロジェクト本質、§2 Architecture Invariants（INV）、§3 Dependency DAG 概要、
  §10 Forbidden の ID 一覧、§11 Crate 別責務早見表。
- すべての INV-/FBD- 番号と §番号アンカーは **ID を変えずに維持**する（本文が参照
  doc に移っても、ガイド側にアンカー付き要約とリンクを残す）。

## 却下した選択肢

### A: 現状維持（何もしない）
毎セッション 1325 行を常時ロードし続ける。指示忠実度の低下とコスト増が続くため却下。

### B: ガイドを物理分割せず、その場で攻撃的に削るだけ
2026-06-19 に一度実施済みで、再び肥大化した。手続きを Skill に出さない限り構造的に
再発するため、本質的解決にならない。

### C: ADR を起票せず一気に分割実装する
ガイド付録の更新ルール（ADR + 人間承認必須）に反する。番号体系・他 doc リンクへの
影響が大きく、合意なしの大改修はリスクが高いため却下。

## 実装チェックリスト

- [x] Hook: block-cjk-commit.py（PreToolUse）導入・検証
- [x] Hook: check-rust-fmt.py（PostToolUse）導入・検証
- [x] .claude/settings.json にフック登録（チーム共有）
- [x] 本 ADR の人間レビュー・承認（Proposed → Accepted）
- [x] §9 AI Change Checklist を Skill 化（/ai-change-checklist）
- [x] §10 詳細 → docs/forbidden-changes.md（ガイドに FBD ID 一覧 + リンク残置）
- [x] §12 → docs/design-violations.md（ガイドにリンク残置）
- [x] §7 詳細 → docs/event-schema-evolution.md（ガイドにプレリリース注記 + リンク残置）
- [x] §4 Event Workflow は常時ロード核に残置（INV-/FBD-/§番号アンカーは ID 維持）
- [x] doc-sync スキルの照合対象パスを再構成後の配置に更新
- [ ]（任意・将来）dawn-core 制約の path-scoped Rule 化

## 期待効果

- 常時ロード文脈が縮小し、長セッションでの指示忠実度が安定する（記事の主眼）。
- 「英語コミット」など決定論的規約が Hook で機械的に守られ、レビュー負荷が下がる。
- 手続きが Skill 化され、呼び出し時のみロードされることでノイズが減る。
- 番号体系を維持するため既存 ADR・doc からの参照リンクは無傷のまま。
