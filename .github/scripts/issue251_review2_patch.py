from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


# 1) Keep ship identity in ModuleActivation and ignore foreign-ship events.
path = Path("crates/dawn-client-core/src/client_state.rs")
text = path.read_text()
text = replace_once(
    text,
    """    ModuleActivation {
        module_id: u32,
        active: bool,
        forced_reason: String,
    },
""",
    """    ModuleActivation {
        ship_id: i64,
        module_id: u32,
        active: bool,
        forced_reason: String,
    },
""",
    "ClientFact::ModuleActivation declaration",
)
text = replace_once(
    text,
    """            ClientFact::ModuleActivation {
                module_id,
                active,
                forced_reason,
            } => {
                if let Some(loadout) = self.loadout.as_mut() {
                    loadout.apply_module_activation(module_id, active, forced_reason);
                }
                return Ok(WorldSessionEffect::None);
            }
""",
    """            ClientFact::ModuleActivation {
                ship_id,
                module_id,
                active,
                forced_reason,
            } => {
                if ship_id == self.session.player_ship_id() {
                    if let Some(loadout) = self.loadout.as_mut() {
                        loadout.apply_module_activation(module_id, active, forced_reason);
                    }
                }
                return Ok(WorldSessionEffect::None);
            }
""",
    "ClientFact::ModuleActivation application",
)
text = replace_once(
    text,
    """            .apply(ClientFact::ModuleActivation {
                module_id: 7,
                active: true,
                forced_reason: String::new(),
            })
""",
    """            .apply(ClientFact::ModuleActivation {
                ship_id: 1,
                module_id: 7,
                active: true,
                forced_reason: String::new(),
            })
""",
    "player module activation test",
)
text = replace_once(
    text,
    """        assert_eq!(row.cycle_remaining, 0);
    }

    #[test]
    fn system_change_updates_only_the_player_system() {
""",
    """        assert_eq!(row.cycle_remaining, 0);
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
    fn system_change_updates_only_the_player_system() {
""",
    "foreign module activation regression test insertion",
)
path.write_text(text)

# 2) Preserve ship identity while adapting module events.
path = Path("crates/dawn-client-gdext/src/server_message_gd.rs")
text = path.read_text()
text = replace_once(
    text,
    """                    ClientFact::ModuleActivation {
                        module_id: *module_id,
                        active: true,
                        forced_reason: String::new(),
                    },
""",
    """                    ClientFact::ModuleActivation {
                        ship_id: godot_i64(*ship_id),
                        module_id: *module_id,
                        active: true,
                        forced_reason: String::new(),
                    },
""",
    "activated module fact",
)
text = replace_once(
    text,
    """                    ClientFact::ModuleActivation {
                        module_id: *module_id,
                        active: false,
                        forced_reason: reason.to_owned(),
                    },
""",
    """                    ClientFact::ModuleActivation {
                        ship_id: godot_i64(*ship_id),
                        module_id: *module_id,
                        active: false,
                        forced_reason: reason.to_owned(),
                    },
""",
    "deactivated module fact",
)
path.write_text(text)

# 3) Restore validation regression coverage without restoring ClientOutcome.
path = Path("crates/dawn-client-gdext/src/server_message_validation.rs")
text = path.read_text()
marker = "\n#[cfg(test)]\nmod tests {"
start = text.find(marker)
if start < 0:
    raise SystemExit("validation test module marker not found")
