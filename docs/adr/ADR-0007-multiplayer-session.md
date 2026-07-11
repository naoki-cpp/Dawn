---
id      : ADR-0007
title   : マルチプレイヤー対応設計（Phase 5）
status  : accepted
date    : 2026-06-05
deciders: [human, ai-agent]
related : ADR-0005（ClientConnection）, ADR-0006（Fitting/Combat）
---

# ADR-0007 — マルチプレイヤー対応設計（Phase 5）

## 背景

Phase 4 の仕様チェック（2026-06-05）で、現在の実装がシングルプレイ専用に
なっている箇所を特定した。Phase 5 移行前に設計方針を確定する。

---

## 決定一覧

### 1. プロトコル：WebSocket を継続する

```
決定: gRPC への移行を行わず、WebSocket + JSON を維持する。
理由: 現在のボトルネックはトランスポート効率ではない。
     Godot 側の変更コストを最小化する。
     gRPC は Phase 9 以降（分散ノード間通信が必要になったとき）に再検討する。
```

> **2026-07-11 追記（ADR-0042）**: 上記の再検討トリガー（分散ノード間通信）は
> 既に発火し、解決済みである——ただし本ADRが指していたクライアント向け
> WebSocket ではなく、`dawn-consensus`/`dawn-replication` の TCP+postcard
> という別のプロトコルとして実装された。クライアント向け WebSocket は
> トランスポートとして維持しつつ、ペイロードを JSON テキストから postcard
> バイナリへ段階的に移行している（ADR-0042 段階1）。詳細は ADR-0042 参照。

### 2. 接続ハンドシェイク：Hello / Welcome メッセージを導入する

ORIGIN 座標シグナル（Phase 4 の暫定実装）を廃止し、
明示的なハンドシェイクに置き換える。

```
接続フロー:
  1. クライアントが WebSocket に接続する
  2. クライアントが Hello を送信する
        {"type": "Hello"}
  3. サーバーが PlayerId と ShipId を採番して Welcome を返す
        {"type": "Welcome", "player_id": N, "ship_id": N}
  4. サーバーが InitialState を送信する（§4 参照）
  5. 以降、通常の Tick ループで DomainEvent を受信する
```

#### 2-A. 現在の追加仕様: Redirect 後の resume Hello

8D の `dawn-sector-node` では、Sector Transit によりプレイヤー船が別の
物理ノードへ移動したとき、現在のノードが `Redirect` を送信して Godot
クライアントを宛先 WebSocket に誘導する。

```
Redirect:
  {
    "type": "Redirect",
    "ws_addr": "127.0.0.1:7879",
    "player_id": 7,
    "ship_id": 504403158265495553
  }

Resume Hello:
  {
    "type": "Hello",
    "player_id": 7,
    "ship_id": 504403158265495553
  }
```

Redirect を受けたクライアントは `ws_addr` に再接続し、保持している
`player_id` / `ship_id` を Hello に含める。宛先 `dawn-sector-node`
は Hello を読んでからセッションを確定し、その `ship_id` が自 Sector
内に存在する場合だけ `player_id` の所有 ship として adopt する。

新規クライアントは従来どおり `{"type":"Hello"}` を送る。この場合、
サーバーは新しい `PlayerId` / `ShipId` を採番して fresh spawn する。

この resume はクライアント接続の再確立だけを扱う。Sector 間の所有権移動
そのものは ADR-0014 の consensus path と `dawn-sector` の transit/import
処理を通る。宛先 Sector に ship が存在しない resume Hello は拒否し、
fresh spawn にフォールバックしない。これは同じプレイヤー ship の重複生成を
防ぐためである。

### 3. PlayerId の管理：接続レイヤーで保持する（Option B）

```
決定: ClientCommand に player_id フィールドを追加しない。
     WsServer が「この接続 = この PlayerId」を管理し、
     コマンドを受け取った時点で PlayerId を付与してサーバーに渡す。

理由: Godot 側のコマンド JSON を変更しなくてよい。
     ClientCommand の型定義も変わらない。
     ADR-0005 のインターフェースを維持できる。

実装イメージ:
  struct PlayerSession {
      player_id : PlayerId,
      ship_id   : ShipId,
      conn      : WsClientConnection,
  }

  // コマンド受信時
  while let Some(cmd) = session.conn.try_recv_command() {
      match cmd {
          ClientCommand::Move(mv) => {
              // player_id で所有権を検証してから処理
              node.apply_move_command(session.player_id, mv.ship_id, mv.target_position);
          }
          ClientCommand::LockOn(lo) => {
              node.apply_lock_on(session.player_id, lo);
          }
      }
  }
```

