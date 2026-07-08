use dawn_core::{EntityId, PlayerId, ShipId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeIdentity {
    pub player_id: PlayerId,
    pub ship_id: ShipId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelloMessage {
    pub resume: Option<ResumeIdentity>,
}

/// Parse the client Hello line. Fresh clients send only `{"type":"Hello"}`;
/// clients following a Redirect include the identity to resume.
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
