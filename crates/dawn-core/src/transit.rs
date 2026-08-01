//! Cross-Sector Transit domain state.
//!
//! `TransitHandoffState` is the single Ship-state representation carried by
//! Raft Commit payloads and persisted in `SectorTransitCompleted` for replay.
//! Source-local coordinates, anchors, and tackle state are deliberately absent.

use crate::fitting::FittingSnapshot;
use crate::item::ItemId;
use crate::{ShipId, ShipTypeId, Velocity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Runtime state that follows one Ship across a Sector ownership boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitHandoffState {
    pub ship_id: ShipId,
    pub ship_type_id: ShipTypeId,
    pub velocity: Velocity,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    pub capacitor: Option<f32>,
    pub fitting: FittingSnapshot,
    pub inventory: BTreeMap<ItemId, u64>,
}
