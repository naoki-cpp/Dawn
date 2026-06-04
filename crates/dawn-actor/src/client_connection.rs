//! # ClientConnection — サーバー／クライアント通信の抽象境界
//!
//! ## 設計方針 (ADR-0005)
//!
//! この trait は **2 方向のみ** を定義する:
//!
//! ```text
//! サーバー側                      クライアント側
//! ─────────────────────────       ──────────────────
//! SectorSimulatorActor            Godot シーン / テスト
//!     │  send_events()                ↑  recv_event()
//!     │                              │
//!     └─── ClientConnection ─────────┘
//!              │  try_recv_command()
//!              └──────── ←  command_tx.send()
//! ```
//!
//! Phase 4: `InProcessConnection` (tokio::mpsc チャンネル直結)
//! Phase 5: `GrpcConnection`     (tonic による本物のネットワーク)
//!
//! Godot / クライアント側のコードは trait に向かって書くため、
//! Phase 5 での差し替え時に Godot 側のコードを変更しない。
//!
//! ## 責務の範囲
//!
//! この trait はドメインイベントとコマンドの **転送のみ** を行う。
//! バリデーション・永続化・レプリケーションは上位層の責務である。

use dawn_core::{DomainEvent, MoveCommand};
use tokio::sync::mpsc;

// ── Error ─────────────────────────────────────────────────────────────────────

/// `ClientConnection` の操作で発生しうるエラー。
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// 接続先（クライアント）が既に切断されている。
    #[error("client disconnected")]
    Disconnected,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

/// サーバー側から見たクライアント接続の抽象。
///
/// # 実装ルール
///
/// - `send_events` は非ブロッキングで完了すること（`Err` でバックプレッシャーを表現）。
/// - `try_recv_command` はノンブロッキングであること（コマンドがなければ `None`）。
/// - 実装は `Send + 'static` を満たすこと（Actor スレッドをまたいで移動するため）。
pub trait ClientConnection: Send + 'static {
    /// サーバーからクライアントへイベントを送信する。
    ///
    /// `events` が空の場合は何もしない（エラーにならない）。
    /// クライアントが切断済みの場合は `Err(ConnectionError::Disconnected)` を返す。
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), ConnectionError>;

    /// クライアントから届いたコマンドを 1 件ノンブロッキングで取り出す。
    ///
    /// コマンドがなければ `None` を返す。
    /// クライアントが切断済みでコマンドもなければ `None` を返す。
    fn try_recv_command(&mut self) -> Option<MoveCommand>;
}

// ── InProcessConnection ───────────────────────────────────────────────────────

/// In-Process 実装。tokio unbounded channel で直結する。
///
/// Phase 4 専用。本番ネットワークは `GrpcConnection`（Phase 5）で実装する。
///
/// ## 使い方
///
/// ```rust
/// use dawn_actor::client_connection::{InProcessConnection, InProcessClientEndpoint};
///
/// let (server_side, client_side) = InProcessConnection::pair();
/// // server_side → SectorSimulatorActor に渡す
/// // client_side → Godot / テストコードに渡す
/// ```
pub struct InProcessConnection {
    event_tx:   mpsc::UnboundedSender<DomainEvent>,
    command_rx: mpsc::UnboundedReceiver<MoveCommand>,
}

/// クライアント側のエンドポイント。
///
/// Godot GDScript または統合テストからイベントを受信し、コマンドを送信する。
pub struct InProcessClientEndpoint {
    pub event_rx:   mpsc::UnboundedReceiver<DomainEvent>,
    pub command_tx: mpsc::UnboundedSender<MoveCommand>,
}

impl InProcessConnection {
    /// サーバー側 / クライアント側のペアを生成する。
    pub fn pair() -> (Self, InProcessClientEndpoint) {
        let (event_tx,   event_rx)   = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();

        let server = InProcessConnection { event_tx, command_rx };
        let client = InProcessClientEndpoint { event_rx, command_tx };
        (server, client)
    }
}

impl ClientConnection for InProcessConnection {
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), ConnectionError> {
        for event in events {
            self.event_tx
                .send(event.clone())
                .map_err(|_| ConnectionError::Disconnected)?;
        }
        Ok(())
    }

    fn try_recv_command(&mut self) -> Option<MoveCommand> {
        self.command_rx.try_recv().ok()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{
        EntityId, NodeId, Position, ShipId, Tick,
        DomainEvent, SectorId, MoveCommand,
    };
    use dawn_core::events::ShipSpawned;

    fn make_ship_spawned() -> DomainEvent {
        DomainEvent::ShipSpawned(ShipSpawned {
            ship_id:          ShipId(EntityId::new(NodeId(0), 1)),
            initial_position: Position { x: 1.0, y: 2.0, z: 3.0 },
            sector_id:        SectorId(0),
            tick:             Tick::ZERO,
        })
    }

    fn make_move_command() -> MoveCommand {
        MoveCommand {
            ship_id:         ShipId(EntityId::new(NodeId(0), 1)),
            target_position: Position { x: 10.0, y: 0.0, z: 0.0 },
        }
    }

    // サーバーが送ったイベントをクライアントエンドポイントで受信できる
    #[test]
    fn server_events_are_received_by_client_endpoint() {
        let (server, mut client) = InProcessConnection::pair();
        let event = make_ship_spawned();

        server.send_events(&[event.clone()]).unwrap();

        let received = client.event_rx.try_recv().expect("event should be available");
        assert_eq!(format!("{:?}", received), format!("{:?}", event));
    }

    // クライアントが送ったコマンドをサーバー側で取り出せる
    #[test]
    fn client_commands_are_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        let cmd = make_move_command();

        client.command_tx.send(cmd.clone()).unwrap();

        let received = server.try_recv_command().expect("command should be available");
        assert_eq!(received.ship_id, cmd.ship_id);
        assert_eq!(received.target_position, cmd.target_position);
    }

    // コマンドがないときは None を返す
    #[test]
    fn try_recv_command_returns_none_when_no_command_pending() {
        let (mut server, _client) = InProcessConnection::pair();
        assert!(server.try_recv_command().is_none());
    }

    // クライアントが切断されたら send_events は Disconnected を返す
    #[test]
    fn send_events_returns_disconnected_when_client_dropped() {
        let (server, client) = InProcessConnection::pair();
        drop(client); // クライアントを切断

        let result = server.send_events(&[make_ship_spawned()]);
        assert!(matches!(result, Err(ConnectionError::Disconnected)));
    }

    // 空スライスの send_events は常に Ok
    #[test]
    fn send_events_with_empty_slice_is_always_ok() {
        let (server, _client) = InProcessConnection::pair();
        assert!(server.send_events(&[]).is_ok());
    }

    // 複数イベントをまとめて送受信できる
    #[test]
    fn multiple_events_are_delivered_in_order() {
        let (server, mut client) = InProcessConnection::pair();
        let events: Vec<DomainEvent> = (0..3).map(|_| make_ship_spawned()).collect();

        server.send_events(&events).unwrap();

        let mut count = 0;
        while client.event_rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
