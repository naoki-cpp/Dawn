//! Cross-Sector Transit domain state.
//!
//! `TransitHandoffState` is the single Ship-state representation carried by
//! Raft Commit payloads and persisted in `SectorTransitCompleted` for replay.
//! Source-local coordinates, anchors, and tackle state are deliberately absent.

use crate::fitting::FittingSnapshot;
use crate::item::ItemId;
use crate::{PlayerId, ResumeTicket, ShipId, ShipTypeId, Velocity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Runtime state that follows one Ship across a Sector ownership boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedTransitHandoffState")]
pub struct TransitHandoffState {
    pub ship_id: ShipId,
    /// Durable owner binding carried across Sector boundaries. `None` is an
    /// NPC or otherwise unowned Ship and must not create client ownership.
    pub owner_player_id: Option<PlayerId>,
    /// Reconnect capability carried with an owned player Ship across Transit.
    /// NPC handoffs leave this absent.
    pub resume_ticket: Option<ResumeTicket>,
    /// A ticket staged by a resume handshake that has not committed yet.
    /// Keeping it in the handoff preserves a client-visible retry across a
    /// concurrent Transit.
    pub pending_resume_ticket: Option<ResumeTicket>,
    /// Expiry of the committed reconnect capability, as a Unix timestamp in
    /// seconds. This is present exactly when `resume_ticket` is present.
    pub resume_ticket_expires_at: Option<u64>,
    /// Expiry of the staged reconnect capability, present exactly when
    /// `pending_resume_ticket` is present.
    pub pending_resume_ticket_expires_at: Option<u64>,
    pub ship_type_id: ShipTypeId,
    pub velocity: Velocity,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    /// Redundant integrity marker. Must equal `current_hull <= 0.0`.
    pub is_destroyed: bool,
    pub capacitor: Option<f32>,
    pub fitting: FittingSnapshot,
    pub inventory: BTreeMap<ItemId, u64>,
}

#[derive(Deserialize)]
struct UncheckedTransitHandoffState {
    ship_id: ShipId,
    owner_player_id: Option<PlayerId>,
    #[serde(default)]
    resume_ticket: Option<ResumeTicket>,
    #[serde(default)]
    pending_resume_ticket: Option<ResumeTicket>,
    #[serde(default)]
    resume_ticket_expires_at: Option<u64>,
    #[serde(default)]
    pending_resume_ticket_expires_at: Option<u64>,
    ship_type_id: ShipTypeId,
    velocity: Velocity,
    current_shield: f32,
    current_armor: f32,
    current_hull: f32,
    is_destroyed: bool,
    capacitor: Option<f32>,
    fitting: FittingSnapshot,
    inventory: BTreeMap<ItemId, u64>,
}

impl TryFrom<UncheckedTransitHandoffState> for TransitHandoffState {
    type Error = &'static str;

    fn try_from(value: UncheckedTransitHandoffState) -> Result<Self, Self::Error> {
        if value.is_destroyed != (value.current_hull <= 0.0) {
            return Err("TransitHandoffState.is_destroyed must equal (current_hull <= 0.0)");
        }
        if value.resume_ticket.is_some() != value.resume_ticket_expires_at.is_some() {
            return Err(
                "TransitHandoffState.resume_ticket and resume_ticket_expires_at must be paired",
            );
        }
        if value.pending_resume_ticket.is_some() != value.pending_resume_ticket_expires_at.is_some()
        {
            return Err("TransitHandoffState.pending_resume_ticket and its expiry must be paired");
        }
        if value.pending_resume_ticket.is_some() && value.resume_ticket.is_none() {
            return Err("TransitHandoffState.pending_resume_ticket requires a current ticket");
        }

        Ok(Self {
            ship_id: value.ship_id,
            owner_player_id: value.owner_player_id,
            resume_ticket: value.resume_ticket,
            pending_resume_ticket: value.pending_resume_ticket,
            resume_ticket_expires_at: value.resume_ticket_expires_at,
            pending_resume_ticket_expires_at: value.pending_resume_ticket_expires_at,
            ship_type_id: value.ship_type_id,
            velocity: value.velocity,
            current_shield: value.current_shield,
            current_armor: value.current_armor,
            current_hull: value.current_hull,
            is_destroyed: value.is_destroyed,
            capacitor: value.capacitor,
            fitting: value.fitting,
            inventory: value.inventory,
        })
    }
}
