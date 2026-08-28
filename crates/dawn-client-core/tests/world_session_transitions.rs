use dawn_client_core::{
    ClientFact, ClientState, NavigationInput, PlayerLoadoutMsg, ShipInput, ShipLeaveReason,
    ShipRegistration, WorldSessionEffect, WorldSessionState,
};
use dawn_core::{NodeId, ShipId, StationId};

fn ship_id(id: u64) -> ShipId {
    ShipId::new(NodeId(0), id)
}

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

fn apply(
    state: &mut WorldSessionState,
    loadout: &mut Option<PlayerLoadoutMsg>,
    fact: ClientFact,
) -> WorldSessionEffect {
    ClientState::new(state, loadout)
        .apply(fact)
        .expect("test fact is representable by client state")
}

fn session() -> (WorldSessionState, Option<PlayerLoadoutMsg>) {
    let mut state = WorldSessionState::default();
    let mut loadout = None;
    let effect = apply(
        &mut state,
        &mut loadout,
        ClientFact::InitialState {
            navigation: NavigationInput::default(),
            ships: vec![
                ShipRegistration {
                    ship_id: ship_id(11),
                    ship: player_ship("Magpie"),
                },
                ShipRegistration {
                    ship_id: ship_id(22),
                    ship: player_ship("Venture"),
                },
            ],
            connection_ship_id: ship_id(11),
        },
    );
    assert_eq!(
        effect,
        WorldSessionEffect::InitialState {
            player_ship_id: Some(ship_id(11)),
        }
    );
    (state, loadout)
}

#[test]
fn health_updates_preserve_registered_maxima() {
    let (mut state, mut loadout) = session();

    apply(
        &mut state,
        &mut loadout,
        ClientFact::HealthChanged {
            ship_id: ship_id(11),
            shield: 40.0,
            armor: 30.0,
            hull: 20.0,
        },
    );

    let health = state
        .ship_hp()
        .get(&ship_id(11))
        .expect("registered player ship");
    assert_eq!(health.shield, 40.0);
    assert_eq!(health.max_shield, 100.0);
    assert_eq!(state.player_health().hull, 20.0);
}

#[test]
fn lock_updates_only_change_the_players_lock() {
    let (mut state, mut loadout) = session();

    assert_eq!(
        apply(
            &mut state,
            &mut loadout,
            ClientFact::TargetLocked {
                locker_id: ship_id(22),
                target_id: ship_id(99),
            },
        ),
        WorldSessionEffect::LockChanged { changed: false }
    );
    assert_eq!(state.player_lock_target(), None);

    assert_eq!(
        apply(
            &mut state,
            &mut loadout,
            ClientFact::TargetLocked {
                locker_id: ship_id(11),
                target_id: ship_id(22),
            },
        ),
        WorldSessionEffect::LockChanged { changed: true }
    );
    assert_eq!(state.player_lock_target(), Some(ship_id(22)));

    assert_eq!(
        apply(
            &mut state,
            &mut loadout,
            ClientFact::LockLost {
                locker_id: ship_id(11),
                target_id: ship_id(22),
            },
        ),
        WorldSessionEffect::LockChanged { changed: true }
    );
    assert_eq!(state.player_lock_target(), None);
}

#[test]
fn aoi_leave_preserves_lock_while_despawn_clears_it() {
    let (mut aoi_state, mut aoi_loadout) = session();
    apply(
        &mut aoi_state,
        &mut aoi_loadout,
        ClientFact::TargetLocked {
            locker_id: ship_id(11),
            target_id: ship_id(22),
        },
    );

    assert_eq!(
        apply(
            &mut aoi_state,
            &mut aoi_loadout,
            ClientFact::ShipLeft {
                ship_id: ship_id(22),
                reason: ShipLeaveReason::AreaOfInterest,
            },
        ),
        WorldSessionEffect::ShipRemoved { removed: true }
    );
    assert_eq!(aoi_state.player_lock_target(), Some(ship_id(22)));
    assert!(!aoi_state.has_ship(ship_id(22)));

    let (mut despawn_state, mut despawn_loadout) = session();
    apply(
        &mut despawn_state,
        &mut despawn_loadout,
        ClientFact::TargetLocked {
            locker_id: ship_id(11),
            target_id: ship_id(22),
        },
    );
    apply(
        &mut despawn_state,
        &mut despawn_loadout,
        ClientFact::ShipLeft {
            ship_id: ship_id(22),
            reason: ShipLeaveReason::Despawn,
        },
    );
    assert_eq!(despawn_state.player_lock_target(), None);
}

#[test]
fn destroying_an_opponent_reports_the_presentation_outcome() {
    let (mut state, mut loadout) = session();

    let effect = apply(
        &mut state,
        &mut loadout,
        ClientFact::ShipDestroyed {
            ship_id: ship_id(22),
        },
    );

    let WorldSessionEffect::ShipDestroyed(outcome) = effect else {
        panic!("ship destruction must return its typed outcome");
    };
    assert!(outcome.destroyed);
    assert!(!outcome.destroyed_player);
    assert!(outcome.destroyed_opponent);
    assert!(!state.has_ship(ship_id(22)));
}

#[test]
fn station_zero_is_docked_and_newer_dock_state_wins() {
    let (mut state, mut loadout) = session();

    assert_eq!(
        apply(
            &mut state,
            &mut loadout,
            ClientFact::Docked {
                ship_id: ship_id(11),
                station_id: StationId(0),
                tick: 20,
            },
        ),
        WorldSessionEffect::DockState { accepted: true }
    );
    assert!(state.is_docked());
    assert_eq!(state.docked_station_id(), Some(StationId(0)));

    assert_eq!(
        apply(
            &mut state,
            &mut loadout,
            ClientFact::Undocked {
                ship_id: ship_id(11),
                tick: 19,
            },
        ),
        WorldSessionEffect::DockState { accepted: false }
    );
    assert!(state.is_docked());
    assert_eq!(state.docked_station_id(), Some(StationId(0)));
}

#[test]
fn newer_undock_rejects_stale_loadout_dock_context() {
    let (mut state, mut loadout) = session();
    apply(
        &mut state,
        &mut loadout,
        ClientFact::Undocked {
            ship_id: ship_id(11),
            tick: 20,
        },
    );

    let effect = apply(
        &mut state,
        &mut loadout,
        ClientFact::PlayerLoadout(PlayerLoadoutMsg {
            active_ship_id: Some(ship_id(11)),
            docked_station_id: Some(StationId(3)),
            docked_station_name: Some("Forge Station".to_string()),
            tick: 19,
            ..PlayerLoadoutMsg::default()
        }),
    );

    assert_eq!(
        effect,
        WorldSessionEffect::PlayerLoadout {
            active_changed: false,
            dock_changed: false,
        }
    );
    assert!(!state.is_docked());
    assert_eq!(state.docked_station_id(), None);
}
