//! Godot-independent policy for Station Inventory row interactions.
//!
//! The module converts typed row metadata and dock context into existing
//! `ClientRequest` values or a small set of Godot-local effects. It owns no
//! rendering, hit testing, drag geometry, networking, or persistence.

use dawn_core::{
    ClientRequest, ItemId, ModuleId, ShipId, ShipTypeId, SlotKind, StationId, TransferDirection,
};

/// Columns in the station inventory surface. The values are deliberately
/// independent of Godot control names and string labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationInventoryColumn {
    Fitted,
    ShipCargo,
    Station,
    Ships,
}

impl StationInventoryColumn {
    /// Convert the small numeric value used at the GDExtension boundary.
    #[must_use]
    pub const fn from_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Fitted),
            1 => Some(Self::ShipCargo),
            2 => Some(Self::Station),
            3 => Some(Self::Ships),
            _ => None,
        }
    }
}

/// The typed identity of one fitted row, used by Unfit All and reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FittedModuleRow {
    pub module: ModuleId,
    pub slot: SlotKind,
    pub index: u32,
}

/// Typed metadata attached to a rendered inventory row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationInventoryRow {
    None,
    Fitted(FittedModuleRow),
    UnfitAll,
    Cargo {
        item: ItemId,
        slot: Option<SlotKind>,
    },
    Station(ItemId),
    Disassemble,
    BuildToggle,
    BuildShipType(ShipTypeId),
    OwnedShip {
        ship: ShipId,
        active: bool,
    },
}

impl StationInventoryRow {
    #[must_use]
    pub const fn column(self) -> Option<StationInventoryColumn> {
        match self {
            Self::Fitted(_) | Self::UnfitAll => Some(StationInventoryColumn::Fitted),
            Self::Cargo { .. } => Some(StationInventoryColumn::ShipCargo),
            Self::Station(_) | Self::Disassemble | Self::BuildToggle | Self::BuildShipType(_) => {
                Some(StationInventoryColumn::Station)
            }
            Self::OwnedShip { .. } => Some(StationInventoryColumn::Ships),
            Self::None => None,
        }
    }
}

/// Runtime context required to validate a station interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StationInventoryContext<'a> {
    pub active_ship_id: Option<ShipId>,
    pub docked_station_id: Option<StationId>,
    pub fitted_modules: &'a [FittedModuleRow],
}

impl<'a> StationInventoryContext<'a> {
    #[must_use]
    pub const fn new(
        active_ship_id: Option<ShipId>,
        docked_station_id: Option<StationId>,
        fitted_modules: &'a [FittedModuleRow],
    ) -> Self {
        Self {
            active_ship_id,
            docked_station_id,
            fitted_modules,
        }
    }

    #[must_use]
    const fn active_docked(self) -> Option<(ShipId, StationId)> {
        match (self.active_ship_id, self.docked_station_id) {
            (Some(ship), Some(station)) => Some((ship, station)),
            _ => None,
        }
    }
}

/// Godot-local effects produced by the station inventory policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationInventoryLocalAction {
    ToggleBuildPicker,
}

/// A typed station inventory result. `Requests` intentionally preserves the
/// existing non-atomic Unfit All behavior: each request is sent independently
/// in order, so a later rejection cannot roll back an earlier successful one.
#[derive(Debug, Clone, PartialEq)]
pub enum StationInventoryAction {
    None,
    Request(ClientRequest),
    Requests(Vec<ClientRequest>),
    Local(StationInventoryLocalAction),
}

impl StationInventoryAction {
    #[must_use]
    pub fn request_count(&self) -> usize {
        match self {
            Self::Request(_) => 1,
            Self::Requests(requests) => requests.len(),
            Self::None | Self::Local(_) => 0,
        }
    }

    #[must_use]
    pub fn request_at(&self, index: usize) -> Option<ClientRequest> {
        match self {
            Self::Request(request) if index == 0 => Some(request.clone()),
            Self::Requests(requests) => requests.get(index).cloned(),
            _ => None,
        }
    }
}

/// Engine-independent policy for the Station Inventory HUD.
#[derive(Debug, Default, Clone, Copy)]
pub struct StationInventoryInteraction;

