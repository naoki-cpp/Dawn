use super::*;
use dawn_core::{DomainEvent, NodeId, Position, SectorBounds, SectorId, Tick, Velocity};

fn mem_node() -> SimulationNode {
    SimulationNode::new(
        NodeId(0),
        SectorId(0),
        SectorBounds::centered(SectorBounds::DEFAULT_HALF),
    )
}

#[test]
fn damage_taken_event_is_replayed_to_restore_current_hp() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::DamageTaken(dawn_core::events::DamageTaken {
        ship_id,
        damage: 100.0,
        current_shield: 100.0,
        current_armor: 150.0,
        current_hull: 150.0,
        tick: Tick(1),
    }));

    let hp = node.get_ship_hp(ship_id).unwrap();
    assert_eq!(hp, 400.0, "HP total after replay = 100 + 150 + 150 = 400");
}

#[test]
fn repair_applied_event_is_replayed_to_restore_current_hp() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::RepairApplied(
        dawn_core::events::RepairApplied {
            ship_id,
            amount: 50.0,
            layer: dawn_core::events::RepairLayer::Shield,
            current_shield: 150.0,
            current_armor: 150.0,
            current_hull: 150.0,
            tick: Tick(2),
        },
    ));

    let hp = node.get_ship_hp(ship_id).unwrap();
    assert_eq!(hp, 450.0, "HP total after replay = 150 + 150 + 150 = 450");
}

#[test]
fn module_deactivated_event_replay_resets_cycle_remaining() {
    use crate::modules;
    use dawn_core::{FitModuleCommand, SlotKind};

    let mut node = mem_node();
    for def in crate::game_data::test_catalog().modules().to_vec() {
        node.register_module(def);
    }
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.fit_module(FitModuleCommand {
        ship_id,
        slot: SlotKind::High,
        module_id: modules::MODULE_RAILGUN_SMALL,
    });

    // Reach into the fitting directly (bypassing activation/capacitor)
    // to simulate a module that was mid-cycle when the node stopped:
    // is_active = true and cycle_remaining > 0.
    {
        let entity = *node.ships.index.get(&ship_id).unwrap();
        let mut fitting = node.world.get_mut::<FittingComp>(entity).unwrap();
        let slot = fitting
            .find_slot_mut(modules::MODULE_RAILGUN_SMALL, SlotKind::High)
            .unwrap();
        slot.is_active = true;
        slot.cycle_remaining = 7;
    }

    node.apply_event_pub(DomainEvent::ModuleDeactivated(
        dawn_core::events::ModuleDeactivated {
            ship_id,
            module_id: modules::MODULE_RAILGUN_SMALL,
            slot: SlotKind::High,
            forced_reason: None,
            tick: Tick(3),
        },
    ));

    let entity = *node.ships.index.get(&ship_id).unwrap();
    let mut fitting = node.world.get_mut::<FittingComp>(entity).unwrap();
    let slot = fitting
        .find_slot_mut(modules::MODULE_RAILGUN_SMALL, SlotKind::High)
        .unwrap();
    assert!(!slot.is_active);
    assert_eq!(
        slot.cycle_remaining, 0,
        "replaying ModuleDeactivated must reset cycle_remaining, matching every live \
         deactivation path (capacitor exhaustion, Range Gate, player-issued)"
    );
}

/// ADR-0038: the credit/debit already happened live in
/// `build_packaged_ship_owned` (synchronously, straight to SQLite) before
/// this event was ever appended. Replaying it must NOT touch Station
/// inventory again -- pre-seed the state as it would already be
/// post-live-command, replay the event, and confirm nothing changed.
#[test]
fn packaged_ship_built_event_replay_does_not_double_apply_station_inventory() {
    let mut node = mem_node();
    let player_id = dawn_core::PlayerId(5);
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    // As if build_packaged_ship_owned already ran live: 3 -> 2 ScrapMetal,
    // +1 PackagedShip.
    node.replace_station_inventory(
        player_id,
        dawn_core::StationId(0),
        std::collections::BTreeMap::from([
            (dawn_core::ItemId::ScrapMetal, 2),
            (
                dawn_core::ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE),
                1,
            ),
        ]),
    );

    node.apply_event_pub(DomainEvent::PackagedShipBuilt(
        dawn_core::events::PackagedShipBuilt {
            ship_id,
            player_id,
            station_id: dawn_core::StationId(0),
            ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
            scrap_cost: 1,
            tick: Tick(3),
        },
    ));

    assert_eq!(
        node.station_item_count(
            player_id,
            dawn_core::StationId(0),
            dawn_core::ItemId::ScrapMetal,
        ),
        2,
        "replay must not debit ScrapMetal a second time"
    );
    assert_eq!(
        node.station_item_count(
            player_id,
            dawn_core::StationId(0),
            dawn_core::ItemId::PackagedShip(crate::ship_types::SHIP_TYPE_MAGPIE)
        ),
        1,
        "replay must not credit the PackagedShip a second time"
    );
}

