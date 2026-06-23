# /doc-sync — ドキュメントと実装の乖離チェック＆修正

このスキルはコードベースの現状とドキュメントを照合し、
不一致・陳腐化・誤記を洗い出して修正するまでを一括で行う。

フェーズ完了時・大きなリファクタ後・セッション開始時の定期メンテナンスとして実行する。

---

## 手順

### Step 1: イベント定義の照合

`dawn-core/src/events.rs` の `DomainEvent` enum を読み、
`docs/architecture/event-catalog.md` のイベント一覧と照合する。

確認項目:
- コードにあってカタログにないイベント → カタログに追記
- カタログにあってコードにないイベント → カタログから削除（または削除済みと明記）
- フィールド定義の不一致（型・フィールド名）→ カタログを実装に合わせる
- ステータス欄（✅ 実装済み / ⬜ 未実装 / @deprecated）が実態と異なる → 修正

### Step 2: ADR 実装チェックリストの照合

`docs/adr/` 以下の ADR のうち `## 実装チェックリスト` セクションを持つものだけを対象にする。
Grep で `実装チェックリスト` を含むファイルを特定してから読むこと（全 ADR を一括読みしない）。

確認項目:
- `[ ]` のままになっているが実際は実装済みの項目 → `[x]` に更新
- 説明文がコードの現状と食い違っている箇所 → 修正
- 存在しない型・フィールド・メソッドを参照している箇所 → 修正

### Step 3: Tick 処理順序の照合

`crates/dawn-sector/src/node/tick.rs` の `tick_with_lock_commands()` メソッドの処理順を読み、
`docs/architecture/tick-model.md` §3「Tick 内の処理ステップ」と照合する。

確認項目:
- ステップの順序・数・内容が実装と一致しているか
- 各ステップで発行されるイベントの種類が正しいか
- `AI_DEVELOPMENT_GUIDE.md` §6 の「Tick 内の処理順序」も同様に照合する

### Step 4: ロードマップの照合

`docs/process/roadmap.md` を読み、完了フラグ（`[x]` / `✅`）が実態と合っているか確認する。

確認項目:
- 実装済みなのに `[ ]` のままのタスク → `[x]` に更新
- 完了フェーズの説明文が現在の実装内容と一致しているか
- 次フェーズの前提条件に未完了のものがないか

### Step 5: AI_DEVELOPMENT_GUIDE.md の照合

`AI_DEVELOPMENT_GUIDE.md` を読み（`CLAUDE.md` はこのファイルへの委譲のみで §番号を持たない）、
以下の箇所が実態と合っているか確認する。

確認項目:
- §1「現在のスコープ」に列挙されているコンポーネント・イベント・コマンドが実装済みか
- §6「Tick 内の処理順序」が `tick.rs` と一致しているか（Step 3 と重複確認）
- §11「Crate別責務早見表」に全クレートが載っているか、禁止依存が変わっていないか
- フッターの「最終更新日」「対応ADR範囲」が古くなっていないか
- ADR-0030 で正典を外部化したセクションの**ポインタとリンクが生きているか**:
  §7 → `docs/architecture/event-schema-evolution.md` / §9 → `/ai-change-checklist` スキル /
  §10 → `docs/architecture/forbidden-changes.md`（FBD-00x ID 一覧がガイドと一致）/
  §12 → `docs/architecture/design-violations.md` / §8 GdUnit4 詳細 → `docs/process/godot-client-testing.md`。
  これらの参照先ファイルが存在し、ガイド側の要約・ID 一覧と矛盾しないことを確認する。
- ガイド冒頭が単一の H1 見出しであること、コードフェンスの外に裸の `#` 行（Markdown 見出し
  と誤認される）が残っていないことを確認する（2026-06-23 に発見・修正した不具合）。

### Step 6: プレイヤー向けドキュメントの照合

`docs/process/playtest-guide.md` を読み、現在のキー操作・機能と一致しているか確認する。

確認項目:
- キーバインドが `client/scripts/main.gd` の実装と一致しているか
- 存在しない機能が記載されていないか・実装済みの機能が未記載でないか

### Step 7: 設計ドキュメント群の照合

`docs/architecture/architecture.md` / `docs/architecture/entity-model.md` / `docs/architecture/ownership.md` /
`docs/design/game-design.md` を読み、実装状況の記述が実態と合っているか確認する。

確認項目:
- architecture.md: クレート一覧・依存 DAG に全クレートが載っているか
  （`ls crates/` と照合）。通信方式の記述が ADR-0007 と矛盾していないか
- architecture-review-server.md / architecture-review-client.md: ファイルサイズ一覧の行数が実際のファイルと一致しているか
  （`wc -l` で主要ファイルを照合。リファクタ後に stale になりやすい）
- entity-model.md: ECS Component 一覧が `dawn-ecs/src/components/` と一致しているか。
  「将来」「未実装」と書かれた項目が実装済みになっていないか
- ownership.md: 冒頭の実装状況テーブルとフェーズ表記が現在のフェーズと合っているか。
  状態遷移図に `（未実装）` ラベルが残っているイベント・操作が実際には実装済みでないか確認する
- game-design.md: §4 は「4.1 実装済み」と「4.2 将来検討する機能（未実装）」に分離済み
  （2026-06-23、混在が分かりにくいとの指摘で分割）。4.2 に実装済みの機能が紛れ込んでいないか、
  4.1 に対応 ADR が明記されているかを確認し、実装が進んだ項目は 4.2 から 4.1 へ移すこと

---

## 報告フォーマット

各 Step の後に以下の形式で報告する:

```
### Step N: <対象>
✅ 問題なし
```

または

```
### Step N: <対象>
⚠️ 不一致 N 件
  - <ファイル>: <内容>
  - ...
→ 修正済み
```

全 Step 完了後、変更があればまとめてコミットする。
コミットメッセージ: `docs: sync documentation with current implementation`