impl StationInventoryInteraction {
    /// Resolve a row click into a typed request or a local picker effect.
    #[must_use]
    pub fn click(
        &self,
        row: StationInventoryRow,
        context: StationInventoryContext<'_>,
    ) -> StationInventoryAction {
        match row {
            StationInventoryRow::Fitted(fitted) => context
                .active_docked()
                .map(|(ship, _)| {
                    StationInventoryAction::Request(ClientRequest::UnfitModule {
                        ship,
                        module: fitted.module,
                        slot: fitted.slot,
                    })
                })
                .unwrap_or(StationInventoryAction::None),
            StationInventoryRow::UnfitAll => context
                .active_docked()
                .map(|(ship, _)| {
                    let requests = context
                        .fitted_modules
                        .iter()
                        .map(|fitted| ClientRequest::UnfitModule {
                            ship,
                            module: fitted.module,
                            slot: fitted.slot,
                        })
                        .collect();
                    StationInventoryAction::Requests(requests)
                })
                .unwrap_or(StationInventoryAction::None),
            StationInventoryRow::Cargo {
                item: ItemId::Module(module),
                slot: Some(slot),
            } => context
                .active_docked()
                .map(|(ship, _)| {
                    StationInventoryAction::Request(ClientRequest::FitModule { ship, module, slot })
                })
                .unwrap_or(StationInventoryAction::None),
            StationInventoryRow::Cargo { .. }
            | StationInventoryRow::None
            | StationInventoryRow::OwnedShip { active: true, .. } => StationInventoryAction::None,
            StationInventoryRow::Station(ItemId::PackagedShip(ship_type)) => context
                .docked_station_id
                .map(|station| {
                    StationInventoryAction::Request(ClientRequest::Assemble { station, ship_type })
                })
                .unwrap_or(StationInventoryAction::None),
            StationInventoryRow::Station(ItemId::Module(_) | ItemId::ScrapMetal) => {
                StationInventoryAction::None
            }
            StationInventoryRow::BuildToggle => {
                StationInventoryAction::Local(StationInventoryLocalAction::ToggleBuildPicker)
            }
            StationInventoryRow::BuildShipType(ship_type) => context
                .active_docked()
                .map(|(ship, station)| {
                    StationInventoryAction::Request(ClientRequest::BuildPackagedShip {
                        ship,
                        station,
                        ship_type,
                    })
                })
                .unwrap_or(StationInventoryAction::None),
            StationInventoryRow::Disassemble => context
                .active_docked()
                .map(|(ship, station)| {
                    StationInventoryAction::Request(ClientRequest::DisassembleShip {
                        ship,
                        station,
                    })
                })
                .unwrap_or(StationInventoryAction::None),
            StationInventoryRow::OwnedShip {
                ship,
                active: false,
            } => StationInventoryAction::Request(ClientRequest::SelectActiveShip { ship }),
        }
    }

    /// Resolve a drag/drop gesture. The UI supplies only the hit-tested target
    /// column and row; all valid transitions are decided here.
    #[must_use]
    pub fn drop(
        &self,
        row: StationInventoryRow,
        target_column: StationInventoryColumn,
        target_row: StationInventoryRow,
        context: StationInventoryContext<'_>,
    ) -> StationInventoryAction {
        let Some(source_column) = row.column() else {
            return StationInventoryAction::None;
        };

        if source_column == target_column {
            return self.reorder_if_valid(row, target_row, context);
        }

        let Some((ship, station)) = context.active_docked() else {
            return StationInventoryAction::None;
        };

        match (row, target_column) {
            (
                StationInventoryRow::Cargo {
                    item: ItemId::Module(module),
                    slot: Some(slot),
                },
                StationInventoryColumn::Fitted,
            ) => StationInventoryAction::Request(ClientRequest::FitModule { ship, module, slot }),
            (StationInventoryRow::Cargo { .. }, StationInventoryColumn::Fitted) => {
                StationInventoryAction::None
            }
            (StationInventoryRow::Cargo { item, .. }, StationInventoryColumn::Station) => {
                StationInventoryAction::Request(ClientRequest::TransferCargo {
                    ship,
                    station,
                    item,
                    direction: TransferDirection::ToStation,
                })
            }
            (StationInventoryRow::Fitted(fitted), StationInventoryColumn::ShipCargo) => {
                StationInventoryAction::Request(ClientRequest::UnfitModule {
                    ship,
                    module: fitted.module,
                    slot: fitted.slot,
                })
            }
            (StationInventoryRow::Station(item), StationInventoryColumn::ShipCargo) => {
                StationInventoryAction::Request(ClientRequest::TransferCargo {
                    ship,
                    station,
                    item,
                    direction: TransferDirection::ToShip,
                })
            }
            _ => StationInventoryAction::None,
        }
    }

