use dawn_core::{EntityId, PlayerId, ShipId};
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

/// Parse the client Hello line. Fresh clients send only `{"type":"Hello"}`;
/// clients following a Redirect include the identity to resume.
///
/// This JSON-text parser stays in place for the legacy handshake path
/// (ADR-0042 stage 1 keeps only Welcome/Redirect/Event/Hello/Command on the
/// binary envelope for the *already*-typed messages; a plain-text Hello
/// during the WebSocket upgrade is unaffected either way). New code that
/// constructs a `ClientMessage::Hello` directly should build a
/// [`HelloMessage`] instead of hand-writing JSON.
pub fn parse_hello(line: &str) -> Option<HelloMessage> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "Hello" {
        return None;
    }

    let resume = match (
        v.get("player_id").and_then(|id| id.as_u64()),
        v.get("ship_id").and_then(|id| id.as_u64()),
    ) {
        (Some(player_id), Some(ship_id)) => Some(ResumeIdentity {
            player_id: PlayerId(player_id),
            ship_id: ShipId(EntityId::from_raw(ship_id)),
        }),
        _ => None,
    };

    Some(HelloMessage { resume })
}
