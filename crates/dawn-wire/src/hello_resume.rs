use dawn_core::{PlayerId, ShipId};
use serde::{Deserialize, Serialize};

/// Identity a reconnecting client asks the server to resume (ADR-0007 §2-A):
/// present only after a `Redirect` (cross-node Sector Transit), absent for a
/// fresh connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeIdentity {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
}

/// The client's Hello message (ADR-0007 §2), carried by
/// `ClientMessage::Hello` in the binary envelope (ADR-0042).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloMessage {
    pub resume: Option<ResumeIdentity>,
}
