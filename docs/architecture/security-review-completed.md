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


## 2026-11-02 — SEC-1 fixed; SEC-2 historical correction

SEC-1: all WebSocket accept paths now explicitly limit incoming frames and
messages to 64 KiB. Invited upgrade tests reject an oversized pre-Hello frame.
SEC-2: the finding below described raw player/ship claims before ADR-0048;
current Hello contains only an optional opaque ResumeTicket, and server-owned
admission resolution verifies that capability. This entry corrects the stale
pending record; this playtest change does not implement or weaken resume policy.

The previous pending descriptions are retained verbatim for audit history:

### SEC-1（low・トリガー付き保留）: WebSocketサイズ上限が暗黙値

`crates/dawn-server/src/ws_server.rs`（359行）: `accept_async(stream)`が`WebSocketConfig`を
明示せず呼ばれており、`max_message_size`/`max_frame_size`がtokio-tungsteniteのライブラリ
既定値（現行ピン留めバージョンで64MiB/16MiB）任せになっている。

**根本原因**: サイズ上限を意識的に選んだのではなく、ライブラリのデフォルトに乗っているだけ。
**判断: 保留**。LANでは悪用してもメモリ圧迫止まりで実害は小さい。修正は
`accept_async_with_config`呼び出し1箇所で完結する軽微な変更。
**再評価トリガー**: 公開準備が始まったとき、または他の理由でこのファイルに手を入れる機会があるとき
（ついでに直す）。

### SEC-2（high・トリガー付き保留）: Hello resumeでの船の乗っ取り

`crates/dawn-protocol/src/hello_resume.rs` → `crates/dawn-server/src/bin/sector-node/client_admission.rs`
(`select_handshake_identity`) → `crates/dawn-sector/src/node/spawner_logic.rs`
(`adopt_player_ship`)。

クライアントがHelloに生の`player_id`/`ship_id`を載せるだけでresumeが成立し、
`adopt_player_ship`は「そのship_idがこのノードのECSに存在するか」だけを確認して、
名乗った`player_id`を無条件に`ships.owners`へ上書きする。ship_id/player_idはワイヤ上で
他イベント（`Redirect`メッセージ等）を通じて露出しているため、攻撃者は他プレイヤーの
船を指定するだけで所有権を奪取でき、以降の`owns_ship`ベースの検証は全て素通しになる。

**根本原因（設計レベル）**: 調査の結果、これは単純な検証漏れではなく、より深い設計上の
ギャップだと判明した。ノード間転送（Sector Transit）で運ばれる
`TransitHandoffState`は`player_id`/所有権情報を一切含んでいない。
永続化用`ShipSnapshot`も同様にownership authorityではない。
つまり転送先ノードでは、船が実際に誰の所有かという情報がどこにも存在せず、
**クライアントのHello resumeが最初に主張した内容がそのまま所有権の確立になる** —
これは`adopt_player_ship`のdocコメント通りの意図された設計であり、認証なしという
既存方針の必然的な帰結である。

修正案を2段階で検討した:
1. **狭い修正**: `TransitHandoffState`にトランジット元ノードの正規`owners`から`player_id`を
   載せて転送先へ引き継ぎ、resume時は「そのペアが一致するか」だけを検証する。
   存在しないship_idや無関係なペアでの乗っ取りは防げるが、player_id/ship_idはワイヤ上で
   観測可能なため、正しいペアを知っている攻撃者はなお成りすませる — 部分的な緩和に留まる。
2. **本格修正**: 初回接続時にサーバーが秘密のセッショントークンを発行し、resume時に
   必須で照合する。これでようやく「知っているだけでは奪えない」状態になるが、
   トークンの発行・保持・失効という新しい状態管理が要る、ADRを伴う設計変更。

**判断: 保留（トリガー付き）。** LANプロトタイプで信頼できるプレイヤー同士という前提に
照らし、今すぐ本格的なセッション認証を実装するコストは見合わないと判断。ただし
「無認証」の既定方針とは異なる、能動的に権限を付与してしまう経路である点は記録として残す。

**再評価トリガー**: 公開準備（`THIRD-PARTY-LICENSES.md`のトリガーと同じタイミング）が
始まったとき、信頼できない参加者を想定する必要が生じたとき、またはこの経路が実際に
悪用された（悪用の兆候が観測された）とき。
