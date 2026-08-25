//! Wire- and Godot-independent policy for applying server facts to client state.

use std::fmt;

use crate::{
    NavigationInput, PlayerLoadoutMsg, ShipRegistration, WorldSessionEffect, WorldSessionState,
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
        ship_id: i64,
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
#[derive(Debug)]
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
        if !matches!(
            fact,
            ClientFact::InitialState { .. }
                | ClientFact::PlayerLoadout(_)
                | ClientFact::ModuleActivation { .. }
        ) {
            self.session.increment_event_count();
        }

        match fact {
            ClientFact::InitialState {
                navigation,
                ships,
                connection_ship_id,
            } => {
                self.session.reset();
                self.session.ingest_navigation(navigation);
                for registration in ships {
                    self.session.register_ship(
                        registration.ship_id,
                        registration.ship,
                        connection_ship_id,
                    );
                }
                Ok(WorldSessionEffect::InitialState {
                    player_ship_id: self.session.player_ship_id(),
                })
            }
            ClientFact::ShipEntered {
                ship,
                connection_ship_id,
            } => {
                let ship_id = ship.ship_id;
                let registered = !self.session.has_ship(ship_id);
                let became_player =
                    self.session
                        .register_ship(ship_id, ship.ship, connection_ship_id);
                Ok(self.reconcile_ship_registration(ship_id, registered, became_player))
            }
            ClientFact::ShipSpawned {
                ship_id,
                connection_ship_id,
            } => {
                let registered = !self.session.has_ship(ship_id);
                let became_player = self.session.register_ship(
                    ship_id,
                    crate::ShipInput {
                        max_shield: crate::default_max_shield(),
                        max_armor: crate::default_max_armor(),
                        max_hull: crate::default_max_hull(),
                        cap_max: crate::default_cap_max(),
                        cap_recharge_per_tick: crate::default_cap_recharge(),
                        ..crate::ShipInput::default()
                    },
                    connection_ship_id,
                );
                Ok(self.reconcile_ship_registration(ship_id, registered, became_player))
            }
            ClientFact::ShipLeft { ship_id, reason } => Ok(WorldSessionEffect::ShipRemoved {
                removed: self
                    .session
                    .remove_ship(ship_id, reason == ShipLeaveReason::Despawn),
            }),
            ClientFact::ShipDestroyed { ship_id } => Ok(WorldSessionEffect::ShipDestroyed(
                self.session.destroy_ship(ship_id),
            )),
            ClientFact::HealthChanged {
                ship_id,
                shield,
                armor,
                hull,
            } => {
                self.session.apply_hp_event(ship_id, shield, armor, hull);
                Ok(WorldSessionEffect::None)
            }
            ClientFact::TargetLocked {
                locker_id,
                target_id,
            } => Ok(WorldSessionEffect::LockChanged {
                changed: self.session.apply_target_locked(locker_id, target_id),
            }),
            ClientFact::LockLost {
                locker_id,
                target_id,
            } => Ok(WorldSessionEffect::LockChanged {
                changed: self.session.apply_lock_lost(locker_id, target_id),
            }),
            ClientFact::Docked {
                ship_id,
                station_id,
                tick,
            } => {
                let station_name = self.session.station_name(station_id);
                Ok(WorldSessionEffect::DockState {
                    accepted: self.session.apply_dock_event(
                        ship_id,
                        station_id,
                        station_name,
                        tick,
                    ),
                })
            }
            ClientFact::Undocked { ship_id, tick } => Ok(WorldSessionEffect::DockState {
                accepted: self.session.apply_undock_event(ship_id, tick),
            }),
            ClientFact::SystemChanged { ship_id, to_system } => {
                Ok(WorldSessionEffect::SystemChanged {
                    name: self.session.system_changed(ship_id, to_system),
                })
            }
            ClientFact::Tick { tick } => Ok(WorldSessionEffect::TickAdvanced {
                ticks_elapsed: self
                    .session
                    .advance_tick_from_event(tick, self.loadout.as_mut()),
            }),
            ClientFact::PlayerLoadout(loadout) => self.replace_loadout(loadout),
            ClientFact::ModuleActivation {
                ship_id,
                module_id,
                active,
                forced_reason,
            } => {
                let belongs_to_loadout = self.active_loadout_ship_id() == Some(ship_id);
                if belongs_to_loadout {
                    if let Some(loadout) = self.loadout.as_mut() {
                        loadout.apply_module_activation(module_id, active, forced_reason);
                    }
                }
                Ok(WorldSessionEffect::None)
            }
            ClientFact::ObservedEvent => Ok(WorldSessionEffect::None),
        }
    }

    fn reconcile_ship_registration(
        &mut self,
        ship_id: i64,
        registered: bool,
        became_player: bool,
    ) -> WorldSessionEffect {
        if became_player || self.active_loadout_ship_id() != Some(ship_id) {
            return WorldSessionEffect::ShipRegistered {
                registered,
                became_player,
            };
        }
        self.session.set_player_ship_id(ship_id);
        WorldSessionEffect::ShipRegistered {
            registered,
            became_player: true,
        }
    }

    fn active_loadout_ship_id(&self) -> Option<i64> {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.active_ship_id)
            .and_then(|ship_id| i64::try_from(ship_id).ok())
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

        let requested_ship_id = active_ship_id.unwrap_or(-1);
        let active_changed = requested_ship_id != self.session.player_ship_id()
            && (requested_ship_id < 0 || self.session.has_ship(requested_ship_id));
        if active_changed {
            self.session.set_player_ship_id(requested_ship_id);
        }
        let dock_changed = self.session.apply_dock_fitting(
            docked_station_id.unwrap_or(-1),
            docked_station_name.unwrap_or_default(),
            tick,
        );
        Ok(WorldSessionEffect::PlayerLoadout {
            active_changed,
            dock_changed,
        })
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
    use crate::{ModuleRow, PositionInput, ShipInput, StatDelta, StationInput, SystemNameInput};
    use dawn_core::ModuleKind;

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
            active_ship_id: Some(1),
            modules: vec![module(7, false)],
            ..PlayerLoadoutMsg::default()
        });

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 1,
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
    fn foreign_ship_module_activation_does_not_mutate_player_loadout() {
        let (mut session, _) = setup();
        let mut loadout = Some(PlayerLoadoutMsg {
            active_ship_id: Some(1),
            modules: vec![module(7, false)],
            ..PlayerLoadoutMsg::default()
        });

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 2,
                module_id: 7,
                active: true,
                forced_reason: "foreign".to_owned(),
            })
            .unwrap();

        let row = &loadout.as_ref().unwrap().modules[0];
        assert!(!row.is_active);
        assert_eq!(row.cycle_remaining, 7);
        assert!(row.forced_reason.is_empty());
    }

    #[test]
    fn known_active_ship_switch_routes_module_events_to_new_loadout_owner() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                active_ship_id: Some(2),
                modules: vec![module(7, false)],
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert_eq!(session.player_ship_id(), 2);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 1,
                module_id: 7,
                active: true,
                forced_reason: "old ship".to_owned(),
            })
            .unwrap();
        let row = &loadout.as_ref().unwrap().modules[0];
        assert!(!row.is_active);
        assert_eq!(row.cycle_remaining, 7);
        assert!(row.forced_reason.is_empty());

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 2,
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
    fn unknown_active_ship_switch_uses_loadout_owner_for_module_events() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                active_ship_id: Some(33),
                modules: vec![module(7, false)],
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert_eq!(session.player_ship_id(), 1);
        assert_eq!(loadout.as_ref().unwrap().active_ship_id, Some(33));

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 1,
                module_id: 7,
                active: true,
                forced_reason: "old ship".to_owned(),
            })
            .unwrap();
        let row = &loadout.as_ref().unwrap().modules[0];
        assert!(!row.is_active);
        assert_eq!(row.cycle_remaining, 7);
        assert!(row.forced_reason.is_empty());

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 33,
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
    fn pending_active_ship_switch_pauses_capacitor_until_registration() {
        let (mut session, mut loadout) = setup();
        let mut active_module = module(7, true);
        active_module.cycle_remaining = 0;
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                active_ship_id: Some(33),
                modules: vec![active_module],
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert_eq!(session.player_ship_id(), 1);
        assert_eq!(session.cap_current(), 100.0);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Tick { tick: 1 })
            .unwrap();
        assert_eq!(session.current_tick(), 1);
        assert_eq!(session.cap_current(), 100.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 0);

        session.advance_client_ticks(1, loadout.as_mut());
        assert_eq!(session.current_tick(), 2);
        assert_eq!(session.cap_current(), 100.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 0);

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipEntered {
                ship: ship(33, true),
                connection_ship_id: 1,
            })
            .unwrap();
        assert_eq!(
            effect,
            WorldSessionEffect::ShipRegistered {
                registered: true,
                became_player: true,
            }
        );
        assert_eq!(session.player_ship_id(), 33);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::Tick { tick: 3 })
            .unwrap();
        assert_eq!(session.cap_current(), 95.0);
        assert_eq!(loadout.as_ref().unwrap().modules[0].cycle_remaining, 10);
    }

    #[test]
    fn unknown_active_ship_switch_completes_when_ship_enters() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                active_ship_id: Some(33),
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert_eq!(session.player_ship_id(), 1);

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipEntered {
                ship: ship(33, true),
                connection_ship_id: 1,
            })
            .unwrap();

        assert_eq!(
            effect,
            WorldSessionEffect::ShipRegistered {
                registered: true,
                became_player: true,
            }
        );
        assert_eq!(session.player_ship_id(), 33);
        assert_eq!(session.player_ship_type_name(), "Ship 33");
        assert!(session.has_ship(1));
        assert!(!session.opponent_ship_ids().contains(&33));
    }

    #[test]
    fn unknown_active_ship_switch_completes_when_ship_spawns() {
        let (mut session, mut loadout) = setup();
        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg {
                active_ship_id: Some(44),
                ..PlayerLoadoutMsg::default()
            }))
            .unwrap();
        assert_eq!(session.player_ship_id(), 1);

        let effect = ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ShipSpawned {
                ship_id: 44,
                connection_ship_id: 1,
            })
            .unwrap();

        assert_eq!(
            effect,
            WorldSessionEffect::ShipRegistered {
                registered: true,
                became_player: true,
            }
        );
        assert_eq!(session.player_ship_id(), 44);
        assert!(session.has_ship(1));
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
    fn client_fact_application_accounts_for_each_server_event_once() {
        let (mut session, mut loadout) = setup();
        assert_eq!(session.event_count(), 0);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ObservedEvent)
            .unwrap();
        assert_eq!(session.event_count(), 1);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::ModuleActivation {
                ship_id: 1,
                module_id: 7,
                active: true,
                forced_reason: String::new(),
            })
            .unwrap();
        assert_eq!(session.event_count(), 1);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::PlayerLoadout(PlayerLoadoutMsg::default()))
            .unwrap();
        assert_eq!(session.event_count(), 1);

        ClientState::new(&mut session, &mut loadout)
            .apply(ClientFact::TargetLocked {
                locker_id: 1,
                target_id: 2,
            })
            .unwrap();
        assert_eq!(session.event_count(), 2);
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
