---
scope    : Dawnサーバーの信頼できない入力経路のセキュリティレビュー（OWASP準拠）— 未解消のfinding・検証済み健全項目
audience : AI Agent / Human Developer
update   : /security-check で状態が変わるたびに更新
related  : .agents/skills/security-check/SKILL.md,
           .agents/skills/security-check/references/owasp-map.md,
           .agents/skills/security-check/references/baseline.md（初回レビューの凍結記録）,
           docs/architecture/security-review-completed.md（解消済みfindingの作業ログ）
date     : 2026-11-03
---

# Security Review — Dawn Server（OWASP観点）

2026-11-03 scoped client-lifecycle update: pure Rust owns Welcome gating,
bounded retry/log timing and independent 90-second start/reconnect/Redirect
deadlines. Opening a socket never extends an unfinished attempt. Cancel and
timeout clear adapter-owned invite/ticket bytes; late Welcome cannot restore
a cancelled identity. Existing endpoint validation and exact credential pinning
are retained and exercised through the real connection with a controlled socket.
No SQL, server admission authority, wire type or automatic gameplay-command
retry was added. Client elapsed time rejects non-finite/negative inputs. No new
security finding was introduced by this client-only change.

2026-11-02 scoped public-playtest update (ADR-0056): both `simulate --serve`
and `simulate --serve --cluster` share ingress in `serve/playtest.rs`, requiring
validated individual invites when `PORT` exposes the listener, and rejects
unauthorized requests before upgrade/Hello. `subtle` compares bearer tokens;
Debug/errors do not disclose secrets. Upgrade and Hello each time out after
3 seconds; process-wide handshakes and their shared pending queue are capped at eight.
Public denials do not emit per-attempt logs. Client credentials are restricted
to WSS (or explicit loopback tests) and are not forwarded across redirects.
All client listeners now explicitly cap incoming frames/messages at 64 KiB.
Token gates do not supply a PlayerId or replace ResumeTicket verification.
No wire schema, SQL query, ownership check or command validation changed.
Tests exercise HTTP rejection, two valid invitations, invalid/duplicate
configuration, redaction, oversized input and client endpoint pinning.
Real-binary cloud-style tests also cover missing/invalid invitation rejection,
fail-closed startup, a gate handoff on one socket, and ResumeTicket routing to
the destination Sector. The existing consensus/ownership and command-validation
paths are unchanged. Cloud Market storage is fresh per process in both modes;
no SQL query changed. Population limits remain per Sector, not a global resource
quota. No new finding was introduced by the in-process multi-Sector extension.

This limited trusted-tester configuration is not an internet-hardened
production deployment: admitted output queues, long-lived InMemoryJournal
growth and broader admission fairness remain resource risks. Do not broaden
the audience without a full public-internet security/resource review.

2026-09-01 SQL-only update: Issue #345 replaced Market's whole-state reload
and rewrite with indexed, command-scoped working sets and transactional delta
writes. SQL identifiers remain fixed, all values remain parameter-bound, and
matching candidates, public order views, and pending-settlement pages now have
explicit resource bounds. Ownership and authoritative price-time matching stay
inside the unchanged pure Market policy. No new security finding was introduced.

2026-08-26 SQL-boundary update: Issue #344 moved existing admission,
identity, and Station projection queries into private bounded-context
repository modules. The schema, single-connection transaction boundary,
fixed SQL identifiers, and parameter binding are unchanged. No new input
surface or security finding was introduced.

2026-08-26 command-path update: Issue #343 moved authenticated requests into
the prepare -> durable commit -> live apply frame boundary. Session identity
and output-routing indexes remain server-derived, `ClientRequest::validate()`
and ownership checks still run before speculative mutation, and the existing
bounded per-session queues are unchanged. No new security finding was
introduced.

2026-08-22 SQL-only update: the production Station projection added an
idempotency range column, a fixed-schema migration, and transactional ordered
stack mutations. SQL identifiers remain static, every transition/range/item
value is parameter-bound or derived from a closed server enum, and invalid
zero/underflowing mutations roll back the row changes and cursor together. No
new security finding was introduced.

2026-08-20 update: Recovery/Public replication cursor separation changed
peer catch-up metadata to carry two fixed-width positions. Peer protocol v2
rejects mixed layouts during the handshake; existing control/bulk frame caps,
bounded channels, Sector identity checks, suffix event limits, and snapshot
byte limits remain in force. No new security finding was introduced.

2026-08-17 update: Issue #317 replaced cargo/admission event snapshots with
`BTreeMap<ItemId, u64>` stack rows, removing per-unit expansion. Market wire
orders now reject values above SQLite's signed 64-bit range at admission; no
new security finding was introduced.