text = text[:start] + r'''
#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{ModuleKind, StatDelta};
    use dawn_wire::{
        AbsPosWire, InitialStateWire, ItemRowWire, ItemWire, MarketOrderWire,
        MarketSnapshotWire, ModuleRowWire, OwnedShipRowWire, PlayerLoadoutWire,
        ShipStateWire, SlotCapacityWire, VelWire,
    };

    fn decode(message: ServerMessage) -> ServerMessage {
        decode_server_message(&message.encode()).expect("valid raw ServerMessage frame")
    }

    fn position() -> AbsPosWire {
        AbsPosWire {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        }
    }

    fn velocity() -> VelWire {
        VelWire {
            dx: 4.0,
            dy: 5.0,
            dz: 6.0,
        }
    }

    fn ship() -> ShipStateWire {
        ShipStateWire {
            ship_id: 7,
            ship_type_name: "Test Ship".to_owned(),
            position: position(),
            velocity: velocity(),
            max_speed: 500.0,
            mass: 10_000.0,
            inertia_modifier: 0.3,
            max_shield: 100.0,
            max_armor: 90.0,
            max_hull: 80.0,
            current_shield: 70.0,
            current_armor: 60.0,
            current_hull: 50.0,
            cap_max: 40.0,
            cap_recharge_per_tick: 1.0,
            is_player: true,
        }
    }

    fn loadout() -> PlayerLoadoutWire {
        PlayerLoadoutWire {
            tick: 1,
            modules: Vec::new(),
            inventory: Vec::new(),
            station_inventory: Vec::new(),
            docked_station_id: None,
            docked_station_name: None,
            slot_capacity: SlotCapacityWire {
                high: 0,
                mid: 0,
                low: 0,
                rig: 0,
            },
            active_ship_id: Some(7),
            owned_ships: Vec::new(),
        }
    }

    #[test]
    fn raw_frames_decode_every_server_message_family_without_a_second_mirror() {
        let messages = vec![
            ServerMessage::Welcome {
                player_id: 1,
                ship_id: 7,
            },
            ServerMessage::Redirect {
                ws_addr: "127.0.0.1:7880".to_owned(),
                player_id: 1,
                ship_id: 7,
            },
            ServerMessage::Event(EventWire::ShipDespawned {
                ship_id: 7,
                tick: 2,
            }),
            ServerMessage::PlayerLoadout(loadout()),
            ServerMessage::InitialState(InitialStateWire {
                ships: vec![ship()],
                system_name: "Alpha".to_owned(),
                systems: Vec::new(),
                jump_gates: Vec::new(),
                stations: Vec::new(),
                celestial_bodies: Vec::new(),
                buildable_ship_types: Vec::new(),
            }),
            ServerMessage::AoiEnter(ship()),
            ServerMessage::AoiLeave { ship_id: 7 },
            ServerMessage::PositionSnap {
                ship_id: 7,
                position: position(),
            },
            ServerMessage::MotionCorrection {
                ship_id: 7,
                position: position(),
                velocity: velocity(),
                tick: 3,
            },
            ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 100,
                orders: Vec::new(),
                notice: "Ready".to_owned(),
            }),
        ];

        for message in messages {
            assert_eq!(decode(message.clone()), message);
        }
    }

    #[test]
    fn module_events_decode_with_their_ship_identity() {
        let activated = ServerMessage::Event(EventWire::ModuleActivated {
            ship_id: 7,
            module_id: 3,
            slot: "High".to_owned(),
            target_ship_id: Some(9),
            tick: 4,
        });
        assert_eq!(decode(activated.clone()), activated);

        let deactivated = ServerMessage::Event(EventWire::ModuleDeactivated {
            ship_id: 7,
            module_id: 3,
            slot: "High".to_owned(),
            reason: Some("range".to_owned()),
            tick: 5,
        });
        assert_eq!(decode(deactivated.clone()), deactivated);
    }

    #[test]
    fn unsigned_ids_outside_godot_int_range_are_rejected() {
        let invalid = (i64::MAX as u64) + 1;
        let error = decode_server_message(
            &ServerMessage::Welcome {
                player_id: 1,
                ship_id: invalid,
            }
            .encode(),
        )
        .unwrap_err();
        assert!(error.contains("ship_id"));
    }

    #[test]
    fn player_loadout_rejects_every_narrowing_overflow() {
        let invalid_godot_int = (i64::MAX as u64) + 1;

        let mut invalid_tick = loadout();
        invalid_tick.tick = invalid_godot_int;
        let error =
            decode_server_message(&ServerMessage::PlayerLoadout(invalid_tick).encode()).unwrap_err();
        assert!(error.contains("player_loadout.tick"));

        let mut invalid_ship = loadout();
        invalid_ship.active_ship_id = Some(invalid_godot_int);
        let error =
            decode_server_message(&ServerMessage::PlayerLoadout(invalid_ship).encode()).unwrap_err();
        assert!(error.contains("player_loadout.active_ship_id"));

        let mut invalid_owned_ship = loadout();
        invalid_owned_ship.owned_ships.push(OwnedShipRowWire {
            ship_id: invalid_godot_int,
            ship_type_id: None,
            ship_type_name: None,
            docked_station_id: None,
            is_active: false,
        });
        let error = decode_server_message(
            &ServerMessage::PlayerLoadout(invalid_owned_ship).encode(),
        )
        .unwrap_err();
        assert!(error.contains("player_loadout.owned_ships.ship_id"));

        let mut invalid_count = loadout();
        invalid_count.inventory.push(ItemRowWire {
            item_id: ItemWire::ScrapMetal,
            name: "Scrap Metal".to_owned(),
            kind: "Commodity".to_owned(),
            slot: String::new(),
            count: invalid_godot_int,
        });
        let error =
            decode_server_message(&ServerMessage::PlayerLoadout(invalid_count).encode()).unwrap_err();
        assert!(error.contains("player_loadout.inventory.count"));

        let mut invalid_cycle = loadout();
        invalid_cycle.modules.push(ModuleRowWire {
            slot: "High".to_owned(),
            index: 0,
            module_id: 3,
            name: "Test Module".to_owned(),
            kind: ModuleKind::Weapon,
            is_active: false,
            is_active_module: true,
            cap_cost_per_cycle: 1.0,
            cycle_time_ticks: (u32::MAX as u64) + 1,
            stat_delta: StatDelta::ZERO,
        });
        let error =
            decode_server_message(&ServerMessage::PlayerLoadout(invalid_cycle).encode()).unwrap_err();
        assert!(error.contains("player_loadout.modules.cycle_time_ticks"));
    }

    #[test]
    fn invalid_item_identities_are_rejected_before_projection() {
        let mut invalid_loadout = loadout();
        invalid_loadout.inventory.push(ItemRowWire {
            item_id: ItemWire::Module { module_id: 0 },
            name: "Invalid".to_owned(),
            kind: String::new(),
            slot: String::new(),
            count: 1,
        });
        let error = decode_server_message(
            &ServerMessage::PlayerLoadout(invalid_loadout).encode(),
        )
        .unwrap_err();
        assert!(error.contains("player_loadout.inventory.item_id"));

        let error = decode_server_message(
            &ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 0,
                orders: vec![MarketOrderWire {
                    order_id: 1,
                    item_id: ItemWire::PackagedShip { ship_type_id: 0 },
                    side: "Bid".to_owned(),
                    price: 1,
                    quantity: 1,
                    is_own: false,
                }],
                notice: String::new(),
            })
            .encode(),
        )
        .unwrap_err();
        assert!(error.contains("market.item_id"));
    }

    #[test]
    fn corrupted_raw_frame_is_rejected_before_projection() {
        assert!(decode_server_message(&[0xff, 0x01, 0x02]).is_err());
    }
}
'''
path.write_text(text)