### 4. 初期状態同期：InitialState メッセージを導入する（Option B）

```
決定: DomainEvent とは別に InitialState メッセージを送信する。
     イベントストリームとスナップショットを概念的に分離する。

InitialState の内容:
  {
    "type": "InitialState",
    "ships": [
      {
        "ship_id"   : N,
        "position"  : {"x": F, "y": F, "z": F},
        "max_hp"    : F,
        "current_hp": F
      },
      ...
    ]
  }

理由:
  - 接続後に戦闘中の世界に入っても HP が正しく表示される
  - DomainEvent のスキーマを変更しなくてよい（INV-002 / §7 を遵守）
  - Replay とクライアント初期化を別の概念として明確に分離する
```

### 5. イベント配信：グローバルブロードキャストを維持する

```
決定: 全クライアントに全 DomainEvent を配信する（Phase 4 と同じ）。
     Interest Management（近くのイベントのみ送る）は Phase 8 以降。
理由: 正しく動くことを優先する。最適化は後から。
```

### 6. Phase 4 卒業基準

Phase 5 着手は以下の条件を全て満たした時点とする。

```
□ 2クライアントが同時に接続できる
□ 両クライアントの世界状態が同期している（ShipMoved が両方に届く）
□ プレイヤーのロックオン操作が機能する
□ 再接続後に InitialState で状態が復元される
□ 基本的なゲームループ（移動・ロック・戦闘）でクラッシュしない
```

---

## 実装チェックリスト（Phase 5 着手時に使用）

### サーバー側

- [x] `dawn-core`: `PlayerId(u64)` 型追加
- [x] `dawn-core`: `CommandError::NotOwner` 追加
- [x] `dawn-simulation/node.rs`: `player_ships: HashMap<PlayerId, ShipId>` フィールド追加
- [x] `dawn-simulation/node.rs`: `spawn_player_ship(player_id)` メソッド追加
- [x] `dawn-simulation/node.rs`: 全コマンドに `PlayerId` + 所有権チェック追加
- [x] `dawn-simulation/ws_server.rs`: `Hello` メッセージのパース
- [x] `dawn-simulation/ws_server.rs`: `Welcome` メッセージの送信
- [x] `dawn-simulation/ws_server.rs`: `InitialState` メッセージの送信
- [x] `dawn-simulation/ws_server.rs`: `PlayerSession` 構造体でセッション管理
- [x] `dawn-simulation/ws_server.rs`: 複数クライアントの同時接続対応（accept ループ）
- [x] `dawn-actor/protocol.rs`: Redirect に resume identity（`player_id` / `ship_id`）を含める
- [x] `dawn-actor/ws_server.rs`: Hello の optional resume identity をパースする
- [x] `dawn-sector-node`: Redirect resume Hello で宛先 Sector の既存 ship を adopt する
- [x] `dawn-simulation/main.rs`: ORIGIN シグナル処理を削除
- [x] `dawn-simulation/ws_server.rs`: `AttackCommand` JSON パーサー追加
- [x] 全テスト通過（138テスト）

### クライアント側（Godot）

- [x] `connection.gd`: 接続後に `Hello` を自動送信
- [x] `connection.gd`: `Redirect` を受けて宛先 WS に再接続し resume Hello を送信
- [x] `connection.gd`: `Welcome` を受け取り `player_id` / `ship_id` を保持・シグナル発行
- [x] `connection.gd`: `InitialState` を受け取り各 Ship の HP を初期化
- [x] `main.gd`: ORIGIN シグナル送信を削除
- [x] `main.gd`: `Welcome` シグナルを受けてプレイヤー船を設定

---

## 変更しない設計

```
ClientConnection trait のインターフェース（ADR-0005）
  → send_events / try_recv_command の 2 方向構成はそのまま

ClientCommand の型定義
  → player_id を含まない（接続レイヤーで管理）

DomainEvent のスキーマ
  → InitialState は DomainEvent ではない（専用メッセージ）

イベント配信方式
  → グローバルブロードキャスト（Interest Management は Phase 8）
```

---

## 参照

- ADR-0005: ClientConnection 抽象化
- ADR-0006: Fitting / Combat / Lock-on 設計
- ORIGIN 座標シグナル: Phase 4 の暫定実装（本 ADR の実装により廃止済み）
