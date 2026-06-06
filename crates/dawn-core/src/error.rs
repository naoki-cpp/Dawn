//! Shared error types for the dawn platform.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DawnError {
    #[error("ship {0} does not exist")]
    ShipNotFound(crate::ShipId),

    #[error("ship {0} is currently in transit and cannot accept commands")]
    ShipInTransit(crate::ShipId),

    #[error("target position is outside sector boundary")]
    OutsideSectorBoundary,

    #[error("entity id counter overflow on node {0}")]
    IdCounterOverflow(crate::NodeId),

    #[error("player {0:?} does not own ship {1}")]
    NotOwner(crate::PlayerId, crate::ShipId),
}
