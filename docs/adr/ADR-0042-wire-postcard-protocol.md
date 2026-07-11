---
id      : ADR-0042
title   : クライアント⇔サーバー ワイヤプロトコルを WebSocket + postcard バイナリへ移行（段階1: Event/Command）
status  : accepted
date    : 2026-07-11
deciders: [human, ai-agent]
related : ADR-0007（マルチプレイヤー対応設計・WebSocket+JSON採用）, ADR-0039（dawn-client-core）,
          ADR-0040（dawn-client-gdext）, ADR-0041（dawn-wire・コマンド送信のGDExtension化）,
          docs/process/roadmap.md §13（Phase 10 タスク4）
---

# ADR-0042 — クライアント⇔サーバー ワイヤプロトコルを WebSocket + postcard バイナリへ移行

## 背景

ADR-0007（Phase 5）は「gRPC への移行を行わず WebSocket + JSON を維持する。gRPC は
Phase 9 以降（分散ノード間通信が必要になったとき）に再検討する」と決定した。

このトリガーは既に発火し、解決済みである——ただし WebSocket+JSON ではなく、
`dawn-consensus`/`dawn-replication` の `TcpRaftTransport`/`TcpReplicationTransport`
（生TCP + `[u32 LE length][postcard message]` フレーミング）という**別の**プロトコルとして
実装された。`docs/architecture/architecture.md` も「Inter-node: TCP」「Client: WebSocket+JSON」
と明確に分離済みである。つまり ADR-0007 の元のトリガー（分散ノード間通信）は
クライアント向け通信には当てはまらない。

一方、roadmap.md §13 タスク4は別の理由でクライアント⇔サーバー間の通信方式再検討を求めている:
GDExtension 導入（ADR-0039/0040/0041）でクライアント側も Rust 型を直接構築・
シリアライズできるようになったため、ワイヤ形式も JSON テキストではなく同じ型を
バイナリ化できる、という発展的な動機である。今後 Phase 9C/9D（Station 建造・Market/Currency）
で新規コマンド・イベントが増える前に、今のうちに見直す方が移行コストが低い。

## 決定

### トランスポートは WebSocket を維持し、ペイロードのみ postcard バイナリ化する

`Unreal Engine`（独自の信頼性付きUDP）や多くのAAAアクションゲームは低レイテンシ
UDP系プロトコルを使うが、Dawnはロック/オービット/ワープのような判断ベースの
操作が中心で、EVE Online同様FPS的な入力精度を必要としない。gRPC（HTTP/2）は
Godot側に組み込みサポートがなく、実装コストが動機（移行コストの低減）に見合わない。
`WebSocketPeer` は Godot 組み込みで既に機能しており、変更が必要なのはペイロードの
シリアライズ形式のみと判断する。

### シリアライズ形式は postcard

`dawn-consensus`/`dawn-replication` が既に使っており、依存が増えない。`serde` ベースなので
`dawn-wire`/`dawn-actor::protocol` の既存型定義（`#[derive(Serialize, Deserialize)]`）を
ほぼそのまま流用できる。FlatBuffers/protobuf のような新しいスキーマ言語の導入は
既存の serde ベースの型定義を全部書き直すことになり、動機に反する。

### フレーミング: WebSocket の1フレーム = 1メッセージ（追加の長さプレフィックス不要）

現状の実装を調査した結果、`ws_server.rs` は既に「1メッセージ = 1回の `Message::Text` send
= 1 WSフレーム」という設計になっている（`"\n"` 区切りはクライアント側の行バッファ処理の
名残で、実際のフレーミングは WebSocket 自体が提供している）。`dawn-consensus`/
`dawn-replication` の `[u32 LE length]` プレフィックスは生TCPソケット（フレーミングなし）
向けのものであり、WebSocket上では不要な二重の複雑さになる。`Message::Text(json+"\n")` を
`Message::Binary(postcard bytes)` に置き換え、クライアント側の行バッファ分割ロジックは撤去する。

### 統合 enum（`ServerMessage`/`ClientMessage`）を新設する

postcard は JSON と異なり自己記述的なタグを持たず、デシリアライズ側が先に型を
決め打ちする必要がある。現状 Hello/Welcome/InitialState/Redirect/DomainEvent/
ClientCommand はそれぞれ別々の型（JSON `"type"` 文字列で判別）になっているため、
`dawn-wire` に以下の統合 enum を新設する:

