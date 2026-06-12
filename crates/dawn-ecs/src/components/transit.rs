//! Sector Transit state component (ADR-0014).
//!
//! A Ship that is `InTransit` must reject further Move / Despawn / Transit
//! commands until the transit completes or is aborted (CLAUDE.md §5).

use dawn_core::SectorId;

/// Sector Transit state of a Ship.
///
/// Defaults to `None`: most ships are never in transit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransitState {
    #[default]
    None,
    InTransit { to: SectorId },
}

impl TransitState {
    /// True while a transit to another Sector is pending commit/completion.
    pub fn is_in_transit(&self) -> bool {
        matches!(self, TransitState::InTransit { .. })
    }
}

/// ECS component wrapping [`TransitState`].
#[derive(Debug, Clone, Copy, Default)]
pub struct TransitComp(pub TransitState);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_transit_state_is_none_and_not_in_transit() {
        let state = TransitState::default();
        assert_eq!(state, TransitState::None);
        assert!(!state.is_in_transit());
    }

    #[test]
    fn in_transit_state_reports_destination_sector() {
        let to = SectorId(7);
        let state = TransitState::InTransit { to };
        assert!(state.is_in_transit());
        assert_eq!(state, TransitState::InTransit { to: SectorId(7) });
    }
}
