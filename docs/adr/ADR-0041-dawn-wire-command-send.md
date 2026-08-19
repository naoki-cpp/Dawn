---
id      : ADR-0041
title   : dawn-protocol crate + GDExtension command-send — client -> server wire schema shared, not duplicated
status  : accepted
date    : 2026-07-11
deciders: [human, ai-agent]
related : ADR-0004（クライアント技術選択・godot-rust既定路線）, ADR-0039（dawn-client-core Phase 1）,
          ADR-0040（dawn-client-gdext GDExtensionバインディング）,
          docs/process/roadmap.md §13（Phase 10 タスク2）
---

# ADR-0041 — dawn-protocol クレート新設 + コマンド送信の GDExtension 化

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

### dawn-protocol クレートの新設

`crates/dawn-actor/src/protocol/client_command.rs`（`ClientCommandJson`/`PosJson`/
`VelJson`/`WarpTargetJson`/`parse_client_command`/`client_command_json_schema`）を、
新規クレート `dawn-protocol`（`dawn-core` + `serde`/`serde_json`/`schemars` のみ依存、
トランスポート/非同期ランタイム依存なし）へ丸ごと移動する。

**理由**: 当初「`dawn-client-gdext` が `dawn-actor` を直接依存する」案を検討したが、
`dawn-actor` は `tokio`（フルランタイム）・`tokio-tungstenite`・`anyhow`・`futures-util`
という **WebSocket サーバー実装のための依存** を抱えている。クレート単位の依存になるため、
「`ClientCommandJson` という型定義だけ」を取り出すことができず、Godot の GDExtension
cdylib にサーバー用の非同期ランタイム一式が丸ごと付いてくることになる。`dawn-protocol` を
`dawn-core` 直下の葉クレートとして切り出すことで、`dawn-actor`（サーバー、deserialize）と
`dawn-client-gdext`（クライアント、construct + serialize）の両方が同じ型を、
不要な依存を持ち込まずに使える。

`dawn-actor::protocol` は `pub use dawn_protocol::{...}` で同名再エクスポートし、
`ws_server.rs`/`dawn-sector-node`/既存テスト等、呼び出し側の import パスは無変更で済む。

`ClientCommandJson` に `Serialize` を追加する（従来は `Deserialize` のみ）。
サーバーは受信したワイヤ行をこの型へ deserialize する一方、クライアント
（`dawn-client-gdext`）はこの型の値を直接構築して serialize で送り出す、
という双方向の使い方になるため。

### コマンド送信の GDExtension 化

`crates/dawn-client-gdext/src/client_command_gd.rs` に新規 GDExtension クラス
`ClientCommand`（`dawn-protocol` にのみ依存、`dawn-client-core`とは独立）を追加する。
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
  ロジック）よりも新設`dawn-protocol`（ワイヤ*スキーマ*専用）の責務に合致する。
- **DomainEvent受信も同じADRでまとめて対応する**: 受信側は型の種類がコマンド側より
  多く、`main.gd`側のハンドラ変更量も大きい。8D最小化方針に従い、送信側だけの薄い
  スライスに留めた。

## 実装チェックリスト

- [x] `crates/dawn-protocol/Cargo.toml`新設（`dawn-core`+`serde`+`serde_json`+`schemars`のみ）
- [x] 旧 `crates/dawn-actor/src/protocol/client_command.rs` を
      `crates/dawn-protocol/src/client_request.rs` へ移動し、`Serialize`を追加
      （後続の #273 で `ClientRequest` に統合）
- [x] `dawn-actor/src/protocol/mod.rs`: `pub use dawn_protocol::{...}`で同名再エクスポート
- [x] `dawn-actor/Cargo.toml`: `dawn-protocol`依存追加
- [x] ワークスペース`Cargo.toml`の`members`に`dawn-protocol`追加
- [x] `cargo run -p dawn-actor --example gen_wire_schema`で schema 再生成
      （形状は不変、docコメントのみ差分）
- [x] `crates/dawn-client-gdext/src/client_command_gd.rs`新設（`ClientCommand`
      GDExtensionクラス、26メソッド）
- [x] `crates/dawn-client-gdext/Cargo.toml`: `dawn-protocol`依存追加
- [x] `client/scripts/connection.gd`: 全`send_*_command`を`ClientCommand.*`経由に置換、
      `_send_json`→`_send_line`に縮小