    fn reorder_if_valid(
        &self,
        row: StationInventoryRow,
        target_row: StationInventoryRow,
        context: StationInventoryContext<'_>,
    ) -> StationInventoryAction {
        let (
            StationInventoryRow::Fitted(source),
            StationInventoryRow::Fitted(target),
            Some((ship, _)),
        ) = (row, target_row, context.active_docked())
        else {
            return StationInventoryAction::None;
        };
        if source.slot != target.slot || source.module == target.module {
            return StationInventoryAction::None;
        }
        StationInventoryAction::Request(ClientRequest::ReorderFittedModule {
            ship,
            slot: source.slot,
            from_index: source.index,
            to_index: target.index,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{EntityId, NodeId};

    fn ship(raw: u64) -> ShipId {
        ShipId(EntityId::new(NodeId(0), raw))
    }

    fn context(
        active: Option<ShipId>,
        station: Option<StationId>,
    ) -> StationInventoryContext<'static> {
        StationInventoryContext::new(active, station, &[])
    }

    fn fitted(module: u32, slot: SlotKind, index: u32) -> StationInventoryRow {
        StationInventoryRow::Fitted(FittedModuleRow {
            module: ModuleId(module),
            slot,
            index,
        })
    }

    #[test]
    fn shipless_docked_player_can_assemble_a_packaged_ship() {
        let policy = StationInventoryInteraction;
        let action = policy.click(
            StationInventoryRow::Station(ItemId::PackagedShip(ShipTypeId(7))),
            context(None, Some(StationId(3))),
        );
        assert_eq!(
            action,
            StationInventoryAction::Request(ClientRequest::Assemble {
                station: StationId(3),
                ship_type: ShipTypeId(7),
            })
        );
    }

    #[test]
    fn shipless_player_can_select_an_owned_ship() {
        let policy = StationInventoryInteraction;
        let action = policy.click(
            StationInventoryRow::OwnedShip {
                ship: ship(8),
                active: false,
            },
            context(None, Some(StationId(3))),
        );
        assert_eq!(
            action,
            StationInventoryAction::Request(ClientRequest::SelectActiveShip { ship: ship(8) })
        );
    }

    #[test]
    fn columns_and_row_metadata_have_stable_typed_boundaries() {
        assert_eq!(
            StationInventoryColumn::from_code(0),
            Some(StationInventoryColumn::Fitted)
        );
        assert_eq!(
            StationInventoryColumn::from_code(1),
            Some(StationInventoryColumn::ShipCargo)
        );
        assert_eq!(
            StationInventoryColumn::from_code(2),
            Some(StationInventoryColumn::Station)
        );
        assert_eq!(
            StationInventoryColumn::from_code(3),
            Some(StationInventoryColumn::Ships)
        );
        assert_eq!(StationInventoryColumn::from_code(-1), None);
        assert_eq!(StationInventoryColumn::from_code(4), None);

        assert_eq!(StationInventoryRow::None.column(), None);
        assert_eq!(
            fitted(1, SlotKind::High, 0).column(),
            Some(StationInventoryColumn::Fitted)
        );
        assert_eq!(
            StationInventoryRow::UnfitAll.column(),
            Some(StationInventoryColumn::Fitted)
        );
        assert_eq!(
            StationInventoryRow::Cargo {
                item: ItemId::ScrapMetal,
                slot: None,
            }
            .column(),
            Some(StationInventoryColumn::ShipCargo)
        );
        assert_eq!(
            StationInventoryRow::Station(ItemId::ScrapMetal).column(),
            Some(StationInventoryColumn::Station)
        );
        assert_eq!(
            StationInventoryRow::OwnedShip {
                ship: ship(1),
                active: false,
            }
            .column(),
            Some(StationInventoryColumn::Ships)
        );
    }

    #[test]
    fn action_request_accessors_are_safe_for_each_result_kind() {
        let request = StationInventoryAction::Request(ClientRequest::Assemble {
            station: StationId(3),
            ship_type: ShipTypeId(7),
        });
        assert_eq!(request.request_count(), 1);
        assert!(request.request_at(0).is_some());
        assert!(request.request_at(1).is_none());

        let requests = StationInventoryAction::Requests(vec![ClientRequest::Stop]);
        assert_eq!(requests.request_count(), 1);
        assert!(matches!(requests.request_at(0), Some(ClientRequest::Stop)));

        for action in [
            StationInventoryAction::None,
            StationInventoryAction::Local(StationInventoryLocalAction::ToggleBuildPicker),
        ] {
            assert_eq!(action.request_count(), 0);
            assert!(action.request_at(0).is_none());
        }
    }

    #[test]
    fn build_and_disassemble_require_active_ship_and_docked_station() {
        let policy = StationInventoryInteraction;
        for row in [
            StationInventoryRow::BuildShipType(ShipTypeId(7)),
            StationInventoryRow::Disassemble,
        ] {
            assert_eq!(
                policy.click(row, context(None, Some(StationId(3)))),
                StationInventoryAction::None
            );
            assert_eq!(
                policy.click(row, context(Some(ship(1)), None)),
                StationInventoryAction::None
            );
        }
        assert!(matches!(
            policy.click(
                StationInventoryRow::BuildShipType(ShipTypeId(7)),
                context(Some(ship(1)), Some(StationId(3)))
            ),
            StationInventoryAction::Request(ClientRequest::BuildPackagedShip { .. })
        ));
        assert!(matches!(
            policy.click(
                StationInventoryRow::Disassemble,
                context(Some(ship(1)), Some(StationId(3)))
            ),
            StationInventoryAction::Request(ClientRequest::DisassembleShip { .. })
        ));
    }

    #[test]
    fn click_policy_covers_fitting_and_non_action_rows() {
        let policy = StationInventoryInteraction;
        let active_docked = context(Some(ship(1)), Some(StationId(3)));

        assert!(matches!(
            policy.click(fitted(5, SlotKind::High, 0), active_docked),
            StationInventoryAction::Request(ClientRequest::UnfitModule {
                ship: requested_ship,
                module: ModuleId(5),
                slot: SlotKind::High,
            }) if requested_ship == ship(1)
        ));
        assert!(matches!(
            policy.click(
                StationInventoryRow::Cargo {
                    item: ItemId::Module(ModuleId(5)),
                    slot: Some(SlotKind::Mid),
                },
                active_docked,
            ),
            StationInventoryAction::Request(ClientRequest::FitModule {
                ship: requested_ship,
                module: ModuleId(5),
                slot: SlotKind::Mid,
            }) if requested_ship == ship(1)
        ));
        assert_eq!(
            policy.click(
                StationInventoryRow::Cargo {
                    item: ItemId::Module(ModuleId(5)),
                    slot: None,
                },
                active_docked,
            ),
            StationInventoryAction::None
        );
        assert_eq!(
            policy.click(
                StationInventoryRow::Station(ItemId::ScrapMetal),
                active_docked
            ),
            StationInventoryAction::None
        );
        assert_eq!(
            policy.click(
                StationInventoryRow::OwnedShip {
                    ship: ship(2),
                    active: true,
                },
                active_docked,
            ),
            StationInventoryAction::None
        );
    }

    #[test]
    fn click_requires_dock_for_fit_unfit_and_unfit_all() {
        let policy = StationInventoryInteraction;
        let fitted_modules = [FittedModuleRow {
            module: ModuleId(5),
            slot: SlotKind::High,
            index: 0,
        }];
        let context_without_dock =
            StationInventoryContext::new(Some(ship(1)), None, &fitted_modules);
        assert_eq!(
            policy.click(fitted(5, SlotKind::High, 0), context_without_dock),
            StationInventoryAction::None
        );
        assert_eq!(
            policy.click(StationInventoryRow::UnfitAll, context_without_dock),
            StationInventoryAction::None
        );
    }

    #[test]
    fn build_picker_is_local_and_ship_choice_is_a_typed_request() {
        let policy = StationInventoryInteraction;
        assert_eq!(
            policy.click(StationInventoryRow::BuildToggle, context(None, None)),
            StationInventoryAction::Local(StationInventoryLocalAction::ToggleBuildPicker)
        );
        let action = policy.click(
            StationInventoryRow::BuildShipType(ShipTypeId(9)),
            context(Some(ship(1)), Some(StationId(3))),
        );
        let StationInventoryAction::Request(ClientRequest::BuildPackagedShip {
            ship: requested_ship,
            station: requested_station,
            ship_type: ShipTypeId(9),
        }) = action
        else {
            panic!("expected typed build request");
        };
        assert_eq!(requested_ship, ship(1));
        assert_eq!(requested_station, StationId(3));
    }

    #[test]
    fn unfit_all_returns_independent_requests_in_module_order() {
        let modules = [
            FittedModuleRow {
                module: ModuleId(2),
                slot: SlotKind::Low,
                index: 1,
            },
            FittedModuleRow {
                module: ModuleId(1),
                slot: SlotKind::High,
                index: 0,
            },
        ];
        let policy = StationInventoryInteraction;
        let action = policy.click(
            StationInventoryRow::UnfitAll,
            StationInventoryContext::new(Some(ship(1)), Some(StationId(3)), &modules),
        );
        assert_eq!(action.request_count(), 2);
        assert!(matches!(
            action.request_at(0),
            Some(ClientRequest::UnfitModule {
                module: ModuleId(2),
                ..
            })
        ));
        assert!(matches!(
            action.request_at(1),
            Some(ClientRequest::UnfitModule {
                module: ModuleId(1),
                ..
            })
        ));
    }

    #[test]
    fn cargo_transfer_requires_the_canonical_item_identity_and_valid_direction() {
        let policy = StationInventoryInteraction;
        let cargo = StationInventoryRow::Cargo {
            item: ItemId::ScrapMetal,
            slot: None,
        };
        let station = StationInventoryRow::Station(ItemId::ScrapMetal);
        assert!(matches!(
            policy.drop(
                cargo,
                StationInventoryColumn::Station,
                station,
                context(Some(ship(1)), Some(StationId(3)))
            ),
            StationInventoryAction::Request(ClientRequest::TransferCargo {
                direction: TransferDirection::ToStation,
                item: ItemId::ScrapMetal,
                ..
            })
        ));
        assert!(matches!(
            policy.drop(
                station,
                StationInventoryColumn::ShipCargo,
                cargo,
                context(Some(ship(1)), Some(StationId(3)))
            ),
            StationInventoryAction::Request(ClientRequest::TransferCargo {
                direction: TransferDirection::ToShip,
                item: ItemId::ScrapMetal,
                ..
            })
        ));
    }

    #[test]
    fn drop_policy_covers_fit_unfit_and_invalid_contexts() {
        let policy = StationInventoryInteraction;
        let ctx = context(Some(ship(1)), Some(StationId(3)));
        assert!(matches!(
            policy.drop(
                StationInventoryRow::Cargo {
                    item: ItemId::Module(ModuleId(5)),
                    slot: Some(SlotKind::High),
                },
                StationInventoryColumn::Fitted,
                StationInventoryRow::None,
                ctx,
            ),
            StationInventoryAction::Request(ClientRequest::FitModule {
                module: ModuleId(5),
                slot: SlotKind::High,
                ..
            })
        ));
        assert_eq!(
            policy.drop(
                StationInventoryRow::Cargo {
                    item: ItemId::Module(ModuleId(5)),
                    slot: None,
                },
                StationInventoryColumn::Fitted,
                StationInventoryRow::None,
                ctx,
            ),
            StationInventoryAction::None
        );
        assert!(matches!(
            policy.drop(
                fitted(5, SlotKind::High, 0),
                StationInventoryColumn::ShipCargo,
                StationInventoryRow::None,
                ctx,
            ),
            StationInventoryAction::Request(ClientRequest::UnfitModule { .. })
        ));
        assert_eq!(
            policy.drop(
                StationInventoryRow::Station(ItemId::ScrapMetal),
                StationInventoryColumn::ShipCargo,
                StationInventoryRow::None,
                context(Some(ship(1)), None),
            ),
            StationInventoryAction::None
        );
    }

    #[test]
    fn same_column_and_invalid_drop_targets_are_no_ops() {
        let policy = StationInventoryInteraction;
        let cargo = StationInventoryRow::Cargo {
            item: ItemId::ScrapMetal,
            slot: None,
        };
        let ctx = context(Some(ship(1)), Some(StationId(3)));
        assert_eq!(
            policy.drop(cargo, StationInventoryColumn::ShipCargo, cargo, ctx),
            StationInventoryAction::None
        );
        assert_eq!(
            policy.drop(
                cargo,
                StationInventoryColumn::Ships,
                StationInventoryRow::None,
                ctx
            ),
            StationInventoryAction::None
        );
        assert_eq!(
            policy.drop(
                cargo,
                StationInventoryColumn::Fitted,
                StationInventoryRow::None,
                ctx
            ),
            StationInventoryAction::None
        );
    }

    #[test]
    fn fitted_reorder_requires_same_slot_kind_and_docked_active_ship() {
        let policy = StationInventoryInteraction;
        let source = fitted(1, SlotKind::Mid, 0);
        let target = fitted(2, SlotKind::Mid, 1);
        assert!(matches!(
            policy.drop(
                source,
                StationInventoryColumn::Fitted,
                target,
                context(Some(ship(1)), Some(StationId(3)))
            ),
            StationInventoryAction::Request(ClientRequest::ReorderFittedModule {
                from_index: 0,
                to_index: 1,
                ..
            })
        ));
        assert_eq!(
            policy.drop(
                source,
                StationInventoryColumn::Fitted,
                fitted(2, SlotKind::High, 1),
                context(Some(ship(1)), Some(StationId(3)))
            ),
            StationInventoryAction::None
        );
        assert_eq!(
            policy.drop(
                source,
                StationInventoryColumn::Fitted,
                target,
                context(None, Some(StationId(3)))
            ),
            StationInventoryAction::None
        );
    }
}
