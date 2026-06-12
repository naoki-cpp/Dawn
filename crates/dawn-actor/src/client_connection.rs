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
//!              │  try_recv_command() → ClientCommand
//!              └──────── ←  command_tx.send()
//! ```
//!
//! Phase 4: `InProcessConnection` (tokio::mpsc チャンネル直結)
//! Phase 5: `GrpcConnection`     (tonic による本物のネットワーク)
//!
//! ## ClientCommand の拡張方針
//!
//! クライアントから送信できるコマンドは `ClientCommand` enum に追加する。
//! `ClientConnection` trait 自体は変更しない。
//! 新コマンドの追加 = `ClientCommand` に variant を追加するだけ。

use dawn_core::{ActivateModuleCommand, AttackCommand, DeactivateModuleCommand, DomainEvent, JumpCommand, LockOnCommand, MoveCommand, StopCommand};
use tokio::sync::mpsc;

// ── Error ─────────────────────────────────────────────────────────────────────

/// `ClientConnection` の操作で発生しうるエラー。
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// 接続先（クライアント）が既に切断されている。
    #[error("client disconnected")]
    Disconnected,
}

// ── ClientCommand ─────────────────────────────────────────────────────────────

/// クライアントからサーバーへ送信できる全コマンドの列挙。
///
/// 新コマンドを追加する場合はここに variant を追加し、
/// `ws_server.rs` の JSON パーサーと `main.rs` の振り分けを更新する。
///
/// `ClientConnection::try_recv_command()` が返す型。
#[derive(Debug, Clone)]
pub enum ClientCommand {
    /// 推力方向の指定（Cycle 2〜）
    Move(MoveCommand),
    /// ロックオン開始（Cycle 3〜）
    LockOn(LockOnCommand),
    /// Active モジュールをオンにする
    Activate(ActivateModuleCommand),
    /// Active モジュールをオフにする
    Deactivate(DeactivateModuleCommand),
    /// ターゲットへの攻撃（将来の手動攻撃モード用、現在は自動戦闘が優先）
    Attack(AttackCommand),
    /// 減速停止（thrust を逆方向に掛けて速度ゼロまで減速する）
    Stop(StopCommand),
    /// ジャンプゲート経由の Sector 移動（ADR-0009）
    Jump(JumpCommand),
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
    fn send_events(&self, events: &[DomainEvent]) -> Result<(), ConnectionError>;

    /// クライアントから届いたコマンドを 1 件ノンブロッキングで取り出す。
    fn try_recv_command(&mut self) -> Option<ClientCommand>;
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
    event_tx   : mpsc::UnboundedSender<DomainEvent>,
    command_rx : mpsc::UnboundedReceiver<ClientCommand>,
}

/// クライアント側のエンドポイント。
pub struct InProcessClientEndpoint {
    pub event_rx   : mpsc::UnboundedReceiver<DomainEvent>,
    pub command_tx : mpsc::UnboundedSender<ClientCommand>,
}

impl InProcessConnection {
    pub fn pair() -> (Self, InProcessClientEndpoint) {
        let (event_tx,   event_rx)   = mpsc::unbounded_channel();
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        (
            InProcessConnection { event_tx, command_rx },
            InProcessClientEndpoint { event_rx, command_tx },
        )
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

    fn try_recv_command(&mut self) -> Option<ClientCommand> {
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
            ship_id          : ShipId(EntityId::new(NodeId(0), 1)),
            initial_position : Position { x: 1.0, y: 2.0, z: 3.0 },
            sector_id        : SectorId(0),
            ship_type_id     : dawn_core::ShipTypeId(1),
            tick             : Tick::ZERO,
        })
    }

    fn make_move_command() -> ClientCommand {
        ClientCommand::Move(MoveCommand {
            ship_id         : ShipId(EntityId::new(NodeId(0), 1)),
            target_position : Position { x: 10.0, y: 0.0, z: 0.0 },
        })
    }

    fn make_lock_on_command() -> ClientCommand {
        ClientCommand::LockOn(LockOnCommand {
            ship_id   : ShipId(EntityId::new(NodeId(0), 1)),
            target_id : ShipId(EntityId::new(NodeId(0), 2)),
        })
    }

    #[test]
    fn server_events_are_received_by_client_endpoint() {
        let (server, mut client) = InProcessConnection::pair();
        let event = make_ship_spawned();
        server.send_events(&[event.clone()]).unwrap();
        let received = client.event_rx.try_recv().expect("event should be available");
        assert_eq!(format!("{:?}", received), format!("{:?}", event));
    }

    #[test]
    fn move_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(make_move_command()).unwrap();
        let cmd = server.try_recv_command().expect("command should be available");
        assert!(matches!(cmd, ClientCommand::Move(_)));
    }

    #[test]
    fn lock_on_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(make_lock_on_command()).unwrap();
        let cmd = server.try_recv_command().expect("command should be available");
        assert!(matches!(cmd, ClientCommand::LockOn(_)));
    }

    #[test]
    fn commands_are_delivered_in_order() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(make_move_command()).unwrap();
        client.command_tx.send(make_lock_on_command()).unwrap();
        assert!(matches!(server.try_recv_command().unwrap(), ClientCommand::Move(_)));
        assert!(matches!(server.try_recv_command().unwrap(), ClientCommand::LockOn(_)));
    }

    #[test]
    fn jump_command_is_received_by_server_connection() {
        let (mut server, client) = InProcessConnection::pair();
        client.command_tx.send(ClientCommand::Jump(dawn_core::JumpCommand {
            ship_id: ShipId(EntityId::new(NodeId(0), 1)),
            gate_id: dawn_core::JumpGateId(0),
        })).unwrap();
        let cmd = server.try_recv_command().expect("command should be available");
        assert!(matches!(cmd, ClientCommand::Jump(_)));
    }

    #[test]
    fn try_recv_command_returns_none_when_no_command_pending() {
        let (mut server, _client) = InProcessConnection::pair();
        assert!(server.try_recv_command().is_none());
    }

    #[test]
    fn send_events_returns_disconnected_when_client_dropped() {
        let (server, client) = InProcessConnection::pair();
        drop(client);
        let result = server.send_events(&[make_ship_spawned()]);
        assert!(matches!(result, Err(ConnectionError::Disconnected)));
    }

    #[test]
    fn send_events_with_empty_slice_is_always_ok() {
        let (server, _client) = InProcessConnection::pair();
        assert!(server.send_events(&[]).is_ok());
    }

    #[test]
    fn multiple_events_are_delivered_in_order() {
        let (server, mut client) = InProcessConnection::pair();
        let events: Vec<DomainEvent> = (0..3).map(|_| make_ship_spawned()).collect();
        server.send_events(&events).unwrap();
        let mut count = 0;
        while client.event_rx.try_recv().is_ok() { count += 1; }
        assert_eq!(count, 3);
    }
}
