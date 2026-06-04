//! Commands — requests to change the world that may be rejected.
//!
//! A Command is *not* an Event.  It expresses intent, not fact.
//! The system validates a Command before producing an Event.  (INV-006)

use crate::{Position, ShipId};
use serde::{Deserialize, Serialize};

/// Request to move a Ship to `target_position` within its current Sector.
///
/// May be rejected if:
/// - The Ship does not exist.
/// - The Ship is currently in transit between Sectors.
/// - `target_position` is outside the Sector boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveCommand {
    pub ship_id         : ShipId,
    pub target_position : Position,
}

impl MoveCommand {
    pub fn new(ship_id: ShipId, target_position: Position) -> Self {
        Self { ship_id, target_position }
    }
}
