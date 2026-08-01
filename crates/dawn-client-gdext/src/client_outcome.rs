use dawn_wire::{
    AbsPosWire, EventWire, InitialStateWire, MarketSnapshotWire, ServerMessage, ShipStateWire,
    VelWire,
};

/// Godot-independent result of decoding one server frame.
///
/// This is the single projection seam between the wire schema and client
/// behavior. GDScript receives one of the typed Godot wrappers around these
/// outcomes; it never reconstructs `ServerMessage` from Dictionary tags.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ClientOutcome {
    Welcome {
        player_id: u64,
        ship_id: u64,
    },
    Redirect {
        ws_addr: String,
        player_id: u64,
        ship_id: u64,
    },
    Event(ClientEventOutcome),
    PlayerLoadout,
    InitialState(InitialStateWire),
    ModuleActivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
    },
    ModuleDeactivated {
        ship_id: u64,
        module_id: u32,
        slot: String,
        reason: Option<String>,
    },
    MotionCorrection {
        ship_id: u64,
        position: AbsPosWire,
        velocity: VelWire,
        tick: u64,
    },
    MarketSnapshot(MarketSnapshotWire),
}

/// World-facing event outcome dispatched by `main.gd`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ClientEventOutcome {
    Domain(EventWire),
    AoiEnter(ShipStateWire),
    AoiLeave { ship_id: u64 },
    PositionSnap { ship_id: u64, position: AbsPosWire },
}

impl ClientOutcome {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        ServerMessage::decode(bytes)
            .map(Self::from_message)
            .map_err(|error| error.to_string())
    }

    fn from_message(message: ServerMessage) -> Self {
        match message {
            ServerMessage::Welcome { player_id, ship_id } => Self::Welcome { player_id, ship_id },
            ServerMessage::Redirect {
                ws_addr,
                player_id,
                ship_id,
            } => Self::Redirect {
                ws_addr,
                player_id,
                ship_id,
            },
            ServerMessage::Event(EventWire::ModuleActivated {
                ship_id,
                module_id,
                slot,
                ..
            }) => Self::ModuleActivated {
                ship_id,
                module_id,
                slot,
            },
            ServerMessage::Event(EventWire::ModuleDeactivated {
                ship_id,
                module_id,
                slot,
                reason,
                ..
            }) => Self::ModuleDeactivated {
                ship_id,
                module_id,
                slot,
                reason,
            },
            ServerMessage::Event(event) => Self::Event(ClientEventOutcome::Domain(event)),
            ServerMessage::PlayerLoadout(_) => Self::PlayerLoadout,
            ServerMessage::InitialState(state) => Self::InitialState(state),
            ServerMessage::AoiEnter(ship) => Self::Event(ClientEventOutcome::AoiEnter(ship)),
            ServerMessage::AoiLeave { ship_id } => {
                Self::Event(ClientEventOutcome::AoiLeave { ship_id })
            }
            ServerMessage::PositionSnap { ship_id, position } => {
                Self::Event(ClientEventOutcome::PositionSnap { ship_id, position })
            }
            ServerMessage::MotionCorrection {
                ship_id,
                position,
                velocity,
                tick,
            } => Self::MotionCorrection {
                ship_id,
                position,
                velocity,
                tick,
            },
            ServerMessage::MarketSnapshot(snapshot) => Self::MarketSnapshot(snapshot),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_wire::{PlayerLoadoutWire, SlotCapacityWire};

    fn decode(message: ServerMessage) -> ClientOutcome {
        ClientOutcome::decode(&message.encode()).expect("valid raw ServerMessage frame")
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
    fn raw_frames_project_every_server_message_family() {
        assert!(matches!(
            decode(ServerMessage::Welcome {
                player_id: 1,
                ship_id: 7
            }),
            ClientOutcome::Welcome {
                player_id: 1,
                ship_id: 7
            }
        ));
        assert!(matches!(
            decode(ServerMessage::Redirect {
                ws_addr: "127.0.0.1:7880".to_owned(),
                player_id: 1,
                ship_id: 7,
            }),
            ClientOutcome::Redirect { .. }
        ));
        assert!(matches!(
            decode(ServerMessage::Event(EventWire::ShipDespawned {
                ship_id: 7,
                tick: 2
            })),
            ClientOutcome::Event(ClientEventOutcome::Domain(EventWire::ShipDespawned {
                ship_id: 7,
                tick: 2
            }))
        ));
        assert!(matches!(
            decode(ServerMessage::PlayerLoadout(loadout())),
            ClientOutcome::PlayerLoadout
        ));
        assert!(matches!(
            decode(ServerMessage::InitialState(InitialStateWire {
                ships: vec![ship()],
                system_name: "Alpha".to_owned(),
                systems: Vec::new(),
                jump_gates: Vec::new(),
                stations: Vec::new(),
                celestial_bodies: Vec::new(),
                buildable_ship_types: Vec::new(),
            })),
            ClientOutcome::InitialState(_)
        ));
        assert!(matches!(
            decode(ServerMessage::AoiEnter(ship())),
            ClientOutcome::Event(ClientEventOutcome::AoiEnter(_))
        ));
        assert!(matches!(
            decode(ServerMessage::AoiLeave { ship_id: 7 }),
            ClientOutcome::Event(ClientEventOutcome::AoiLeave { ship_id: 7 })
        ));
        assert!(matches!(
            decode(ServerMessage::PositionSnap {
                ship_id: 7,
                position: position()
            }),
            ClientOutcome::Event(ClientEventOutcome::PositionSnap { ship_id: 7, .. })
        ));
        assert!(matches!(
            decode(ServerMessage::MotionCorrection {
                ship_id: 7,
                position: position(),
                velocity: velocity(),
                tick: 3,
            }),
            ClientOutcome::MotionCorrection {
                ship_id: 7,
                tick: 3,
                ..
            }
        ));
        assert!(matches!(
            decode(ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 100,
                orders: Vec::new(),
                notice: "Ready".to_owned(),
            })),
            ClientOutcome::MarketSnapshot(_)
        ));
    }

    #[test]
    fn module_events_project_to_dedicated_connection_outcomes() {
        assert!(matches!(
            decode(ServerMessage::Event(EventWire::ModuleActivated {
                ship_id: 7,
                module_id: 3,
                slot: "High".to_owned(),
                target_ship_id: Some(9),
                tick: 4,
            })),
            ClientOutcome::ModuleActivated {
                ship_id: 7,
                module_id: 3,
                ..
            }
        ));
        assert!(matches!(
            decode(ServerMessage::Event(EventWire::ModuleDeactivated {
                ship_id: 7,
                module_id: 3,
                slot: "High".to_owned(),
                reason: Some("range".to_owned()),
                tick: 5,
            })),
            ClientOutcome::ModuleDeactivated {
                reason: Some(reason),
                ..
            } if reason == "range"
        ));
    }

    #[test]
    fn corrupted_raw_frame_is_rejected_before_projection() {
        assert!(ClientOutcome::decode(&[0xff, 0x01, 0x02]).is_err());
    }
}
