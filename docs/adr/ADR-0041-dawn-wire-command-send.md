---
id      : ADR-0041
title   : dawn-wire crate + GDExtension command-send — client -> server wire schema shared, not duplicated
status  : accepted
date    : 2026-07-11
deciders: [human, ai-agent]
related : ADR-0004（クライアント技術選択・godot-rust既定路線）, ADR-0039（dawn-client-core Phase 1）,
          ADR-0040（dawn-client-gdext GDExtensionバインディング）,
          docs/process/roadmap.md §13（Phase 10 タスク2）
---

# ADR-0041 — dawn-wire クレート新設 + コマンド送信の GDExtension 化

## 背景

ADR-0040 は roadmap.md §13 タスク2（`dawn-core` 型を GDExtension 経由で Godot へ公開）の
第一弾として PlayerLoadout（受信側・状態表現）だけを実装し、「残るチャンネル（Command 送信・
DomainEvent 受信）は未着手」と明記してスコープ外にした。

本ADRはそのうち **Command 送信** を対象にする。現状 `client/scripts/connection.gd` は
`send_move_command`等26個の関数それぞれが、`crates/dawn-actor/src/protocol/client_command.rs`
（`ClientCommandJson`）のワイヤ形状を GDScript の `Dictionary` リテラルとして手打ちで再現し、
`JSON.stringify()` して送信している。この手打ちには型チェックが一切効かず、サーバー側の
スキーマが変わってもクライアント側が気づかず壊れる（あるいは逆に、クライアント側の
タイプミスがサーバー側で静かに `None` 拒否される）リスクが常にある。PlayerLoadout で
`dawn-client-core`/`dawn-client-gdext` が解決したのと同種の問題が、送信方向にも存在する。

## 決定

### dawn-wire クレートの新設

`crates/dawn-actor/src/protocol/client_command.rs`（`ClientCommandJson`/`PosJson`/
`VelJson`/`WarpTargetJson`/`parse_client_command`/`client_command_json_schema`）を、
新規クレート `dawn-wire`（`dawn-core` + `serde`/`serde_json`/`schemars` のみ依存、
トランスポート/非同期ランタイム依存なし）へ丸ごと移動する。

**理由**: 当初「`dawn-client-gdext` が `dawn-actor` を直接依存する」案を検討したが、
`dawn-actor` は `tokio`（フルランタイム）・`tokio-tungstenite`・`anyhow`・`futures-util`
という **WebSocket サーバー実装のための依存** を抱えている。クレート単位の依存になるため、
「`ClientCommandJson` という型定義だけ」を取り出すことができず、Godot の GDExtension
cdylib にサーバー用の非同期ランタイム一式が丸ごと付いてくることになる。`dawn-wire` を
`dawn-core` 直下の葉クレートとして切り出すことで、`dawn-actor`（サーバー、deserialize）と
`dawn-client-gdext`（クライアント、construct + serialize）の両方が同じ型を、
不要な依存を持ち込まずに使える。

`dawn-actor::protocol` は `pub use dawn_wire::{...}` で同名再エクスポートし、
`ws_server.rs`/`dawn-sector-node`/既存テスト等、呼び出し側の import パスは無変更で済む。

`ClientCommandJson` に `Serialize` を追加する（従来は `Deserialize` のみ）。
サーバーは受信したワイヤ行をこの型へ deserialize する一方、クライアント
（`dawn-client-gdext`）はこの型の値を直接構築して serialize で送り出す、
という双方向の使い方になるため。

### コマンド送信の GDExtension 化

`crates/dawn-client-gdext/src/client_command_gd.rs` に新規 GDExtension クラス
`ClientCommand`（`dawn-wire` にのみ依存、`dawn-client-core`とは独立）を追加する。
`connection.gd`の26個の`send_*_command`関数それぞれに対応する静的メソッド
（例: `ClientCommand.move_command(x, y, z) -> String`）を持ち、`ClientCommandJson`
のバリアントを直接構築して `serde_json::to_string` した1行JSON文字列を返す
（`redirect_json`が同じパターンをサーバー側で既にやっている）。