```rust
#[derive(Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome { player_id: u64, ship_id: u64 },
    Redirect { ws_addr: String, player_id: u64, ship_id: u64 },
    Event(EventJson),
}

#[derive(Serialize, Deserialize)]
pub enum ClientMessage {
    Hello { resume: Option<ResumeIdentity> },
    Command(ClientCommandJson),
}
```

受信側はまずこの外側の enum として postcard デコードし、中身の variant で分岐する。

### `EventJson`/Hello関連の型も `dawn-wire` に統合する

ADR-0041 が `ClientCommandJson` を `dawn-wire` に切り出した理由（`dawn-client-gdext` が
tokio 等のトランスポート依存を持たずに同じ型を扱える）は `EventJson` にも同様に当てはまる
——将来 GDExtension 側で DomainEvent 受信をデコードする実装（roadmap タスク2の残り
チャンネル）が入る際、同じ型を再利用できる。`hello_resume.rs` の `ResumeIdentity` も
`ClientMessage::Hello` の一部として `dawn-wire` に移動する。`dawn-actor::protocol` は
（ADR-0041 のときと同様）薄い再エクスポート層になる。

`EventJson` は現状 `Serialize` のみだが、`Deserialize` も追加する（クライアント側で
`ServerMessage` をデコードするため、`ClientCommandJson` に `Serialize` を追加した
ADR-0041 と対称の変更）。

### 段階1のスコープ: 型が既に決まっている部分のみ

調査の結果、`InitialState`/`AoiEnter`/`PlayerLoadout` は固定の Rust 構造体ではなく、
`crates/dawn-sector/src/node/serialization.rs`/`aoi.rs`/`player_loadout_projection.rs`
等で `serde_json::json!` マクロにより動的に組み立てられた自由形式 JSON である
（船・天体・星系・ゲート・ステーション・建造可能船種などの可変長リストを含む）。
これらを固定型に起こす作業は当初想定より大きく、影響ファイルは dawn-sector 側
（`serialization.rs`/`aoi.rs`/`ship_registry.rs`/`player_loadout_projection.rs`/
`client_admission.rs`/`runtime.rs`）と dawn-simulation 側（`aoi_delivery.rs`/
`cluster.rs`/`single.rs`）に及ぶ。8D 最小化方針に従い、本 ADR の実装（段階1）では
**`ServerMessage`/`ClientMessage`（Welcome/Redirect/Event/Hello/Command、既に型が
決まっている高頻度メッセージ）だけを postcard バイナリ化**し、`InitialState`/
`AoiEnter`/`PlayerLoadout` は**引き続き JSON テキストフレームとして送る**
（`WsClientConnection::send_raw` はそのまま）。WebSocket は同一接続上で
text/binary 両方のフレーム種別を送受信できるため、この共存は技術的に問題ない。
実際のゲームプレイ中のトラフィックの大部分（DomainEvent ストリーム + コマンド送信、
1接続あたり1回しか送らない InitialState/PlayerLoadout とは対照的に毎ティック発生する）
はこれで postcard 化される。

段階2（`InitialState`/`AoiEnter`/`PlayerLoadout` を固定型に起こして統合 enum に合流させる）
は本 ADR の実装チェックリストに残タスクとして記録し、別セッション・別PRで着手する。

### クライアント側の受信デコード責務は `dawn-client-gdext` に置く

GDScript 自身は postcard をデコードできない。`connection.gd::_flush_buffer` の
`JSON.parse_string(line)` 相当の変換ステップを Rust 側（`dawn-client-gdext`）に
持たせる。既存の `Dict = Dictionary<Variant, Variant>` 変換パターン
（`loadout_gd.rs`/`module_row_gd.rs`/`item_row_gd.rs`）と同様、
`ServerMessage` のバイト列を受け取り `Dictionary` に変換する GDExtension クラス
（`ServerMessageDecoder` 等）を新設し、`connection.gd` は `WebSocketPeer` から
バイナリパケットを受け取ってこの関数に渡すだけにする。

## 却下した案

- **gRPC（HTTP/2）へのトランスポート全体の置き換え**: Godot に組み込みサポートがなく、
  実装コストが動機に見合わない。「Godot以外の外部クライアントに公開する」といった
  別要求が出るまで再検討しない。
- **JSON対応を残したデュアルパス**: 実際に存在しない「外部非Godotクライアント」の
  ために両対応を維持するのは仮想要件対応であり、今回の動機（機能追加前の移行
  コスト低減）に反する。プレ・リリースにつき直接的な破壊的変更を許容する。