#[test]
fn docking_event_replay_restores_player_docked_context() {
    let mut node = mem_node();
    let player_id = dawn_core::PlayerId(5);
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.adopt_player_ship(ship_id, player_id);

    node.apply_event_pub(DomainEvent::ShipDocked(dawn_core::events::ShipDocked {
        ship_id,
        station_id: dawn_core::StationId(0),
        tick: Tick(3),
    }));

    assert_eq!(
        node.player_docked_station(player_id),
        Some(dawn_core::StationId(0))
    );
}

/// ADR-0038: the credit already happened live in `disassemble_ship_owned`.
/// Replay must still remove the ship (that's not SQLite-durable, it's
/// ECS state reconstructed from the event tail) but must NOT credit
/// Station inventory again.
#[test]
fn ship_disassembled_event_replay_does_not_double_credit_station_inventory() {
    let mut node = mem_node();
    let player_id = dawn_core::PlayerId(5);
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.adopt_player_ship(ship_id, player_id);
    // As if disassemble_ship_owned already ran live.
    node.credit_station_item(
        player_id,
        dawn_core::StationId(0),
        dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(1)),
        1,
    );

    node.apply_event_pub(DomainEvent::ShipDisassembled(
        dawn_core::events::ShipDisassembled {
            ship_id,
            player_id,
            station_id: dawn_core::StationId(0),
            ship_type_id: dawn_core::ShipTypeId(1),
            tick: Tick(3),
        },
    ));

    assert_eq!(
        node.station_item_count(
            player_id,
            dawn_core::StationId(0),
            dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(1))
        ),
        1,
        "replay must not credit the PackagedShip a second time"
    );
    assert!(node.get_ship_position(ship_id).is_none());
}

/// ADR-0038: the debit already happened live in `assemble_ship_owned`
/// (there is nothing left to debit here -- pre-seeding a stack and
/// expecting replay to consume it, as this test used to, would prove the
/// double-application bug this ADR fixes rather than guard against it).
/// Replay must still reconstruct the ship ECS state from the event.
#[test]
fn ship_assembled_event_replay_does_not_double_debit_station_inventory() {
    let mut node = mem_node();
    let player_id = dawn_core::PlayerId(5);
    let new_ship_id = dawn_core::ShipId::new(dawn_core::NodeId(0), 99);

    node.apply_event_pub(DomainEvent::ShipAssembled(
        dawn_core::events::ShipAssembled {
            ship_id: new_ship_id,
            player_id,
            station_id: dawn_core::StationId(0),
            ship_type_id: dawn_core::ShipTypeId(1),
            tick: Tick(3),
        },
    ));

    assert_eq!(
        node.station_item_count(
            player_id,
            dawn_core::StationId(0),
            dawn_core::ItemId::PackagedShip(dawn_core::ShipTypeId(1))
        ),
        0,
        "replay must not debit a stack that was already consumed live"
    );
    assert!(node.owns_ship(player_id, new_ship_id));
    assert_eq!(
        node.docked_station(new_ship_id),
        Some(dawn_core::StationId(0))
    );
    assert!(!node.is_active_ship(player_id, new_ship_id));
}

