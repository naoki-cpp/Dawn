use dawn_client_core::{
    NavigationInput, ShipInput, ShipRegistration, WorldSessionEffect, WorldSessionState,
    WorldSessionUpdate,
};

fn player_ship(ship_type_name: &str) -> ShipInput {
    ShipInput {
        // `is_player` distinguishes player-controlled ships from non-player
        // entities. The connection ship ID selects which player ship is local.
        is_player: true,
        ship_type_name: ship_type_name.to_string(),
        max_shield: 100.0,
        max_armor: 90.0,
        max_hull: 80.0,
        current_shield: Some(70.0),
        current_armor: Some(60.0),
        current_hull: Some(50.0),
        cap_max: 40.0,
        cap_recharge_per_tick: 1.0,
    }
}

fn session() -> WorldSessionState {
    let mut state = WorldSessionState::default();
    let effect = state.apply_update(
        WorldSessionUpdate::InitialState {
            navigation: NavigationInput::default(),
            ships: vec![
                ShipRegistration {
                    ship_id: 11,
                    ship: player_ship("Magpie"),
                },
                ShipRegistration {
                    ship_id: 22,
                    ship: player_ship("Venture"),
                },
            ],
            connection_ship_id: 11,
        },
        None,
    );
    assert_eq!(
        effect,
        WorldSessionEffect::InitialState { player_ship_id: 11 }
    );
    state
}

#[test]
fn health_updates_preserve_registered_maxima() {
    let mut state = session();

    state.apply_update(
        WorldSessionUpdate::HealthChanged {
            ship_id: 11,
            shield: 40.0,
            armor: 30.0,
            hull: 20.0,
        },
        None,
    );

    let health = state.ship_hp().get(&11).expect("registered player ship");
    assert_eq!(health.shield, 40.0);
    assert_eq!(health.max_shield, 100.0);
    assert_eq!(state.player_health().hull, 20.0);
}

#[test]
fn lock_updates_only_change_the_players_lock() {
    let mut state = session();

    assert_eq!(
        state.apply_update(
            WorldSessionUpdate::TargetLocked {
                locker_id: 22,
                target_id: 99,
            },
            None,
        ),
        WorldSessionEffect::LockChanged { changed: false }
    );
    assert_eq!(state.player_lock_target(), -1);

    assert_eq!(
        state.apply_update(
            WorldSessionUpdate::TargetLocked {
                locker_id: 11,
                target_id: 22,
            },
            None,
        ),
        WorldSessionEffect::LockChanged { changed: true }
    );
    assert_eq!(state.player_lock_target(), 22);

    assert_eq!(
        state.apply_update(
            WorldSessionUpdate::LockLost {
                locker_id: 11,
                target_id: 22,
            },
            None,
        ),
        WorldSessionEffect::LockChanged { changed: true }
    );
    assert_eq!(state.player_lock_target(), -1);
}

#[test]
fn aoi_leave_preserves_lock_while_despawn_clears_it() {
    let mut aoi_state = session();
    aoi_state.apply_update(
        WorldSessionUpdate::TargetLocked {
            locker_id: 11,
            target_id: 22,
        },
        None,
    );

    assert_eq!(
        aoi_state.apply_update(
            WorldSessionUpdate::ShipLeft {
                ship_id: 22,
                clear_lock: false,
            },
            None,
        ),
        WorldSessionEffect::ShipRemoved { removed: true }
    );
    assert_eq!(aoi_state.player_lock_target(), 22);
    assert!(!aoi_state.has_ship(22));

    let mut despawn_state = session();
    despawn_state.apply_update(
        WorldSessionUpdate::TargetLocked {
            locker_id: 11,
            target_id: 22,
        },
        None,
    );
    despawn_state.apply_update(
        WorldSessionUpdate::ShipLeft {
            ship_id: 22,
            clear_lock: true,
        },
        None,
    );
    assert_eq!(despawn_state.player_lock_target(), -1);
}

#[test]
fn destroying_an_opponent_reports_the_presentation_outcome() {
    let mut state = session();

    let effect = state.apply_update(
        WorldSessionUpdate::ShipDestroyed { ship_id: 22 },
        None,
    );

    let WorldSessionEffect::ShipDestroyed(outcome) = effect else {
        panic!("ship destruction must return its typed outcome");
    };
    assert!(outcome.destroyed);
    assert!(!outcome.destroyed_player);
    assert!(outcome.destroyed_opponent);
    assert!(!state.has_ship(22));
}

#[test]
fn station_zero_is_docked_and_newer_dock_state_wins() {
    let mut state = session();

    assert_eq!(
        state.apply_update(
            WorldSessionUpdate::Docked {
                ship_id: 11,
                station_id: 0,
                station_name: "Forge Station".to_string(),
                tick: 20,
            },
            None,
        ),
        WorldSessionEffect::DockState { accepted: true }
    );
    assert!(state.is_docked());
    assert_eq!(state.docked_station_id(), 0);

    assert_eq!(
        state.apply_update(
            WorldSessionUpdate::Undocked {
                ship_id: 11,
                tick: 19,
            },
            None,
        ),
        WorldSessionEffect::DockState { accepted: false }
    );
    assert!(state.is_docked());
    assert_eq!(state.docked_station_id(), 0);
}

#[test]
fn newer_undock_rejects_stale_loadout_dock_context() {
    let mut state = session();
    state.apply_update(
        WorldSessionUpdate::Undocked {
            ship_id: 11,
            tick: 20,
        },
        None,
    );

    let effect = state.apply_update(
        WorldSessionUpdate::PlayerLoadout {
            active_ship_id: Some(11),
            docked_station_id: Some(3),
            docked_station_name: Some("Forge Station".to_string()),
            tick: 19,
        },
        None,
    );

    assert_eq!(
        effect,
        WorldSessionEffect::PlayerLoadout {
            active_changed: false,
            dock_changed: false,
        }
    );
    assert!(!state.is_docked());
    assert_eq!(state.docked_station_id(), -1);
}
