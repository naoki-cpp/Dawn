---
scope    : Dawnサーバーの信頼できない入力経路のセキュリティレビュー（OWASP準拠）— 未解消のfinding・検証済み健全項目
audience : AI Agent / Human Developer
update   : /security-check で状態が変わるたびに更新
related  : .agents/skills/security-check/SKILL.md,
           .agents/skills/security-check/references/owasp-map.md,
           .agents/skills/security-check/references/baseline.md（初回レビューの凍結記録）,
           docs/architecture/security-review-completed.md（解消済みfindingの作業ログ）
date     : 2026-08-09
---

# Security Review — Dawn Server（OWASP観点）

2026-08-09 update: #277の`dawn-sector` repository分割とStation projection APIを
レビューした。SQLは引き続き全てparameter bindingで、projectionのtransition indexは
SQLite INTEGER境界を検証する。ownership・入力値・queue/snapshot上限は健全。
既存のSEC-1/SEC-2保留findingは変更なし。

`/security-check`スキルによる、クライアントからの信頼できない入力がネットワークフレームから
ゲーム状態変更に至るまでの経路レビュー。このファイルは「今どういう状態か」
（エントリポイント・検証済み健全項目・未解消のfinding）だけを扱う。解消済みfindingの
詳細な経緯は[security-review-completed.md](./security-review-completed.md)（追記専用の
監査ログ）を参照 — `architecture-review`系ドキュメントの
`server.md`/`server-completed.md`分割と同じ運用。

既知のスコープ境界（再指摘しないこと）:
- **TLSなし・認証なし**はLANオンリープロトタイプの明示的な決定
  （`docs/architecture/architecture-review/server-pending.md`「採らない方針」）。
  公開準備が始まるまでは対象外。
- クライアント側コード（GDScript・クライアント側Rustクレート）は信頼境界ではない。

---

## エントリポイント一覧

| 経路 | ファイル | 内容 |
|---|---|---|
| WebSocketフレーム受信 | `crates/dawn-actor/src/ws_server.rs` | postcardバイナリフレーム（ADR-0042） |
| コマンドデコード | `crates/dawn-wire/src/client_request.rs` + `dawn-core/src/commands.rs` | typed `ClientRequest` |
| Marketデコード | `crates/dawn-wire/src/market.rs` | `MarketCommandWire`（Sector commandとは別queue） |
| Hello/resumeハンドシェイク | `crates/dawn-wire/src/hello_resume.rs` | セッション識別（resume identity） |
| コマンドディスパッチ | `crates/dawn-sector/src/node/commands.rs` | one exhaustive typed `ClientRequest` admission seam calling family-local policy directly |
| Market bridge | `crates/dawn-simulation/src/serve/market.rs` | 入力検証、所有船へのRemove/Return/Credit適用 |
| Market SQL | `crates/dawn-market/src/order_book.rs` | `MarketDb`の注文帳/Currency台帳、全値をparameter binding |
| ノード間トランスポート | `crates/dawn-peer-transport`（control/bulk）とRaft/replication adapters | フレーム長上限・identity handshakeあり、無認証（LAN方針内） |

---

## 検証済み・健全

### A03 SQLインジェクション — `crates/dawn-sector/src/node/repositories.rs`

Repository queryは`params![]`によるパラメータ化済み。テーブル/カラム名はクライアント入力から
導出されない（`item_id_to_columns`は`ItemId` enumへの閉じたmatch）。詳細は
[baseline.md](../../.agents/skills/security-check/references/baseline.md)参照。

### A03 非SQLインジェクション

クライアント文字列は全てclosedなmatchでenum化され、パス・シェル・フォーマット文字列への
埋め込みなし。

`dawn-market/src/order_book.rs`のmatching SQLも、`OrderSide`からサーバー内部で組み立てる
比較・並び順以外の値は全て`params![]`で束縛する。クライアント文字列をSQL識別子へ
通さず、Market item/sideもruntimeのclosed matchで変換する。

### A01 アクセス制御 — `_owned`ハンドラ群

`fit_module_owned`/`unfit_module_owned`/`build_packaged_ship_owned`/`disassemble_ship_owned`は
状態変更前に`owns_ship`+ドック状態を検証済み。`dock_owned`等は`active_ship`解決経由でそもそも
クライアント供給IDを信頼しない設計。

MarketのPlaceは`MarketRuntime`で`owns_ship(player_id, ship_id)`をRemove前に確認し、
Cancelは`MarketDb`が注文所有者を確認した後、保存済みの船IDを`return_item_owned`へ渡す。
single/clusterともCreditの宛先は`owns_ship`で検索したノードに限定される。

### A04 コマンド層のアロケーション

`ClientRequest`はtyped IDs/enumsと固定形payloadのみ。クライアント供給カウントが駆動する無制限ループ/アロケーションはなく、非有限座標・半径はqueue投入前に構造化拒否される。

`MarketCommandWire`もscalar-onlyで、per-sessionのMarket queueは既存command queueと同じ
256件のbounded channel。`MarketSnapshot`は最大200注文で、DBの`open_orders_for`にも同じ
LIMITを設定してからwireへ変換する。

### A08 データ整合性

コマンドはID/意図のみを運び、コスト・数量はサーバー側定数。移動系コマンドは目標地点のみで
物理演算はサーバーが権威を持つ。

Marketのprice/quantityはプレイヤーの注文意図として受けるが、runtimeで0、
`price * quantity`のu64 overflow、未知のitem/sideを拒否する。Currency escrowと
matching結果は`MarketDb`が計算し、client supplied balance/trade resultは存在しない。

### A06 依存関係

`cargo audit`/`cargo deny`をCIで全PRに対して実行。

---

## 未解消のfinding

### SEC-1（low・トリガー付き保留）: WebSocketサイズ上限が暗黙値

`crates/dawn-actor/src/ws_server.rs`（~245行）: `accept_async(stream)`が`WebSocketConfig`を
明示せず呼ばれており、`max_message_size`/`max_frame_size`がtokio-tungsteniteのライブラリ
既定値（現行ピン留めバージョンで64MiB/16MiB）任せになっている。

**根本原因**: サイズ上限を意識的に選んだのではなく、ライブラリのデフォルトに乗っているだけ。
**判断: 保留**。LANでは悪用してもメモリ圧迫止まりで実害は小さい。修正は
`accept_async_with_config`呼び出し1箇所で完結する軽微な変更。
**再評価トリガー**: 公開準備が始まったとき、または他の理由でこのファイルに手を入れる機会があるとき
（ついでに直す）。

### SEC-2（high・トリガー付き保留）: Hello resumeでの船の乗っ取り

`crates/dawn-actor/src/protocol/hello_resume.rs` → `crates/dawn-sector-node/src/client_admission.rs`
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

---

解消済みfinding（SEC-3/SEC-4/SEC-5）の詳細な経緯・修正内容は
[security-review-completed.md](./security-review-completed.md)を参照。
