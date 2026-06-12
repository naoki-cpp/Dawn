//! Sector Transit proposals carried through the Raft Log (ADR-0014 §3).
//!
//! `TransitOp` is the *Command* (proposal) type serialized into Raft
//! `LogEntry` payloads — never an Event (INV-006). Once an op commits,
//! every node applies it deterministically:
//!
//! - `Request`: the owning (from) node marks the Ship `InTransit`, appends
//!   `SectorTransitRequested`, exports the Ship's state, and proposes a
//!   follow-up `Commit` op carrying that state (ADR-0014 §3 [4]).
//! - `Commit`: the destination (to) node imports the Ship at `entry_pos`
//!   and appends `SectorTransitCompleted`. Other nodes ignore it.

use crate::snapshot::ShipSnapshot;
use dawn_core::{Position, SectorId, ShipId};
use serde::{Deserialize, Serialize};

/// A Sector Transit proposal as it travels through the Raft Log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransitOp {
    /// Stage 1: a node requests moving `ship_id` to Sector `to`.
    Request {
        ship_id: ShipId,
        to     : SectorId,
    },
    /// Stage 2: the from-node ships the exported state to the to-node.
    Commit {
        ship     : ShipSnapshot,
        from     : SectorId,
        to       : SectorId,
        entry_pos: Position,
    },
}

impl TransitOp {
    /// Serialize for a Raft `LogEntry` payload.
    pub fn encode(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("TransitOp serialization cannot fail")
    }

    /// Deserialize from a committed Raft `LogEntry` payload.
    ///
    /// Returns `None` for payloads that are not a `TransitOp` (future
    /// proposal types share the same log).
    pub fn decode(payload: &[u8]) -> Option<Self> {
        postcard::from_bytes(payload).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::fitting::FittingSnapshot;
    use dawn_core::{NodeId, ShipTypeId, Velocity};

    #[test]
    fn request_op_round_trips_through_encode_and_decode() {
        let op = TransitOp::Request {
            ship_id: ShipId::new(NodeId(0), 42),
            to     : SectorId(1),
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Request { ship_id, to } => {
                assert_eq!(ship_id, ShipId::new(NodeId(0), 42));
                assert_eq!(to, SectorId(1));
            }
            other => panic!("expected Request, got {other:?}"),
        }
    }

    #[test]
    fn commit_op_round_trips_with_full_ship_snapshot() {
        let op = TransitOp::Commit {
            ship: ShipSnapshot {
                ship_id       : ShipId::new(NodeId(0), 7),
                ship_type_id  : ShipTypeId(1),
                position      : Position::new(1.0, 2.0, 3.0),
                velocity      : Velocity::new(4.0, 5.0, 6.0),
                current_shield: 10.0,
                current_armor : 20.0,
                current_hull  : 30.0,
                is_destroyed  : false,
                capacitor     : Some(50.0),
                fitting       : FittingSnapshot::empty(),
            },
            from     : SectorId(0),
            to       : SectorId(1),
            entry_pos: Position::new(500.0, 0.0, 0.0),
        };
        let decoded = TransitOp::decode(&op.encode()).expect("decode must succeed");
        match decoded {
            TransitOp::Commit { ship, from, to, entry_pos } => {
                assert_eq!(ship.ship_id, ShipId::new(NodeId(0), 7));
                assert_eq!(ship.capacitor, Some(50.0));
                assert_eq!(from, SectorId(0));
                assert_eq!(to, SectorId(1));
                assert_eq!(entry_pos, Position::new(500.0, 0.0, 0.0));
            }
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn decode_returns_none_for_garbage_payload() {
        assert!(TransitOp::decode(&[0xFF, 0xFE, 0xFD]).is_none());
    }
}