- [x] GdUnit4: `client/test/client_command_gd_test.gd`新設（代表的なコマンドの送信JSON検証、
      `ClientCommand`の主要メソッドをカバー）
- [x] `cargo fmt --all -- --check` / `cargo test --workspace` /
      `cargo clippy --workspace -- -D warnings` 全件通過
- [x] Godot エディタでの手動検証: `--headless --editor --quit-after`でGDExtensionロード確認。
      `--headless --script`での直接実行で`ClientCommand`の全代表メソッドが期待通りのJSON
      （例: `{"type":"MoveCommand","target":{"x":10.0,"y":0.0,"z":-5.0}}`）を返すことを確認。
- [x] GdUnit4自動テスト実行: 197/197 pass、0 errors/0 failures/0 orphans（`client_command_gd_test.gd`
      の11ケース含む）。**環境メモ**: `client/addons/gdUnit4/runtest.cmd`が実行する既定コマンドは
      `--headless`を付けずウィンドウ表示を試みる設計（`GdUnitCmdTool.gd`自体が`--headless`単体では
      明示的に実行を拒否する）。ディスプレイのないサンドボックス環境ではウィンドウ生成時に
      SIGSEGVでクラッシュしたため、`--headless --ignoreHeadlessMode`の組み合わせで実行し解決した
      （GdUnit4アドオンの再インストール・`.godot`キャッシュ削除はいずれも無関係だった）。
      この過程で見つかったテスト側の2バグも修正済み: (1) GDScriptの`JSON.parse_string()`は
      数値を常に`float`にするため`assert_int()`には`int(...)`キャストが必要、
      (2) `Option::None`はserdeで明示的な`null`としてシリアライズされる（キー省略ではない）ため、
      「省略される」ではなく「値が`null`」であることを検証するよう修正。`ClientCommand`実装
      本体に問題はなかった。

## 追記（2026-07-11）: gdext ラッパーの3箇所複製を解消

`/improve-codebase-architecture` によるレビューで、新規コマンド追加のたびに
`client_command_gd.rs`（gdext `#[func]`ラッパー）・`dawn-protocol`（`ClientCommandJson`
バリアント）・`node/commands.rs`（`apply_client_command`のmatch arm）という
3つの浅いモジュールを機械的に並行編集する必要がある、という編集面（edit-surface）
の重複が指摘された。

24メソッドのうち、sentinel値（`positive_or_none`/`non_negative_or_none`,
ADR-0031/ADR-0035）やタグ付きナビゲーション target の構築といった
ドメイン意味論を持つ12個（+`move_command`、呼び出し頻度が高いため専用のまま）は
専用メソッドとして維持し、残り14個（フラットなスカラーフィールドのみの単純な
詰め替え）を `ClientCommand.build(kind: String, fields: Dictionary) -> String`
という汎用メソッドに集約した。`build`は`fields`を`serde_json::Value`へ変換して
`"type": kind`を注入し、`serde_json::from_value::<ClientCommandJson>`で
デシリアライズを試みることで検証する（=dawn-protocolの既存Deserialize実装を
そのまま検証ロジックとして再利用）。フィールド名のtypoや必須フィールド欠落は
デシリアライズ失敗として検出され、`push_error`を出し空文字列を返す
（クラッシュせず、かつ黙って無視もしない）。

`Dictionary`→JSON変換はスカラー値（Int/Float/String/Bool）のみ対応し、ネストした
`Dictionary`/`Array`は`push_error`で明示的に弾く（今日の14コマンドは全てスカラー
のみのため、ネスト対応は必要になった時点で追加する）。

その後、Issue #222 で `gate_id` xor `target_id` の排他フィールドと
Warp の legacy `gate_id` fallback は削除され、必須のタグ付き target enum に
置き換えられた。専用メソッドを維持する現在の理由は、この target enum を
型安全に構築するためである。

この結果、今後「単純な」新規コマンドを追加する場合は `dawn-protocol` のバリアント追加
と `node/commands.rs` のdispatch arm追加の2ファイルで済み、gdext側の編集は不要になる。
`connection.gd`側の公開API（`send_*_command`関数のシグネチャ）は変更していない
（内部実装だけが`_cmd.build(...)`呼び出しに変わった）。

GdUnit4: 既存の個別テストは維持しつつ、`build()`自体の契約（正常系・フィールド名
typo・必須フィールド欠落の3パターン）を検証する新規テストを追加。200/200 pass。

## 2026-08-06 amendment: direct typed Sector request builders (issue #273)