`connection.gd`側は「`Dictionary`を手打ち→`JSON.stringify`」を「`ClientCommand.xxx(...)`
の戻り値をそのまま送る」に置き換える。`_send_json`（"type"注入+JSON化を担っていた
共通ヘルパー）は`_send_line`（welcomedガード+改行付与のみ）に縮小される。

DomainEvent 受信（`connection.gd::_handle_message`のDictionaryディスパッチ）は
引き続きスコープ外（次の後続ADR）。

## 却下した案

- **`dawn-client-gdext`が`dawn-actor`を直接依存する**: 背景の通り、`tokio`等の
  トランスポート依存を不要にクライアントcdylibへ持ち込むため却下。
- **`dawn-client-core`に`ClientCommandJson`を追加する**: `dawn-client-core`は
  ADR-0039で「`dawn-core`にのみ依存」と決めた境界を持つ。`ClientCommandJson`は
  ワイヤ形式（`String`スロット名・`Option<T>`の`Some`/`None`によるバリアント選択等）
  であってドメインモデルではないため、`dawn-client-core`（Godot非依存の*ドメイン*
  ロジック）よりも新設`dawn-wire`（ワイヤ*スキーマ*専用）の責務に合致する。
- **DomainEvent受信も同じADRでまとめて対応する**: 受信側は型の種類がコマンド側より
  多く、`main.gd`側のハンドラ変更量も大きい。8D最小化方針に従い、送信側だけの薄い
  スライスに留めた。

## 実装チェックリスト

- [x] `crates/dawn-wire/Cargo.toml`新設（`dawn-core`+`serde`+`serde_json`+`schemars`のみ）
- [x] `crates/dawn-actor/src/protocol/client_command.rs`を`dawn-wire/src/client_command.rs`
      へ移動、`ClientCommandJson`に`Serialize`を追加
- [x] `dawn-actor/src/protocol/mod.rs`: `pub use dawn_wire::{...}`で同名再エクスポート
- [x] `dawn-actor/Cargo.toml`: `dawn-wire`依存追加
- [x] ワークスペース`Cargo.toml`の`members`に`dawn-wire`追加
- [x] `cargo run -p dawn-actor --example gen_wire_schema`で schema 再生成
      （形状は不変、docコメントのみ差分）
- [x] `crates/dawn-client-gdext/src/client_command_gd.rs`新設（`ClientCommand`
      GDExtensionクラス、26メソッド）
- [x] `crates/dawn-client-gdext/Cargo.toml`: `dawn-wire`依存追加
- [x] `client/scripts/connection.gd`: 全`send_*_command`を`ClientCommand.*`経由に置換、
      `_send_json`→`_send_line`に縮小
- [x] GdUnit4: `client/test/client_command_gd_test.gd`新設（代表的なコマンドの送信JSON検証、
      `ClientCommand`の主要メソッドをカバー）
- [x] `cargo fmt --all -- --check` / `cargo test --workspace` /
      `cargo clippy --workspace -- -D warnings` 全件通過
- [x] Godot エディタでの手動検証: `--headless --editor --quit-after`でGDExtensionロード確認。
      `--headless --script`での直接実行で`ClientCommand`の全代表メソッドが期待通りのJSON
      （例: `{"type":"MoveCommand","target":{"x":10.0,"y":0.0,"z":-5.0}}`）を返すことを確認。
- [ ] **GdUnit4自動テスト実行は未達成** — この実装作業を行った環境で、GdUnit4のテスト
      ランナー自体が本ADRの変更と無関係に（このセッションの変更を一切含まないtrue baseline、
      および既存の無関係なテストファイル単体でも）SIGSEGVでクラッシュする状態だったため、
      `client/test/client_command_gd_test.gd`を含む自動テストスイートの実行確認は次回
      Godotエディタが正常な環境で改めて行うこと。上記の`--headless --script`による手動検証は
      その代替として実施したもので、GdUnit4アサーションの網羅性までは担保しない。
