---
id      : ADR-0005
title   : ClientConnection — サーバー／クライアント通信の抽象化
status  : accepted
date    : 2026-06-05
deciders: [human, ai-agent]
---

# ADR-0005: ClientConnection — サーバー／クライアント通信の抽象化

## 状況

Phase 4 でゲームクライアント（Godot 4）をサーバーの `SectorSimulatorActor` に接続する必要がある。
Phase 5 では本物のネットワーク（gRPC/QUIC）に移行する予定であり、
その際にクライアント側（Godot GDScript）のコードを変更したくない。

## 決定

`ClientConnection` trait を `dawn-actor` クレートに定義し、
フェーズごとに実装を差し替える戦略を採用する。

```
Phase 4: InProcessConnection  — tokio::mpsc チャンネルで直結（ダミーネットワーク）
Phase 5: GrpcConnection       — tonic による本物のネットワーク（未実装）
```

## インターフェース設計

```rust
pub trait ClientConnection: Send + 'static {
    /// サーバー → クライアント: DomainEvent のストリーム配信
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), ConnectionError>;

    /// クライアント → サーバー: Command の受信（ノンブロッキング）
    fn try_recv_command(&mut self) -> Option<MoveCommand>;
}
```

責務はこの **2 方向のみ** とする。

## 根拠

### なぜ 2 方向だけか

EVE Online のサーバーアーキテクチャを参考に、以下を分離する。

```
サーバー → クライアント: 世界で起きた事実（DomainEvent）のブロードキャスト
クライアント → サーバー: ユーザーの意図（MoveCommand）のポイント送信
```

クエリ（位置取得・状態参照）はクライアントがローカルの EventStream を
リプレイして保持する Read Model で対応する。
サーバーへのクエリは追加しない（スケール阻害要因のため）。

### なぜ dawn-actor に置くか

Dependency DAG:
```
dawn-core ← dawn-event-store ← dawn-actor ← dawn-simulation
```

`ClientConnection` は `DomainEvent` と `MoveCommand` を扱うため
`dawn-core` に依存する。
`InProcessConnection` は `tokio::mpsc` を使うため非同期ランタイムに依存する。
これらを満たす最下位クレートが `dawn-actor` である。

`dawn-core` には追加しない。`dawn-core` はドメインモデル定義のみ（FBD-002）。

### なぜ非同期 trait にしないか

`send_events` は unbounded channel への送信であるため、
await point を持つ必要がない。ブロッキングしない同期 API で十分。

`try_recv_command` は名前が示す通りノンブロッキング。
Actor のメインループ（Tick 処理）内でポーリングするため、
async にすると Actor のセレクトロジックが複雑になる。

### Phase 5 での差し替え

```rust
// Phase 4（現在）
let (server_conn, client_endpoint) = InProcessConnection::pair();

// Phase 5（将来）
let server_conn = GrpcConnection::connect("localhost:50051").await?;
// Godot 側のコードは変更しない
```

## InProcessConnection の設計

```
サーバー側                      クライアント側
─────────────────────           ──────────────────────
InProcessConnection             InProcessClientEndpoint
  event_tx ──────────────────→   event_rx
  command_rx ←───────────────    command_tx
```

- `event_tx` / `event_rx`: `UnboundedSender<DomainEvent>` ペア
- `command_tx` / `command_rx`: `UnboundedSender<MoveCommand>` ペア
- `pair()` で両端を同時に生成する

UnboundedChannel を選択した理由:
- Phase 4 は In-Process であるため、ネットワークバックプレッシャーは不要
- Phase 5 で GrpcConnection に差し替えるときにバックプレッシャーを追加する

## 結果

- `dawn-actor` に `client_connection` モジュールが追加された（6 テスト）
- `InProcessConnection::pair()` で即座にサーバー・クライアントの接続が確立できる
- Phase 5 では `ClientConnection` の impl を差し替えるだけで完結する
- Godot GDScript 側のコードは Phase 5 で変更しない

## 参照

- ADR-0003: Local-First Development Strategy
- ADR-0004: Client Technology Selection (Godot 4)
- CLAUDE.md FBD-002: dawn-core への外部依存禁止
- CLAUDE.md FBD-004: Actor 間の直接メソッド呼び出し禁止
- docs/architecture.md §5-A: ClientConnection の詳細
