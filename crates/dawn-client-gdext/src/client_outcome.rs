use dawn_core::ItemId;
use dawn_wire::{
    AbsPosWire, EventWire, InitialStateWire, ItemWire, MarketSnapshotWire, PlayerLoadoutWire,
    ResumeTicket, ServerMessage, ShipStateWire, VelWire,
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
        resume_ticket: ResumeTicket,
    },
    Redirect {
        ws_addr: String,
        resume_ticket: ResumeTicket,
    },
    Event(ClientEventOutcome),
    PlayerLoadout(PlayerLoadoutWire),
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
        let message = ServerMessage::decode(bytes).map_err(|error| error.to_string())?;
        validate_godot_integer_range(&message)?;
        Ok(Self::from_message(message))
    }

    fn from_message(message: ServerMessage) -> Self {
        match message {
            ServerMessage::Welcome {
                player_id,
                ship_id,
                resume_ticket,
            } => Self::Welcome {
                player_id,
                ship_id,
                resume_ticket,
            },
            ServerMessage::Redirect {
                ws_addr,
                resume_ticket,
            } => Self::Redirect {
                ws_addr,
                resume_ticket,
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
            ServerMessage::PlayerLoadout(loadout) => Self::PlayerLoadout(loadout),
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

fn ensure_godot_int(value: u64, field: &str) -> Result<(), String> {
    i64::try_from(value)
        .map(|_| ())
        .map_err(|_| format!("{field}={value} exceeds Godot's signed 64-bit integer range"))
}

fn ensure_client_u32(value: u64, field: &str) -> Result<(), String> {
    u32::try_from(value)
        .map(|_| ())
        .map_err(|_| format!("{field}={value} exceeds the client-side u32 range"))
}

fn ensure_canonical_item(item: ItemWire, field: &str) -> Result<(), String> {
    ItemId::try_from(item)
        .map(|_| ())
        .map_err(|error| format!("{field} has invalid canonical Item identity: {error:?}"))
}

fn validate_event(event: &EventWire) -> Result<(), String> {
    match event {
        EventWire::ShipSpawned { ship_id, tick, .. }
        | EventWire::VelocityChanged { ship_id, tick, .. }
        | EventWire::ShipDespawned { ship_id, tick }
        | EventWire::ShipDocked { ship_id, tick, .. }
        | EventWire::ShipUndocked { ship_id, tick, .. }
        | EventWire::ShipAssembled { ship_id, tick, .. }
        | EventWire::DamageTaken { ship_id, tick, .. }
        | EventWire::RepairApplied { ship_id, tick, .. }
        | EventWire::ModuleDeactivated { ship_id, tick, .. }
        | EventWire::JumpGateUsed { ship_id, tick, .. }
        | EventWire::StarSystemChanged { ship_id, tick, .. } => {
            ensure_godot_int(*ship_id, "event.ship_id")?;
            ensure_godot_int(*tick, "event.tick")?;
        }
        EventWire::ShipDestroyed {
            ship_id,
            killer_id,
            tick,
        } => {
            ensure_godot_int(*ship_id, "event.ship_id")?;
            ensure_godot_int(*killer_id, "event.killer_id")?;
            ensure_godot_int(*tick, "event.tick")?;
        }
        EventWire::TargetLocked {
            locker_id,
            target_id,
            tick,
        }
        | EventWire::LockLost {
            locker_id,
            target_id,
            tick,
        } => {
            ensure_godot_int(*locker_id, "event.locker_id")?;
            ensure_godot_int(*target_id, "event.target_id")?;
            ensure_godot_int(*tick, "event.tick")?;
        }
        EventWire::ModuleActivated {
            ship_id,
            target_ship_id,
            tick,
            ..
        } => {
            ensure_godot_int(*ship_id, "event.ship_id")?;
            if let Some(target_ship_id) = target_ship_id {
                ensure_godot_int(*target_ship_id, "event.target_ship_id")?;
            }
            ensure_godot_int(*tick, "event.tick")?;
        }
    }
    Ok(())
}

pub(crate) fn validate_player_loadout_godot_ranges(
    loadout: &PlayerLoadoutWire,
) -> Result<(), String> {
    ensure_godot_int(loadout.tick, "player_loadout.tick")?;
    if let Some(active_ship_id) = loadout.active_ship_id {
        ensure_godot_int(active_ship_id, "player_loadout.active_ship_id")?;
    }
    for ship in &loadout.owned_ships {
        ensure_godot_int(ship.ship_id, "player_loadout.owned_ships.ship_id")?;
    }
    for module in &loadout.modules {
        ensure_client_u32(
            module.cycle_time_ticks,
            "player_loadout.modules.cycle_time_ticks",
        )?;
    }
    for item in loadout.inventory.iter().chain(&loadout.station_inventory) {
        ensure_canonical_item(item.item_id, "player_loadout.inventory.item_id")?;
        ensure_godot_int(item.count, "player_loadout.inventory.count")?;
    }
    Ok(())
}

fn validate_godot_integer_range(message: &ServerMessage) -> Result<(), String> {
    match message {
        ServerMessage::Welcome {
            player_id, ship_id, ..
        } => {
            ensure_godot_int(*player_id, "player_id")?;
            ensure_godot_int(*ship_id, "ship_id")?;
        }
        ServerMessage::Redirect { .. } => {}
        ServerMessage::Event(event) => validate_event(event)?,
        ServerMessage::PlayerLoadout(loadout) => {
            validate_player_loadout_godot_ranges(loadout)?;
        }
        ServerMessage::InitialState(state) => {
            for ship in &state.ships {
                ensure_godot_int(ship.ship_id, "initial_state.ship_id")?;
            }
        }
        ServerMessage::AoiEnter(ship) => {
            ensure_godot_int(ship.ship_id, "aoi_enter.ship_id")?;
        }
        ServerMessage::AoiLeave { ship_id } | ServerMessage::PositionSnap { ship_id, .. } => {
            ensure_godot_int(*ship_id, "ship_id")?;
        }
        ServerMessage::MotionCorrection { ship_id, tick, .. } => {
            ensure_godot_int(*ship_id, "motion_correction.ship_id")?;
            ensure_godot_int(*tick, "motion_correction.tick")?;
        }
        ServerMessage::MarketSnapshot(snapshot) => {
            ensure_godot_int(snapshot.balance, "market.balance")?;
            for order in &snapshot.orders {
                ensure_canonical_item(order.item_id, "market.item_id")?;
                ensure_godot_int(order.order_id, "market.order_id")?;
                ensure_godot_int(order.price, "market.price")?;
                ensure_godot_int(order.quantity, "market.quantity")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::{ModuleKind, StatDelta};
    use dawn_wire::{ItemRowWire, ItemWire, ModuleRowWire, PlayerLoadoutWire, SlotCapacityWire};

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
                ship_id: 7,
                resume_ticket: dawn_wire::ResumeTicket::from_bytes([3; 32]),
            }),
            ClientOutcome::Welcome {
                player_id: 1,
                ship_id: 7,
                ..
            }
        ));
        assert!(matches!(
            decode(ServerMessage::Redirect {
                ws_addr: "127.0.0.1:7880".to_owned(),
                resume_ticket: dawn_wire::ResumeTicket::from_bytes([3; 32]),
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
            ClientOutcome::PlayerLoadout(_)
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
    fn unsigned_ids_outside_godot_int_range_are_rejected() {
        let invalid_ship_id = (i64::MAX as u64) + 1;
        let error = ClientOutcome::decode(
            &ServerMessage::Welcome {
                player_id: 1,
                ship_id: invalid_ship_id,
                resume_ticket: dawn_wire::ResumeTicket::from_bytes([3; 32]),
            }
            .encode(),
        )
        .expect_err("out-of-range ids must not be saturated");
        assert!(error.contains("ship_id"));
    }

    #[test]
    fn player_loadout_rejects_every_narrowing_overflow() {
        let invalid_godot_int = (i64::MAX as u64) + 1;

        let mut invalid_tick = loadout();
        invalid_tick.tick = invalid_godot_int;
        let error = ClientOutcome::decode(&ServerMessage::PlayerLoadout(invalid_tick).encode())
            .expect_err("loadout tick must fit Godot int");
        assert!(error.contains("player_loadout.tick"));

        let mut invalid_ship = loadout();
        invalid_ship.active_ship_id = Some(invalid_godot_int);
        let error = ClientOutcome::decode(&ServerMessage::PlayerLoadout(invalid_ship).encode())
            .expect_err("active ship ID must fit Godot int");
        assert!(error.contains("player_loadout.active_ship_id"));

        let mut invalid_count = loadout();
        invalid_count.inventory.push(ItemRowWire {
            item_id: ItemWire::ScrapMetal,
            name: "Scrap Metal".to_owned(),
            kind: "Commodity".to_owned(),
            slot: String::new(),
            count: invalid_godot_int,
        });
        let error = ClientOutcome::decode(&ServerMessage::PlayerLoadout(invalid_count).encode())
            .expect_err("inventory count must fit Godot int");
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
        let error = ClientOutcome::decode(&ServerMessage::PlayerLoadout(invalid_cycle).encode())
            .expect_err("module cycle length must fit client u32");
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
        let error = ClientOutcome::decode(&ServerMessage::PlayerLoadout(invalid_loadout).encode())
            .expect_err("zero module IDs must not reach Godot");
        assert!(error.contains("player_loadout.inventory.item_id"));

        let error = ClientOutcome::decode(
            &ServerMessage::MarketSnapshot(MarketSnapshotWire {
                balance: 0,
                orders: vec![dawn_wire::MarketOrderWire {
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
        .expect_err("zero ship type IDs must not reach Godot");
        assert!(error.contains("market.item_id"));
    }

    #[test]
    fn corrupted_raw_frame_is_rejected_before_projection() {
        assert!(ClientOutcome::decode(&[0xff, 0x01, 0x02]).is_err());
    }
}
