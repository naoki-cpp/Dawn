---
id      : ADR-0040
title   : dawn-client-gdext — GDExtension binding exposing dawn-client-core to Godot
status  : accepted
date    : 2026-07-10
deciders: [human, ai-agent]
related : ADR-0004（クライアント技術選択・godot-rust既定路線）, ADR-0039（dawn-client-core Phase 1）,
          docs/process/roadmap.md §13（Phase 10・GDExtension導入の当初計画）
---

# ADR-0040 — dawn-client-gdext GDExtension バインディング

## 背景

ADR-0039 で `dawn-client-core`（Godot 非依存の Loadout ドメインロジック）を Rust に
移植したが、その時点では GDExtension バインディングを意図的にスコープ外とし
「独立した後続ADRで扱う」とした（interface を先に `cargo test` だけで固めるため）。

roadmap.md §13 は GDExtension 導入を Phase 10（Client-Side Prediction 含む大きな
一括フェーズ）として位置づけていたが、本ADRは 8D 最小化方針に従い、
Phase 10 全体ではなく「ADR-0039 の Loadout モジュールを実際に Godot から呼べるようにする」
という薄いスライスだけを対象にする。Client-Side Prediction・WebSocket の Rust 化・
通信方式の再検討（wire-protocol.md 参照）は引き続き別スコープ。

## 決定

- 新規クレート `dawn-client-gdext`（`crate-type = ["cdylib"]`）を追加する。
  `dawn-client-core` にのみ依存し、Godot ↔ Rust の型変換だけを行う薄いアダプタ層とする
  （ドメインロジックは持ち込まない）。
- 依存クレートは `godot`（godot-rust/gdext）0.5、`api-4-6` feature でリポジトリの
  pinned Godot バージョン（`.godot-version`: 4.6.3-stable）に合わせる。
- GDExtension クラスは **旧 GDScript クラスと全く同じ名前**
  （`PlayerLoadout`/`ModuleRow`/`ItemRow`）で登録する。GDExtension クラスは
  `class_name` と同様グローバル識別子として自動公開されるため、この命名一致により
  既存の `const X = preload("res://scripts/x.gd")` 行を削除するだけで
  `main.gd`/`hud_manager.gd`/`hud_surface.gd`/`world_session.gd` 側のロジックは
  一切変更不要になった（フィールド名・メソッド名・`-1`/空文字列/空Dictionaryの
  「まだ何もない」センチネルまで完全一致させた）。
- `client/dawn_client_gdext.gdextension` をリポジトリに追加し、
  `res://../target/{debug,release}/...` を指す（Windows/Linux/macOS 分岐）。
  ビルド成果物の配置スクリプト化は将来必要になれば追加する（現状は
  `cargo build -p dawn-client-gdext` を手動実行し、Godot 側は `compatibility_minimum`
  と `reloadable = true` でホットリロードする前提）。
- `PlayerLoadout.apply_payload()` は JSON 文字列を受け取る（`dawn-client-core`
  の `serde_json` 経路をそのまま使うため）。`connection.gd` は既に全メッセージを
  Dictionary へパースしてから各ハンドラへ配っているため、そのディスパッチ形状は
  変えず、`main.gd::_on_player_fitting` 側で `JSON.stringify(payload)` して
  渡す1行変換のみで済ませた。
- `ModuleRow`/`ItemRow` の `from_json(dict: Dictionary) -> Variant` 静的コンストラクタと
  `PlayerLoadout.simulate_modules_capacitor_ticks(modules: Array, ...)` 静的関数を、
  旧 GDScript 版と同じ契約（必須キー欠落時は `null` を返しエラーログ）で維持した。
  これらは wire JSON 経由ではなく GdUnit4 テストが直接 Dictionary からモジュール行を
  組み立てて渡す経路（`world_session_test.gd`/`hud_surface_test.gd`）で使われている。

## 却下した案

- **Phase 10 全体（Client-Side Prediction 含む）を本ADRに含める**:
  Client-Side Prediction はサーバー権威の再現ロジックをクライアント側にも実装する
  必要があり、reconciliation 設計を要する別スコープの決定。ADR-0039/本ADRのスライスは
  「今の PlayerLoadout ロジックを Rust から呼べるようにする」だけに留める。
- **GDExtension クラス名を `PlayerLoadoutGd` 等の別名にする**: 呼び出し側
  （`main.gd` 等）で `Array[ModuleRow]` のような型注釈や `m.slot`/`m.kind` の
  dot-access が広範囲に使われており、旧クラス名と完全一致させる方が
  呼び出し側の変更ゼロで移行できる。将来的な名前衝突の懸念より、この移行の
  低リスクさを優先した。
- **`Option<T>` を Godot 側にそのまま `null` として伝播する**: `active_ship_id`/
  `docked_station_id` 等は `-1`/空文字列センチネルへ変換して返す
  （旧 GDScript の契約を維持し、呼び出し側の `>= 0` 比較を変えずに済ませるため）。

## 実装チェックリスト

- [x] `crates/dawn-client-gdext/Cargo.toml` 新設（`cdylib`、`dawn-client-core` +
      `godot`(api-4-6) + `serde_json` 依存）
- [x] ワークスペース `Cargo.toml` の `members` に追加
- [x] `PlayerLoadout`/`ModuleRow`/`ItemRow` GDExtension クラス実装
      （旧GDScriptと同名・同フィールド・同メソッド）
- [x] `client/dawn_client_gdext.gdextension` 追加（Windows/Linux/macOS のライブラリパス）
- [x] `main.gd`: `PlayerLoadoutScript`/`preload` 削除、`PlayerLoadout.new()` に置換、
      `apply_payload` 呼び出しを `JSON.stringify()` 経由に変更
- [x] `hud_manager.gd`/`hud_surface.gd`/`world_session.gd`: 旧 `const ModuleRow/ItemRow/
      PlayerLoadout = preload(...)` 行を削除（ロジック本体は無変更）
- [x] 旧 `client/scripts/player_loadout.gd`/`module_row.gd`/`item_row.gd` と
      `client/test/player_loadout_test.gd` を削除
- [x] `client/test/*_test.gd`（hud_manager/hud_surface/main/world_session）の
      preload 行を削除、`main_test.gd`/`world_session_test.gd` 側の呼び出しを
      GDExtension クラス経由に更新
- [x] `cargo fmt --all -- --check` / `cargo test --workspace` /
      `cargo clippy --workspace --all-targets -- -D warnings` 全件通過
- [x] Godot エディタでの手動検証: `--headless --editor --quit-after` で
      GDExtension ロード + `main.gd` パースエラーなしを確認。GdUnit4 全186ケース
      （14スイート）実行 — 0 errors / 0 failures / 0 orphans（実際のゲームプレイ
      による手動プレイテストは未実施、PR 説明に明記する）