- **`dawn-consensus`/`dawn-replication` と同じ長さプレフィックスフレーミング**:
  WebSocket が既にメッセージ境界を提供しているため不要な二重の複雑さ。
- **段階1で `InitialState`/`AoiEnter`/`PlayerLoadout` も含めて全部postcard化**:
  自由形式 JSON を固定型に起こす作業が当初想定より大きく、1セッションでは
  中途半端な状態でのコミットになるリスクが高い。8D 最小化方針に従い分割した。

## 実装チェックリスト

### 段階1（本PR）

- [x] `dawn-wire`: `postcard` 依存追加（workspace依存を流用）
- [x] `crates/dawn-actor/src/protocol/server_event.rs` を `dawn-wire/src/server_event.rs`
      へ移動、`EventJson` に `Deserialize` を追加
- [x] `crates/dawn-actor/src/protocol/hello_resume.rs` を `dawn-wire/src/hello_resume.rs`
      へ移動、`ResumeIdentity`/`HelloMessage` に `Serialize`/`Deserialize` を追加
- [x] `dawn-wire`: `ServerMessage`/`ClientMessage` 統合 enum を新設
- [x] `dawn-actor::protocol`: `pub use dawn_wire::{...}` で再エクスポート（ADR-0041と同じパターン）
- [x] `dawn-actor/src/ws_server.rs`: `Message::Text(json+"\n")` を
      `Message::Binary(postcard::to_stdvec(&ServerMessage::...))` に置き換え。
      `InitialState`/`AoiEnter`/`PlayerLoadout`（`send_raw`）は現状のJSON `Message::Text`
      のまま維持
- [x] `dawn-actor/src/ws_server.rs`: 受信側 `Message::Binary` を
      `postcard::from_bytes::<ClientMessage>` でデコード
- [x] `crates/dawn-client-gdext`: `ServerMessage` デコード用 GDExtension クラス新設
      （postcardバイト列 → `Dictionary`、`ServerMessageDecoder`）。テスト用に対称の
      `ClientMessageDecoder`（`ClientMessage` → `Dictionary`）も追加
- [x] `client/scripts/connection.gd`: バイナリパケット受信パスに対応、
      `_flush_buffer` の改行分割ロジックを撤去（Welcome/Redirect/DomainEvent/
      コマンド送信はbinary、InitialState/PlayerLoadout/AoiEnterはtextのまま）
- [x] GdUnit4: 新しいデコードクラスの契約テスト
- [x] `docs/architecture/wire-protocol.md` 更新（binary/text混在の設計を明記）
- [x] `cargo fmt --all -- --check` / `cargo test --workspace` /
      `cargo clippy --workspace -- -D warnings` 全件通過

**実装中に判明した追加の技術的制約**: postcard は内部タグ付きenum
（`#[serde(tag = "type")]`）を一切デシリアライズできない（`deserialize_any`が
実装されていないため）。`ClientCommandJson`/`EventJson`は両方ともこのタグ付けを
使っていたため、実装中に`ClientMessageDecoder`のデコードが
"This is a feature that PostCard will never implement" エラーで失敗することが
判明した。対応として両enumを外部タグ付き（serdeのデフォルト、
`{"VariantName": {...fields}}`）に変更し、`dawn-client-gdext`側の
デコーダー（`ServerMessageDecoder`/`ClientMessageDecoder`）で
`{"type": "VariantName", ...fields}`という従来のDictionary形状に変換し直すことで、
GDScript側の既存コンシューマ（`main.gd`/`hud_manager.gd`）には影響を与えずに
吸収した。`docs/architecture/wire-protocol*.md`のJSONスキーマ表現もこの形状変更に
合わせて更新済み（実際のワイヤはpostcardバイナリなので、JSONスキーマは
フィールド形状のドキュメントとしてのみ意味を持つ）。

### 段階2（後続タスク・別PR）

- [ ] `InitialState`/`AoiEnter`/`PlayerLoadout` を固定 Rust 構造体に起こす
      （`serialization.rs`/`aoi.rs`/`ship_registry.rs`/`player_loadout_projection.rs`/
      `client_admission.rs`/`runtime.rs`/`aoi_delivery.rs`/`cluster.rs`/`single.rs`）
- [ ] 上記を `ServerMessage` に合流させ、`send_raw`（JSON text）経路を撤去
- [ ] `WsClientConnection::send_raw` の削除（全メッセージが `ServerMessage` 経由になった時点で）
