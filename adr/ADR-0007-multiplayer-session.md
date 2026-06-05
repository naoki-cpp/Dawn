---
id      : ADR-0007
title   : マルチプレイヤー対応設計（Phase 5）
status  : proposed
date    : 2026-06-05
deciders: [human, ai-agent]
related : ADR-0005（ClientConnection）, ADR-0006（Fitting/Combat）
---

# ADR-0007 — マルチプレイヤー対応設計（Phase 5）

## 背景

Phase 4 の仕様チェック（2026-06-05）で、現在の実装がシングルプレイ専用に
なっている箇所を特定した。

Phase 5（本物のネットワーク）への移行前に、マルチプレイヤー対応の
設計方針を決定しておく必要がある。

### Phase 4 で判明したシングルプレイ専用の問題

| 問題 | 箇所 | 深刻度 |
|---|---|---|
| WsServer が1クライアントのみ受け付ける | `ws_server.rs` `accept()` | 🔴 |
| Ship 所有権の概念がない | `node.rs` `apply_move_command` | 🔴 |
| ORIGIN 座標をプレイヤー指定シグナルに流用 | `main.rs` / `main.gd` | 🔴 |
| セッション管理（PlayerId）がない | アーキテクチャ全体 | 🔴 |
| AttackCommand の JSON パーサー未実装 | `ws_server.rs` | 🟡 |
| コマンド認証なし | `node.rs` | 🟡 |

---

## 決定

### 1. `PlayerId` 型の追加（`dawn-core`）

```rust
/// プレイヤーセッションを識別する ID。
/// サーバーが接続時に採番する。セッション切断後も再利用しない（INV-004 と同原則）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u64);
```

### 2. Ship 所有権の管理（`dawn-simulation`）

```rust
// SimulationNode に追加
player_ships: HashMap<PlayerId, ShipId>,  // PlayerId → 所有する ShipId
ship_owners : HashMap<ShipId, PlayerId>,  // 逆引き（ShipId → 所有者）
```

**所有権ルール:**
- `spawn_player_ship(player_id, ...)` で専用の Ship を生成し、所有権を記録する
- 所有権のない Ship（NPC）は誰も直接操作できない
- 所有権のある Ship は所有者のみが操作できる

### 3. コマンド処理での所有権検証

```rust
// 変更前（Phase 4）
fn apply_move_command(&mut self, ship_id: ShipId, target: Position)

// 変更後（Phase 5）
fn apply_move_command(
    &mut self,
    player_id : PlayerId,   // コマンド送信者
    ship_id   : ShipId,
    target    : Position,
) -> Result<(), CommandError> {
    // 所有権チェック
    if self.ship_owners.get(&ship_id) != Some(&player_id) {
        return Err(CommandError::NotOwner);
    }
    // ...以降は現状と同じ
}
```

同様に `fit_module`、`attack` も `PlayerId` を受け取る。

### 4. ORIGIN シグナルの廃止と置き換え

**廃止:** `MoveCommand { target_position: ORIGIN }` によるプレイヤー指定  
**置き換え:** 接続ハンドシェイク時のセッション確立フローに統合する

```
接続フロー（Phase 5）:
  1. クライアントが WebSocket / gRPC に接続する
  2. サーバーが PlayerId を採番して返す
  3. サーバーが専用の Ship を Spawn し、PlayerId と紐付ける
  4. クライアントは採番された PlayerId を以後の全コマンドに含める
  5. サーバーは PlayerId でコマンドを検証する
```

### 5. WsServer の多クライアント対応

```rust
// 変更前（Phase 4）
let conn = server.accept().await;  // 1回のみ

// 変更後（Phase 5）
// accept ループで複数クライアントを並行管理
tokio::spawn(async move {
    loop {
        let (conn, peer_addr) = server.accept().await?;
        let player_id = session_manager.new_player();
        spawn_player_handler(player_id, conn, node_tx.clone());
    }
});
```

`ClientConnection` trait のインターフェース自体は変更しない（ADR-0005 の方針を維持）。
多クライアント管理は `ClientConnection` の上位層（main ループまたは Actor）が担う。

### 6. イベント配信はブロードキャストのまま維持

```
全クライアントに全 DomainEvent を配信する（EVE Online と同じモデル）。

根拠:
  - Event Sourcing では「全員が同じイベントログを持つ」ことで世界が収束する
  - Interest Management（近くのイベントのみ送る最適化）は Phase 8 で対応
  - Phase 5 ではまず正しく動くことを優先する
```

### 7. `AttackCommand` の JSON パーサー追加（Phase 5 前に対応）

```rust
// ws_server.rs に追加
fn parse_attack_command(line: &str) -> Option<AttackCommand> { ... }

// JSON フォーマット
// {"type":"AttackCommand","attacker_id":1,"target_id":2}
```

---

## 実装チェックリスト（Phase 5 着手時に使用）

### サーバー側

- [ ] `dawn-core`: `PlayerId` 型追加、`CommandError::NotOwner` 追加
- [ ] `dawn-simulation/node.rs`: `player_ships`, `ship_owners` フィールド追加
- [ ] `dawn-simulation/node.rs`: `spawn_player_ship(player_id, ...)` メソッド追加
- [ ] `dawn-simulation/node.rs`: 全コマンド処理に `PlayerId` と所有権チェックを追加
- [ ] `dawn-simulation/ws_server.rs`: `parse_attack_command` 追加
- [ ] `dawn-simulation/ws_server.rs`: 多クライアント対応（accept ループ）
- [ ] `dawn-simulation/main.rs`: ORIGIN シグナルを接続ハンドシェイクに置き換え

### クライアント側（Godot）

- [ ] `connection.gd`: 接続時のハンドシェイク（PlayerId 受け取り）を実装
- [ ] `connection.gd`: 全コマンド送信に PlayerId を含める
- [ ] `main.gd`: ORIGIN シグナル送信を削除し、ハンドシェイク完了後に自船を設定
- [ ] `main.gd`: 右クリック → AttackCommand 送信

---

## 変更しない設計（Phase 5 でも維持）

```
ClientConnection trait のインターフェース（ADR-0005）
  → send_events / try_recv_command の 2 方向構成はそのまま

Event Sourcing の原則（INV-001〜006）
  → 全クライアントに全 DomainEvent をブロードキャスト

ShipId のグローバル一意性（INV-004）
  → PlayerId が増えても ShipId の採番ルールは変わらない

Tick の論理カウンタ（INV-005）
  → クライアント数に関係なく Tick は単調増加
```

---

## 結果として生じる制約

- Phase 5 では `PlayerId` を `MoveCommand` / `FitModuleCommand` / `AttackCommand` に追加するため、
  既存コマンドのフィールドが変わる。
  - `MoveCommand` は破壊的変更になるため **新型 `MoveCommandV2`** を定義するか、
    または Phase 5 が greenfield な実装（ws_server.rs を GrpcConnection に完全置換）として
    扱い、Phase 4 のプロトコルとの後方互換を捨てる。
  - **Phase 4 の WebSocket プロトコルとの後方互換は不要**（Phase 5 でプロトコルごと置換するため）。

---

## 参照

- ADR-0005: ClientConnection 抽象化
- ADR-0006: Fitting / Combat 設計
- CLAUDE.md §12 パターン7: ORIGIN 座標シグナル流用（暫定措置の記録）
- EVE Online アーキテクチャ参考: Single Shard + ブロードキャスト配信
