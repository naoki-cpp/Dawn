//! Wire- and Godot-independent policy for applying server facts to client state.

use std::fmt;

use crate::{
    NavigationInput, PlayerLoadoutMsg, ShipRegistration, WorldSessionEffect, WorldSessionState,
    WorldSessionUpdate,
};

/// Why a ship disappeared from the visible world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShipLeaveReason {
    /// The entity no longer exists, so any lock on it is invalid.
    Despawn,
    /// The entity only left the area of interest; retain the logical lock.
    AreaOfInterest,
}

/// One wire-independent fact learned from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientFact {
    InitialState {
        navigation: NavigationInput,
        ships: Vec<ShipRegistration>,
        connection_ship_id: i64,
    },
    ShipEntered {
        ship: ShipRegistration,
        connection_ship_id: i64,
    },
    ShipSpawned {
        ship_id: i64,
        connection_ship_id: i64,
    },
    ShipLeft {
        ship_id: i64,
        reason: ShipLeaveReason,
    },
    ShipDestroyed {
        ship_id: i64,
    },
    HealthChanged {
        ship_id: i64,
        shield: f64,
        armor: f64,
        hull: f64,
    },
    TargetLocked {
        locker_id: i64,
        target_id: i64,
    },
    LockLost {
        locker_id: i64,
        target_id: i64,
    },
    Docked {
        ship_id: i64,
        station_id: i64,
        tick: i64,
    },
    Undocked {
        ship_id: i64,
        tick: i64,
    },
    SystemChanged {
        ship_id: i64,
        to_system: i64,
    },
    Tick {
        tick: i64,
    },
    PlayerLoadout(PlayerLoadoutMsg),
    ModuleActivation {
        module_id: u32,
        active: bool,
        forced_reason: String,
    },
    /// An event whose only state effect is event accounting.
    ObservedEvent,
}

/// A wire value could not be represented by the signed client state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientStateError {
    pub field: &'static str,
    pub value: u64,
}

impl fmt::Display for ClientStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}={} exceeds the client state's signed 64-bit range",
            self.field, self.value
        )
    }
}

impl std::error::Error for ClientStateError {}

/// Deep client-state interface used by external adapters.
///
/// The session and loadout are borrowed together so adapters cannot reorder
/// cross-state rules such as loadout replacement, tick simulation, docking,
/// and module activation.
pub struct ClientState<'a> {
    session: &'a mut WorldSessionState,
    loadout: &'a mut Option<PlayerLoadoutMsg>,
}

impl<'a> ClientState<'a> {
    pub fn new(
        session: &'a mut WorldSessionState,
        loadout: &'a mut Option<PlayerLoadoutMsg>,
    ) -> Self {
        Self { session, loadout }
    }

    pub fn apply(&mut self, fact: ClientFact) -> Result<WorldSessionEffect, ClientStateError> {
        let update = match fact {
            ClientFact::InitialState {
                navigation,
                ships,
                connection_ship_id,
            } => WorldSessionUpdate::InitialState {
                navigation,
                ships,
                connection_ship_id,
            },
            ClientFact::ShipEntered {
                ship,
                connection_ship_id,
            } => WorldSessionUpdate::ShipEntered {
                ship,
                connection_ship_id,
            },
            ClientFact::ShipSpawned {
                ship_id,
                connection_ship_id,
            } => WorldSessionUpdate::ShipSpawned {
                ship_id,
                connection_ship_id,
            },
            ClientFact::ShipLeft { ship_id, reason } => WorldSessionUpdate::ShipLeft {
                ship_id,
                clear_lock: reason == ShipLeaveReason::Despawn,
            },
            ClientFact::ShipDestroyed { ship_id } => WorldSessionUpdate::ShipDestroyed { ship_id },
            ClientFact::HealthChanged {
                ship_id,
                shield,
                armor,
                hull,
            } => WorldSessionUpdate::HealthChanged {
                ship_id,
                shield,
                armor,
                hull,
            },
            ClientFact::TargetLocked {
                locker_id,
                target_id,
            } => WorldSessionUpdate::TargetLocked {
                locker_id,
                target_id,
            },
            ClientFact::LockLost {
                locker_id,
                target_id,
            } => WorldSessionUpdate::LockLost {
                locker_id,
                target_id,
            },
            ClientFact::Docked {
                ship_id,
                station_id,
                tick,
            } => WorldSessionUpdate::Docked {
                ship_id,
                station_id,
                station_name: self.session.station_name(station_id),
                tick,
            },
            ClientFact::Undocked { ship_id, tick } => {
                WorldSessionUpdate::Undocked { ship_id, tick }
            }
            ClientFact::SystemChanged { ship_id, to_system } => {
                WorldSessionUpdate::SystemChanged { ship_id, to_system }
            }
            ClientFact::Tick { tick } => WorldSessionUpdate::Tick { tick },
            ClientFact::PlayerLoadout(loadout) => return self.replace_loadout(loadout),
            ClientFact::ModuleActivation {
                module_id,
                active,
                forced_reason,
            } => {
                if let Some(loadout) = self.loadout.as_mut() {
                    loadout.apply_module_activation(module_id, active, forced_reason);
                }
                return Ok(WorldSessionEffect::None);
            }
            ClientFact::ObservedEvent => WorldSessionUpdate::ObservedEvent,
        };
        Ok(self.apply_world(update))
    }