#[test]
fn ship_spawned_event_replay_reconstructs_the_ship_from_scratch() {
    let mut node = mem_node();
    for def in crate::game_data::test_catalog().modules().to_vec() {
        node.register_module(def);
    }
    for def in crate::game_data::test_catalog().ship_types().to_vec() {
        node.register_ship_type(def);
    }
    let ship_id = dawn_core::ShipId::new(NodeId(0), 42);

    node.apply_event_pub(DomainEvent::ShipSpawned(dawn_core::events::ShipSpawned {
        ship_id,
        sector_id: SectorId(0),
        initial_position: dawn_core::AbsolutePosition::new(10.0, 0.0, 0.0),
        ship_type_id: crate::ship_types::SHIP_TYPE_MAGPIE,
        tick: Tick(1),
    }));

    assert_eq!(
        node.get_ship_position(ship_id),
        Some(Position::new(10.0, 0.0, 0.0))
    );
    let hp = node
        .get_ship_hp(ship_id)
        .expect("ship must exist after replay");
    assert!(hp > 0.0, "freshly replayed ship must start at full HP");
    let entity = *node.ships.index.get(&ship_id).unwrap();
    let inventory = node
        .world
        .get::<dawn_ecs::components::InventoryComp>(entity)
        .expect("Magpie replay must seed starter inventory (ADR-0032)");
    assert_eq!(
        inventory.items.len(),
        crate::game_data::test_catalog().modules().to_vec().len(),
        "starter inventory replay must be a pure function of module_registry, \
         matching the live spawn path exactly (INV-002)"
    );
}

#[test]
fn ship_spawned_event_replay_is_a_no_op_for_an_already_reconstructed_ship() {
    // restore_from replays every post-snapshot event in order; if a ship was
    // spawned then later touched by another event before the snapshot's
    // log_index, ShipSpawned must not double-insert or reset it.
    let mut node = mem_node();
    for def in crate::game_data::test_catalog().ship_types().to_vec() {
        node.register_ship_type(def);
    }
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.apply_event_pub(DomainEvent::DamageTaken(dawn_core::events::DamageTaken {
        ship_id,
        damage: 50.0,
        current_shield: 0.0,
        current_armor: 0.0,
        current_hull: 100.0,
        tick: Tick(1),
    }));

    node.apply_event_pub(DomainEvent::ShipSpawned(dawn_core::events::ShipSpawned {
        ship_id,
        sector_id: SectorId(0),
        initial_position: dawn_core::AbsolutePosition::ORIGIN,
        ship_type_id: dawn_core::ShipTypeId(1),
        tick: Tick(2),
    }));

    assert_eq!(
        node.get_ship_hp(ship_id),
        Some(100.0),
        "replaying ShipSpawned for a ship that already exists must not reset its HP"
    );
}

#[test]
fn velocity_changed_event_replay_integrates_position_across_the_tick_gap() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    // No prior velocity, so gap-tick integration only applies the new
    // velocity's own delta this call; the interesting behaviour under
    // test is that current_tick advances and the velocity is stored.
    node.apply_event_pub(DomainEvent::VelocityChanged(
        dawn_core::events::VelocityChanged {
            ship_id,
            velocity: Velocity {
                dx: 5.0,
                dy: 0.0,
                dz: 0.0,
            },
            tick: Tick(3),
        },
    ));

    let entity = *node.ships.index.get(&ship_id).unwrap();
    let vel = node
        .world
        .get::<dawn_ecs::components::VelocityComp>(entity)
        .unwrap()
        .0;
    assert_eq!(vel.dx, 5.0);
    assert_eq!(node.current_tick, Tick(3));
}

#[test]
fn ship_fitted_event_replay_restores_inventory_snapshot() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::ShipFitted(dawn_core::events::ShipFitted {
        ship_id,
        fitting: dawn_core::fitting::FittingSnapshot::empty(),
        inventory: vec![dawn_core::ItemId::ScrapMetal, dawn_core::ItemId::ScrapMetal],
        tick: Tick(4),
    }));

    let entity = *node.ships.index.get(&ship_id).unwrap();
    let inventory = node
        .world
        .get::<dawn_ecs::components::InventoryComp>(entity)
        .unwrap();
    assert_eq!(inventory.item_count(dawn_core::ItemId::ScrapMetal), 2);
}