Sector command builders no longer construct a Dictionary/JSON shape. Each
GDExtension method constructs the single typed `ClientRequest` and postcard
encodes `ClientMessage::Command` directly. Builder failure is returned as a
structured dictionary (`ok`, `bytes`, `error_code`, `error_message`) instead of
an empty-byte sentinel. The generic schema-driven builder remains Market-only.

## 2026-08-10 amendment: remove the remaining dynamic client builders (issue #281)

The Market-only `ClientCommand.market_build(kind, fields)` helper is removed.
Market requests now use dedicated typed methods for refresh, place, and cancel.
The request side is no longer reconstructed through a Godot `Dictionary`,
`serde_json::Value`, and a string variant name. `MarketOrderSide` is a closed
wire enum, so an invalid side is rejected before postcard encoding.

All Sector and Market builders return the typed GDExtension
`ClientCommandResult`. It carries the encoded bytes or a structured error and
never uses an empty byte array as a failure sentinel. `ClientMessage::encode`
is fallible and its postcard error is propagated into that result.

Keyboard and world-click interpretation then returned the Rust-backed
`ClientIntent` object. The three mutually exclusive selection integers in
`WorldInteraction` are replaced by one `ClientSelection` value. GDScript keeps
scene ownership and side effects, while `main.gd` dispatches through typed
predicates and accessors instead of string tags and magic dictionary keys.

This input-specific part of the amendment was superseded by the 2026-08-17
amendment below; the historical implementation is retained here to explain
the migration sequence.

GdUnit4 covers every intent family, typed selection transitions, dedicated
Market builders, invalid Market sides/IDs, and invalid resume tickets.

## 2026-08-17 amendment: collapse the Client Action ladder

The former `ClientIntent` / `ClientSelection` GDExtension objects and the
GDScript `InputDecoder` policy layer are removed. Input interpretation now
has one engine-independent seam:

```text
Godot InputEvent / hit-test facts
  -> WorldInteraction adapter (Godot key normalization only)
  -> dawn-client-core::ClientInteraction
  -> ClientAction { Request(ClientRequest) | Local(ClientLocalAction) }
  -> main.gd one action executor
```

`dawn-client-core::ClientInteraction` owns mutually exclusive selection,
double-click timing, keyboard policy, and the construction of typed
`dawn_core::ClientRequest` values. `ClientLocalAction` contains only effects
that require the Godot scene or presentation layer. The core crate remains
Godot-independent and continues to depend only on `dawn-core`.

The GDExtension `ClientAction` wrapper exposes the action kind and only the
payload required by that kind. Network actions are validated and postcard
encoded by the existing command encoder. `connection.gd` adds one
`send_action()` transport entry point for typed requests in this path; the
camera-dependent double-click move remains an explicit `send_move_command()`
call after Godot projects the screen ray. The existing command wrappers also
remain for inventory, fitting, and market surfaces until those UI paths are
migrated independently. No wire schema or server contract changes.

The old predicate/accessor ladder, string tags, and `Dictionary` round-trip
are not retained as a compatibility layer. Core unit tests cover the policy;
GdUnit4 keeps only the thin Godot-boundary contract tests.

## 2026-08-19 amendment: extract Station Inventory interaction policy

Station Inventory interaction now follows the same one-way typed boundary,
without adding a new crate or changing the wire contract:

```text
HudManager row rendering / HudHitTest geometry
  -> StationInventoryRow (typed GDExtension metadata)
  -> dawn-client-core::StationInventoryInteraction
  -> StationInventoryAction { Request(s) | Local | None }
  -> connection.gd typed request transport / HudSurface local effect
```

`dawn-client-core` owns the engine-independent policy and constructs existing
`ClientRequest` values. A docked station is required for station operations;
`BuildPackagedShip` and `DisassembleShip` additionally require an active ship.
Assemble of a packaged ship and `SelectActiveShip` remain valid for a shipless
but docked player. Cargo transfer is restricted to `SHIP_CARGO -> STATION` or
`STATION -> SHIP_CARGO` and carries the canonical `ItemId` identity. Same-column
and invalid drops are no-ops.

The build-picker toggle is a Godot-local action; choosing a ship type creates a
typed build request. `Unfit All` returns an ordered list of independent
`UnfitModule` requests, preserving the existing non-atomic per-module send
semantics. Godot retains row rendering, hit testing, drag geometry, and scene
effects. The old station-inventory string action tags and dedicated connection
wrappers are removed only because this typed path replaces them completely.
