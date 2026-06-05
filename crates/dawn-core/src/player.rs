//! PlayerId — プレイヤーセッションの識別子。
//!
//! # 設計 (ADR-0007 §3)
//!
//! - PlayerId は接続時にサーバーが採番する。
//! - セッション切断後も同じ ID は再利用しない（INV-004 と同原則）。
//! - ClientCommand には含まれない。WsServer が接続 ↔ PlayerId の対応を管理する。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u64);

impl PlayerId {
    pub fn raw(self) -> u64 { self.0 }
}

impl std::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Player({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_ids_with_different_values_are_not_equal() {
        assert_ne!(PlayerId(1), PlayerId(2));
    }

    #[test]
    fn player_id_raw_returns_inner_value() {
        assert_eq!(PlayerId(42).raw(), 42);
    }
}