    fn replace_loadout(
        &mut self,
        loadout: PlayerLoadoutMsg,
    ) -> Result<WorldSessionEffect, ClientStateError> {
        let active_ship_id = optional_i64(loadout.active_ship_id, "player_loadout.active_ship_id")?;
        let docked_station_id = loadout.docked_station_id.map(i64::from);
        let docked_station_name = loadout.docked_station_name.clone();
        let tick = client_i64(loadout.tick, "player_loadout.tick")?;

        *self.loadout = Some(loadout);
        Ok(self.apply_world(WorldSessionUpdate::PlayerLoadout {
            active_ship_id,
            docked_station_id,
            docked_station_name,
            tick,
        }))
    }

    fn apply_world(&mut self, update: WorldSessionUpdate) -> WorldSessionEffect {
        self.session.apply_update(update, self.loadout.as_mut())
    }
}

fn optional_i64(value: Option<u64>, field: &'static str) -> Result<Option<i64>, ClientStateError> {
    value.map(|value| client_i64(value, field)).transpose()
}

fn client_i64(value: u64, field: &'static str) -> Result<i64, ClientStateError> {
    i64::try_from(value).map_err(|_| ClientStateError { field, value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ModuleKind, ModuleRow, PositionInput, ShipInput, StatDelta, StationInput, SystemNameInput,
    };

    fn ship(ship_id: i64, is_player: bool) -> ShipRegistration {
        ShipRegistration {
            ship_id,
            ship: ShipInput {
                is_player,
                ship_type_name: format!("Ship {ship_id}"),
                max_shield: 100.0,
                max_armor: 100.0,
                max_hull: 100.0,
                current_shield: Some(100.0),
                current_armor: Some(100.0),
                current_hull: Some(100.0),
                cap_max: 100.0,
                cap_recharge_per_tick: 10.0,
            },
        }
    }

    fn module(module_id: u32, active: bool) -> ModuleRow {
        ModuleRow {
            slot: "High".to_owned(),
            index: 0,
            module_id,
            name: "Test module".to_owned(),
            kind: ModuleKind::Weapon,
            is_active: active,
            is_active_module: true,
            cap_cost_per_cycle: 5.0,
            cycle_time_ticks: 10,
            stat_delta: StatDelta::ZERO,
            cycle_remaining: 7,
            forced_reason: String::new(),
        }
    }

    fn setup() -> (WorldSessionState, Option<PlayerLoadoutMsg>) {
        let mut session = WorldSessionState::default();
        let mut loadout = None;
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::InitialState {
                navigation: NavigationInput {
                    system_name: "Alpha".to_owned(),
                    systems: vec![SystemNameInput {
                        id: 2,
                        name: "Beta".to_owned(),
                    }],
                    stations: vec![StationInput {
                        station_id: 5,
                        name: "Forge Station".to_owned(),
                        position: PositionInput::default(),
                        docking_radius: 5000.0,
                    }],
                    ..NavigationInput::default()
                },
                ships: vec![ship(1, true), ship(2, false)],
                connection_ship_id: 1,
            })
            .unwrap();
        (session, loadout)
    }

    #[test]
    fn despawn_clears_lock_but_aoi_leave_preserves_it() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::TargetLocked {
                locker_id: 1,
                target_id: 2,
            })
            .unwrap();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipLeft {
                ship_id: 2,
                reason: ShipLeaveReason::AreaOfInterest,
            })
            .unwrap();
        assert_eq!(session.player_lock_target(), 2);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipEntered {
                ship: ship(2, false),
                connection_ship_id: 1,
            })
            .unwrap();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipLeft {
                ship_id: 2,
                reason: ShipLeaveReason::Despawn,
            })
            .unwrap();
        assert_eq!(session.player_lock_target(), -1);
    }

    #[test]
    fn dock_fact_resolves_station_name_and_rejects_stale_event() {
        let (mut session, mut loadout) = setup();
        assert_eq!(
            ClientState::new(&mut session, &mut loadout)
                .apply(ClientFact::Docked {
                    ship_id: 1,
                    station_id: 5,
                    tick: 10,
                })
                .unwrap(),
            WorldSessionEffect::DockState { accepted: true }
        );
        assert_eq!(session.docked_station_name(), "Forge Station");
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Undocked {
                ship_id: 1,
                tick: 11,
            })
            .unwrap();
        assert_eq!(
            ClientState::new(&mut session, &mut loadout)
                .apply(ClientFact::Docked {
                    ship_id: 1,
                    station_id: 5,
                    tick: 10,
                })
                .unwrap(),
            WorldSessionEffect::DockState { accepted: false }
        );
    }

    #[test]
    fn loadout_is_replaced_before_session_reconciliation() {
        let (mut session, mut loadout) = setup();
        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                tick: 4,
                active_ship_id: Some(2),
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert!(matches!(effect, WorldSessionEffect::PlayerLoadout { .. }));
        assert_eq!(session.player_ship_id(), 2);
        assert_eq!(loadout.as_ref().unwrap().tick, 4);
    }

    #[test]
    fn invalid_loadout_range_does_not_replace_state() {
        let (mut session, mut loadout) = setup();
        let error = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                tick: (i64::MAX as u64) + 1,
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap_err();
        assert_eq!(error.field, "player_loadout.tick");
        assert!(loadout.is_none());
    }

    #[test]
    fn tick_advances_session_and_capacitor_through_the_shared_loadout() {
        let (mut session, _) = setup();
        let mut active_module = module(7, true);
        active_module.cycle_remaining = 0;
        let mut loadout = Some(PlayerLoadoutMsg {
            active_ship_id: Some(1),
            modules: vec![active_module],
            ..PlayerLoadoutMsg::default()
        });

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Tick { tick: 1 })
            .unwrap();

        assert_eq!(
            effect,
            WorldSessionEffect::TickAdvanced { ticks_elapsed: 1 }
        );
        assert_eq!(session.current_tick(), 1);
        assert_eq!(session.cap_current(), 95.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 10);
    }

    #[test]
    fn module_activation_updates_loadout_state_and_resets_cycle() {
        let (mut session, _) = setup();
        let mut loadout = Some(PlayerLoadoutMsg {
            modules: vec![module(7, false)],
            ..PlayerLoadoutMsg::default()
        });

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                module_id: 7,
                active: true,
                forced_reason: String::new(),
            })
            .unwrap();

        let row = &loadout.as_ref().unwrap().modules[0];
        assert!(row.is_active);
        assert_eq!(row.cycle_remaining, 0);
    }

    #[test]
    fn system_change_updates_only_the_player_system() {
        let (mut session, mut loadout) = setup();
        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::SystemChanged {
                ship_id: 1,
                to_system: 2,
            })
            .unwrap();

        assert_eq!(
            effect,
            WorldSessionEffect::SystemChanged {
                name: Some("Beta".to_owned())
            }
        );
        assert_eq!(session.current_system_name(), "Beta");
    }

    #[test]
    fn initial_state_resets_old_state_before_registering_new_ships() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::TargetLocked {
                locker_id: 1,
                target_id: 2,
            })
            .unwrap();

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::InitialState {
                navigation: NavigationInput {
                    system_name: "Gamma".to_owned(),
                    ..NavigationInput::default()
                },
                ships: vec![ship(3, true)],
                connection_ship_id: 3,
            })
            .unwrap();

        assert_eq!(session.current_system_name(), "Gamma");
        assert_eq!(session.player_ship_id(), 3);
        assert_eq!(session.player_lock_target(), -1);
        assert_eq!(session.event_count(), 0);
        assert_eq!(session.ship_count(), 1);
    }

    #[test]
    fn ship_spawn_and_destroy_lifecycle_is_reported_by_effects() {
        let (mut session, mut loadout) = setup();
        assert_eq!(
            ClientState::new(&mut session, &mut loadout)
                .apply(ClientFact::ShipSpawned {
                    ship_id: 4,
                    connection_ship_id: 1,
                })
                .unwrap(),
            WorldSessionEffect::ShipRegistered {
                registered: true,
                became_player: false,
            }
        );
        assert!(session.has_ship(4));

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipDestroyed { ship_id: 4 })
            .unwrap();
        let WorldSessionEffect::ShipDestroyed(outcome) = effect else {
            panic!("expected destruction effect");
        };
        assert!(outcome.destroyed);
        assert!(!session.has_ship(4));
    }
}
