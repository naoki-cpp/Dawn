---
scope    : Dawnサーバーのセキュリティレビュー — 解消済みfindingの作業ログ
audience : AI Agent / Human Developer
update   : /security-check がfindingを解消済みへ移動するたびに追記
related  : docs/architecture/security-review.md（未解消のfinding・検証済み健全項目）,
           .agents/skills/security-check/SKILL.md
date     : 2026-07-11
---

# Security Review — Dawn Server（解消済みログ）

[security-review.md](./security-review.md)で解消済みとなったfindingを時系列で記録する。
**分析のみ。過去分の削除・改変は行わない（監査ログとして追記のみ）。**

---

## 解消済み

### SEC-5（medium・2026-07-11解消）: 未検証の浮動小数点入力が共有シミュレーション状態に到達

`crates/dawn-actor/src/protocol/client_command.rs`の`MoveCommand`(`PosJson.x/y/z`)・
`OrbitCommand`(`radius`)・`KeepAtRangeCommand`(`range`)は`is_finite()`検証なしで
`commands.rs`の`apply_move_command`等に渡り、`dest_in_ship_frame_abs`→
`steer_thrust_toward`の位置演算に直接使われていた。NaN/Infinityを注入すると
位置・速度がNaN汚染され、以降の距離比較（`dist < range`等）はNaNとの比較が常に
falseになるため範囲判定系のロジックも黙って壊れる。イベント経由で他クライアントにも
伝播する。

**修正**: `PosJson::is_finite()`を追加し、`client_command_from_json`で
`MoveCommand`/`OrbitCommand`/`KeepAtRangeCommand`のパース時に非有限値を`None`（拒否）
として弾く。回帰テスト3件（overflowするJSON数値リテラル`1e40`が`f32`パース時に
`f32::INFINITY`になることを利用 — JSON自体に`NaN`/`Infinity`リテラルは存在しないため）
を`crates/dawn-actor/src/protocol/mod.rs`に追加。`cargo test -p dawn-actor`
58/58 pass確認済み。

### SEC-4（medium・2026-07-11解消）: 無制限のper接続コマンドキュー

`crates/dawn-actor/src/ws_server.rs`の`command_tx`/`command_rx`が
`mpsc::unbounded_channel::<ClientCommand>()`で、パース成功した全コマンドを
無制限に積んでいた。ドレイン側（`dawn-sector-node/src/runtime.rs`）は
`while let Some(cmd) = sess.try_recv_command()`で1tickにつき1セッション分を
全消費するため、高速に妥当なコマンドを送り続けるクライアントはサーバーメモリを
無制限に増やせ、かつ他セッションのtick処理を独占的に遅延させられた。フレーム
サイズ上限（SEC-1）はメッセージ1個の大きさしか制限せず、この「キュー深さ」問題への
歯止めにはならない。

**修正**: `mpsc::unbounded_channel`を`mpsc::channel(COMMAND_QUEUE_CAP)`
（256、TICK_MS=100msで数秒分のバッファに相当）へ変更。送信側を`.send(cmd).await`に
変更し、キューが詰まればソケット読み取りタスク自体が一時停止し、TCPレベルの自然な
backpressureがかかる（切断・ドロップロジックを新設する必要がない）。この
ファイルには元々ユニットテストが皆無（生ソケットのWS統合コードで、テストしやすい
ロジックは`InProcessConnection`側に分離済み）のため、新規テストは追加せず
`cargo build`成功 + 既存ワークスペーステストスイートで動作確認。

### SEC-3（medium・2026-07-11解消）: `transfer_to_station_owned`が船側のドック検証を欠落

`crates/dawn-sector/src/node/inventory.rs`の`transfer_to_station_owned`は
`owns_ship`とプレイヤー側`can_use_station`は検証していたが、`cmd.ship_id`自身が
`cmd.station_id`にドックしているかを検証していなかった。`can_use_station`は
`docked_players`（プレイヤー単位）しか見ないため、ADR-0037の複数所有船モデルの下で、
アクティブ船である`ship_id`でどこかのステーションにドックしているプレイヤーが、
**別の**（そのステーションにドックしていない、あるいは宇宙空間の）所有船のcargoを
そのステーションへ瞬間移動できた。模範実装の`disassemble_ship_owned`（両側のドック先を
検証済み）から逸脱していた。

**修正**: `disassemble_ship_owned`と同じ`docked_station(cmd.ship_id) != Some(cmd.station_id)`
チェックを追加。回帰テスト
`transfer_to_station_owned_is_rejected_when_the_ship_itself_is_not_docked_at_the_station`
を追加（`cargo test -p dawn-sector`確認済み、7/7 pass）。