2026-08-10 update: Issue #279 added the SQLite-backed Market repository and
the settlement adapter. SQL remains parameterized, Market order destinations
are ownership-checked before state mutation, and the new settlement IDs are
validated before Sector delivery; no new security finding was introduced.

2026-08-10 update: Issue #281 changed the client-side construction boundary
and made `MarketOrderSide` a closed wire enum. Server admission, ownership
checks, SQL parameter binding, scalar limits, and bounded per-session command
queues were re-verified; no new security finding was introduced.

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
  ADR-0056の招待制playtestは例外で、Render TLS終端と入口token検証を必須にする。
- クライアント側コード（GDScript・クライアント側Rustクレート）は信頼境界ではない。

---

## エントリポイント一覧

| 経路 | ファイル | 内容 |
|---|---|---|
| WebSocketフレーム受信 | `crates/dawn-server/src/ws_server.rs` | postcardバイナリフレーム（ADR-0042） |
| 招待制WebSocket upgrade | `crates/dawn-server/src/ws_server.rs` + `serve/playtest.rs` | 個別bearer token、PORT設定fail-closed、Hello前の401、ADR-0056 |
| コマンドデコード | `crates/dawn-protocol/src/client_request.rs` + `dawn-core/src/commands.rs` | typed `ClientRequest` |
| Marketデコード | `crates/dawn-protocol/src/market.rs` | `MarketCommandWire`（Sector commandとは別queue） |
| Hello/resumeハンドシェイク | `crates/dawn-protocol/src/hello_resume.rs` | セッション識別（resume identity） |
| コマンドディスパッチ | `crates/dawn-sector/src/node/commands.rs` | one exhaustive typed `ClientRequest` admission seam calling family-local policy directly |
| Market bridge | `crates/dawn-server/src/serve/market.rs` | 入力検証、所有船へのRemove/Return/Credit適用 |
| Market SQL | `crates/dawn-market/src/repository.rs` | `MarketDb`の注文帳/Currency台帳、全値をparameter binding |
| ノード間トランスポート | `crates/dawn-distributed`（control/bulk）とRaft/replication adapters | フレーム長上限・identity handshakeあり、無認証（LAN方針内） |

---

## 検証済み・健全

### A03 SQLインジェクション — dawn-sector repository boundary

Repository schema setup and bounded-context queries are `params![]`でパラメータ化済み。
テーブル/カラム名はクライアント入力から導出されない（`item_id_to_columns`は`ItemId` enumへの
閉じたmatch）。詳細は
[baseline.md](../../.agents/skills/security-check/references/baseline.md)参照。

2026-08-22のStation projection差分も、schema migrationを含めSQL文字列は固定で、
`transition_id`、journal range、Player/Station/Item値、deltaはparameter bindingを使用する。
projection cursorとstack mutationは一つのSQLite transactionでcommitされる。

### A03 非SQLインジェクション

クライアント文字列は全てclosedなmatchでenum化され、パス・シェル・フォーマット文字列への
埋め込みなし。

`dawn-market/src/repository.rs`のSQLも、Marketの閉じたenum変換からサーバー内部で組み立てる
比較・並び順以外の値は全て`params![]`で束縛する。クライアント文字列をSQL識別子へ
通さず、Market item/sideもruntimeのclosed matchで変換する。

### A01 アクセス制御 — `_owned`ハンドラ群

`fit_module_owned`/`unfit_module_owned`/`build_packaged_ship_owned`/`disassemble_ship_owned`は
状態変更前に`owns_ship`+ドック状態を検証済み。`dock_owned`等は`active_ship`解決経由でそもそも
クライアント供給IDを信頼しない設計。

MarketのPlaceは`MarketRuntime`で`owns_ship(player_id, ship_id)`とプレイヤーと同じStationへの
ドック状態をRemove前に確認し、
Cancelは`MarketDb`が注文所有者を確認した後、保存済みの船IDを`return_item_owned`へ渡す。
single/clusterともCreditの宛先は、所有かつドック済みの船を持つノードに限定される。

### A04 コマンド層のアロケーション

`ClientRequest`はtyped IDs/enumsと固定形payloadのみ。クライアント供給カウントが駆動する無制限ループ/アロケーションはなく、非有限座標・半径はqueue投入前に構造化拒否される。Market wireのprice/quantityはSQLiteのsigned 64-bit範囲を超える値もqueue投入前に拒否される。

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

SEC-1は受信上限の明示化で解消。SEC-2の記述はADR-0048導入前の旧Hello形式を参照しており、
現行のResumeTicket検証経路に該当しないことを再確認した。履歴はcompletedを参照する。

---

解消済みfinding（SEC-3/SEC-4/SEC-5）の詳細な経緯・修正内容は
[security-review-completed.md](./security-review-completed.md)を参照。
