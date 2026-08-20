//! Debug-only typed inbound fixtures used by the GdUnit4 client tests.
//!
//! Keeping these constructors outside the production decoder keeps the live
//! delivery path focused on validated wire messages while preserving the
//! binary-envelope test seam exposed by `ServerMessageDecoder`.

use dawn_core::CelestialBodyKind;
use dawn_protocol::{
    AbsPosWire, BuildableShipTypeWire, CelestialBodyWire, InitialStateWire, JumpGateWire,
    MarketOrderWire, MarketSnapshotWire, PlayerLoadoutWire, ServerFact,
    ServerFactDeactivationReason, ServerFactSlot, ServerMessage, ShipStateWire, SlotCapacityWire,
    SystemWire, VelWire,
};

pub(crate) fn message(kind: &str) -> Option<ServerMessage> {
    let position = AbsPosWire {
        x: 5.0 * 149_597_870_700.0 + 10.0,
        y: 20.0,
        z: 30.0,
    };
    let ship = ShipStateWire {
        ship_id: 11,
        ship_type_name: "Magpie".to_owned(),
        position,
        velocity: VelWire {
            dx: 4.0,
            dy: 5.0,
            dz: 6.0,
        },
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
    };
    let mut other_ship = ship.clone();
    other_ship.ship_id = 22;
    other_ship.ship_type_name = "Venture".to_owned();
    other_ship.max_shield = 250.0;
    other_ship.max_armor = 180.0;
    other_ship.max_hull = 120.0;
    other_ship.current_shield = 210.0;
    other_ship.current_armor = 160.0;
    other_ship.current_hull = 110.0;
    other_ship.cap_max = 80.0;
    other_ship.cap_recharge_per_tick = 4.0;
    let mut pending_ship = other_ship.clone();
    pending_ship.ship_id = 33;
    pending_ship.ship_type_name = "Prospect".to_owned();

    let loadout = |tick, active_ship_id| PlayerLoadoutWire {
        tick,
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
        active_ship_id,
        owned_ships: Vec::new(),
    };
    let docked_loadout = |tick, active_ship_id| {
        let mut result = loadout(tick, active_ship_id);
        result.docked_station_id = Some(5);
        result.docked_station_name = Some("Forge Station".to_owned());
        result
    };

    Some(match kind {
        "Welcome" => ServerMessage::Welcome {
            player_id: 5,
            ship_id: 11,
            resume_ticket: dawn_protocol::ResumeTicket::from_bytes([5; 32]),
        },
        "Redirect" => ServerMessage::Redirect {
            ws_addr: "127.0.0.1:7880".to_owned(),
            resume_ticket: dawn_protocol::ResumeTicket::from_bytes([5; 32]),
        },
        "AoiLeave" => ServerMessage::AoiLeave { ship_id: 19 },
        "AoiEnterPending" => ServerMessage::AoiEnter(pending_ship),
        "InitialState" => ServerMessage::InitialState(InitialStateWire {
            ships: vec![ship, other_ship],
            system_name: "Alpha".to_owned(),
            systems: vec![SystemWire {
                id: 2,
                name: "Beta".to_owned(),
            }],
            jump_gates: vec![JumpGateWire {
                gate_id: 7,
                position,
                activation_radius: 1000.0,
                to_system_name: "Beta".to_owned(),
            }],
            stations: vec![dawn_protocol::StationWire {
                station_id: 5,
                name: "Forge Station".to_owned(),
                position,
                docking_radius: 5000.0,
            }],
            celestial_bodies: vec![CelestialBodyWire {
                id: 9,
                kind: CelestialBodyKind::Star,
                name: "Sun".to_owned(),
                position,
                radius: 42.0,
                spectral_type: 0.5,
            }],
            buildable_ship_types: vec![BuildableShipTypeWire {
                ship_type_id: 7,
                name: "Magpie".to_owned(),
            }],
        }),
        "PlayerLoadoutSwitch" => ServerMessage::PlayerLoadout(loadout(12, Some(22))),
        "PlayerLoadoutUnknown" => ServerMessage::PlayerLoadout(loadout(13, Some(33))),
        "PlayerLoadoutUnknownDocked" => ServerMessage::PlayerLoadout(docked_loadout(13, Some(33))),
        "PlayerLoadoutDisembark" => ServerMessage::PlayerLoadout(loadout(14, None)),
        "ModuleActivated" => ServerMessage::Fact(ServerFact::ModuleActivated {
            ship_id: 11,
            module_id: 7,
            slot: ServerFactSlot::Mid,
            target_ship_id: None,
            tick: 15,
        }),
        "ModuleDeactivated" => ServerMessage::Fact(ServerFact::ModuleDeactivated {
            ship_id: 11,
            module_id: 7,
            slot: ServerFactSlot::Mid,
            reason: Some(ServerFactDeactivationReason::CapacitorExhausted),
            tick: 16,
        }),
        "MotionCorrection" => ServerMessage::MotionCorrection {
            ship_id: 11,
            position,
            velocity: VelWire {
                dx: 4.0,
                dy: 5.0,
                dz: -6.0,
            },
            tick: 42,
        },
        "ShipDocked" => ServerMessage::Fact(ServerFact::ShipDocked {
            ship_id: 11,
            station_id: 5,
            tick: 12,
        }),
        "ShipSpawnedPending" => ServerMessage::Fact(ServerFact::ShipSpawned {
            ship_id: 33,
            position,
            tick: 14,
        }),
        "MarketSnapshot" => ServerMessage::MarketSnapshot(MarketSnapshotWire {
            balance: 250,
            orders: vec![
                MarketOrderWire {
                    order_id: 1,
                    item_id: dawn_protocol::ItemWire::ScrapMetal,
                    side: "Ask".to_owned(),
                    price: 10,
                    quantity: 2,
                    is_own: true,
                },
                MarketOrderWire {
                    order_id: 2,
                    item_id: dawn_protocol::ItemWire::Module { module_id: 3 },
                    side: "Bid".to_owned(),
                    price: 20,
                    quantity: 1,
                    is_own: false,
                },
                MarketOrderWire {
                    order_id: 3,
                    item_id: dawn_protocol::ItemWire::PackagedShip { ship_type_id: 7 },
                    side: "Ask".to_owned(),
                    price: 30,
                    quantity: 1,
                    is_own: false,
                },
            ],
            notice: "Ready".to_owned(),
        }),
        _ => return None,
    })
}
