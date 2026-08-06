//! Loadout-refresh client command dispatch.
//!
//! Fit, Unfit, and ReorderFittedModule all target explicitly named owned ships
//! and all require the same client correction: resend the authoritative
//! `PlayerLoadout` whether the optimistic mutation was accepted or rejected.
//! This module owns that family policy; the outer boundary only projects the
//! effect into `ClientCommandFollowup`.

use dawn_core::{FitModuleCommand, PlayerId, ReorderFittedModuleCommand, UnfitModuleCommand};
use dawn_event_store::store::EventStore;

use super::SimulationNode;

pub(super) enum LoadoutDispatchCommand {
    Fit(FitModuleCommand),
    Unfit(UnfitModuleCommand),
    ReorderFittedModule(ReorderFittedModuleCommand),
}

pub(super) enum LoadoutDispatchEffect {
    RefreshPlayerLoadout,
}

impl<S: EventStore> SimulationNode<S> {
    pub(super) fn dispatch_loadout_command(
        &mut self,
        player_id: PlayerId,
        cmd: LoadoutDispatchCommand,
    ) -> LoadoutDispatchEffect {
        match cmd {
            LoadoutDispatchCommand::Fit(cmd) => {
                self.fit_module_owned(player_id, cmd);
            }
            LoadoutDispatchCommand::Unfit(cmd) => {
                self.unfit_module_owned(player_id, cmd);
            }
            LoadoutDispatchCommand::ReorderFittedModule(cmd) => {
                self.reorder_fitted_module_owned(player_id, cmd);
            }
        }
        LoadoutDispatchEffect::RefreshPlayerLoadout
    }
}

#[cfg(test)]
mod tests {
    use dawn_core::{
        EntityId, FitModuleCommand, ModuleId, NodeId, PlayerId, SectorBounds, SectorId, ShipId,
        SlotKind,
    };

    use super::*;

    fn node() -> SimulationNode {
        SimulationNode::new_test(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(crate::galaxy::Galaxy::demo()),
        )
    }

    #[test]
    fn rejected_loadout_mutation_still_requests_authoritative_refresh() {
        let mut node = node();
        let effect = node.dispatch_loadout_command(
            PlayerId(7),
            LoadoutDispatchCommand::Fit(FitModuleCommand {
                ship_id: ShipId(EntityId::from_raw(999)),
                slot: SlotKind::High,
                module_id: ModuleId(1),
            }),
        );

        assert!(matches!(
            effect,
            LoadoutDispatchEffect::RefreshPlayerLoadout
        ));
    }
}
