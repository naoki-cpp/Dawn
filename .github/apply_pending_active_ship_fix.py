from pathlib import Path

client_path = Path("crates/dawn-client-core/src/client_state.rs")
text = client_path.read_text()

old = '''            ClientFact::ShipEntered {
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
'''
new = '''            ClientFact::ShipEntered {
                ship,
                connection_ship_id,
            } => {
                let ship_id = ship.ship_id;
                return Ok(self.apply_ship_registration(
                    ship_id,
                    WorldSessionUpdate::ShipEntered {
                        ship,
                        connection_ship_id,
                    },
                ));
            }
            ClientFact::ShipSpawned {
                ship_id,
                connection_ship_id,
            } => {
                return Ok(self.apply_ship_registration(
                    ship_id,
                    WorldSessionUpdate::ShipSpawned {
                        ship_id,
                        connection_ship_id,
                    },
                ));
            }
'''
assert text.count(old) == 1, "ship registration arms changed"
text = text.replace(old, new)

old = '''                if let Some(loadout) = self.loadout.as_mut() {
                    let belongs_to_loadout = u64::try_from(ship_id)
                        .ok()
                        .is_some_and(|ship_id| loadout.active_ship_id == Some(ship_id));
                    if belongs_to_loadout {
                        loadout.apply_module_activation(module_id, active, forced_reason);
                    }
                }
'''
new = '''                let belongs_to_loadout = self.active_loadout_ship_id() == Some(ship_id);
                if belongs_to_loadout {
                    if let Some(loadout) = self.loadout.as_mut() {
                        loadout.apply_module_activation(module_id, active, forced_reason);
                    }
                }
'''
assert text.count(old) == 1, "module owner block changed"
text = text.replace(old, new)

marker = '''    fn replace_loadout(
'''
helpers = '''    fn apply_ship_registration(
        &mut self,
        ship_id: i64,
        update: WorldSessionUpdate,
    ) -> WorldSessionEffect {
        match self.apply_world(update) {
            WorldSessionEffect::ShipRegistered {
                registered,
                became_player,
            } => {
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
            other => other,
        }
    }

    fn active_loadout_ship_id(&self) -> Option<i64> {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.active_ship_id)
            .and_then(|ship_id| i64::try_from(ship_id).ok())
    }

'''
assert text.count(marker) == 1, "helper insertion point changed"
text = text.replace(marker, helpers + marker)

marker = '''    #[test]
    fn system_change_updates_only_the_player_system() {
'''
tests = '''    #[test]
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

'''
assert text.count(marker) == 1, "test insertion point changed"
client_path.write_text(text.replace(marker, tests + marker))

session_path = Path("crates/dawn-client-core/src/world_session.rs")
text = session_path.read_text()
old = '''    pub fn set_player_ship_id(&mut self, player_ship_id: i64) {
        self.player_ship_id = player_ship_id;
        let Some(ship) = self.ships.get(&player_ship_id).cloned() else {
'''
new = '''    pub fn set_player_ship_id(&mut self, player_ship_id: i64) {
        self.player_ship_id = player_ship_id;
        remove_id(&mut self.opponent_ship_ids, player_ship_id);
        let Some(ship) = self.ships.get(&player_ship_id).cloned() else {
'''
assert text.count(old) == 1, "player ship setter changed"
session_path.write_text(text.replace(old, new))