#[test]
fn module_activated_event_replay_marks_the_slot_active_with_its_target() {
    use crate::modules;
    use dawn_core::{FitModuleCommand, SlotKind};

    let mut node = mem_node();
    for def in crate::game_data::test_catalog().modules().to_vec() {
        node.register_module(def);
    }
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    let target_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.fit_module(FitModuleCommand {
        ship_id,
        slot: SlotKind::High,
        module_id: modules::MODULE_RAILGUN_SMALL,
    });

    node.apply_event_pub(DomainEvent::ModuleActivated(
        dawn_core::events::ModuleActivated {
            ship_id,
            module_id: modules::MODULE_RAILGUN_SMALL,
            slot: SlotKind::High,
            target_ship_id: Some(target_id),
            tick: Tick(5),
        },
    ));

    let entity = *node.ships.index.get(&ship_id).unwrap();
    let mut fitting = node.world.get_mut::<FittingComp>(entity).unwrap();
    let slot = fitting
        .find_slot_mut(modules::MODULE_RAILGUN_SMALL, SlotKind::High)
        .unwrap();
    assert!(slot.is_active);
    assert_eq!(slot.target_ship_id, Some(target_id));
}

#[test]
fn target_locked_then_lock_lost_event_replay_round_trips_lock_comp() {
    let mut node = mem_node();
    let locker_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    let target_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::TargetLocked(dawn_core::events::TargetLocked {
        locker_id,
        target_id,
        tick: Tick(6),
    }));

    let entity = *node.ships.index.get(&locker_id).unwrap();
    {
        let lock = node
            .world
            .get::<dawn_ecs::components::LockComp>(entity)
            .unwrap();
        assert_eq!(lock.entries.len(), 1);
        assert_eq!(
            lock.entries[0].state,
            dawn_ecs::components::LockState::Locked
        );
    }

    node.apply_event_pub(DomainEvent::LockLost(dawn_core::events::LockLost {
        locker_id,
        target_id,
        tick: Tick(7),
    }));

    let lock = node
        .world
        .get::<dawn_ecs::components::LockComp>(entity)
        .unwrap();
    assert!(
        lock.entries.is_empty(),
        "replaying LockLost must remove the matching entry"
    );
}

#[test]
fn tackle_applied_then_released_event_replay_round_trips_tackled_comp() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    let tackler_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::TackleApplied(
        dawn_core::events::TackleApplied {
            ship_id,
            by: tackler_id,
            tick: Tick(8),
        },
    ));

    let entity = *node.ships.index.get(&ship_id).unwrap();
    {
        let tackled = node
            .world
            .get::<TackledComp>(entity)
            .expect("TackleApplied replay must insert TackledComp");
        assert_eq!(tackled.tacklers, vec![tackler_id]);
    }

    node.apply_event_pub(DomainEvent::TackleReleased(
        dawn_core::events::TackleReleased {
            ship_id,
            by: tackler_id,
            tick: Tick(9),
        },
    ));

    assert!(
        node.world.get::<TackledComp>(entity).is_none(),
        "releasing the only tackler must remove TackledComp entirely, \
         matching the live process_tackle behaviour"
    );
}

#[test]
fn anchor_rebased_event_replay_updates_anchor_and_offset() {
    let mut node = mem_node();
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);

    node.apply_event_pub(DomainEvent::AnchorRebased(
        dawn_core::events::AnchorRebased {
            ship_id,
            anchor: dawn_core::AnchorId(3),
            offset: Position::new(1.0, 2.0, 3.0),
            tick: Tick(10),
        },
    ));

    assert_eq!(node.get_ship_anchor(ship_id), Some(dawn_core::AnchorId(3)));
    assert_eq!(
        node.get_ship_position(ship_id),
        Some(Position::new(1.0, 2.0, 3.0))
    );
    assert_eq!(node.current_tick, Tick(10));
}

#[test]
fn ship_undocked_event_replay_clears_docked_state() {
    let mut node = mem_node();
    let player_id = dawn_core::PlayerId(5);
    let ship_id = node.spawn_ship(dawn_core::ShipTypeId(1), Position::ORIGIN, Velocity::ZERO);
    node.adopt_player_ship(ship_id, player_id);
    node.apply_event_pub(DomainEvent::ShipDocked(dawn_core::events::ShipDocked {
        ship_id,
        station_id: dawn_core::StationId(0),
        tick: Tick(11),
    }));
    assert!(node.is_ship_docked(ship_id));

    node.apply_event_pub(DomainEvent::ShipUndocked(dawn_core::events::ShipUndocked {
        ship_id,
        station_id: dawn_core::StationId(0),
        tick: Tick(12),
    }));

    assert!(!node.is_ship_docked(ship_id));
    assert_eq!(node.player_docked_station(player_id), None);
    assert_eq!(node.current_tick, Tick(12));
}
