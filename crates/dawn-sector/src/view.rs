//! Read-only Sector state required by derived consumers.
//!
//! `SectorView` is intentionally narrower than the authoritative engine. It
//! lets presentation and spatial-delivery code read committed state without
//! inheriting the engine's legacy persistence generic. The trait contains no
//! mutation or storage operation; recovery and command ownership stay outside
//! this boundary.

use dawn_core::{AbsolutePosition, ShipId};
use dawn_protocol::ShipStateWire;

/// Read-only committed Sector facts needed by AoI delivery.
pub trait SectorView {
    /// Return every known ship and its absolute Sector-frame position.
    fn ship_absolute_positions(&self) -> Vec<(ShipId, AbsolutePosition)>;

    /// Return one ship's absolute Sector-frame position.
    fn ship_absolute_pos(&self, ship_id: ShipId) -> Option<AbsolutePosition>;

    /// Return the presentation snapshot for one known ship.
    fn ship_state(&self, ship_id: ShipId) -> Option<ShipStateWire>;

    /// Whether the ship is in the committed warp phase.
    fn ship_is_warping(&self, ship_id: ShipId) -> bool;
}
