//! Typed Godot-facing presentation records for decoded server outcomes.
//!
//! These records carry only values needed for scene/HUD updates. The matching
//! client state transition has already been applied to `WorldSessionState`
//! before GDScript receives one of them.

use dawn_core::ItemId;
use dawn_wire::{MarketSnapshotWire, ShipStateWire};
use godot::prelude::*;

use crate::item_identity_gd::ItemIdentity;

#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ShipPresentation {
    #[var]
    ship_id: i64,
    #[var]
    ship_type_name: GString,
    #[var]
    position: PackedFloat64Array,
    #[var]
    velocity: PackedFloat64Array,
    #[var]
    max_speed: f64,
    #[var]
    mass: f64,
    #[var]
    inertia_modifier: f64,
    #[var]
    max_shield: f64,
    #[var]
    max_armor: f64,
    #[var]
    max_hull: f64,
    #[var]
    current_shield: f64,
    #[var]
    current_armor: f64,
    #[var]
    current_hull: f64,
    #[var]
    cap_max: f64,
    #[var]
    cap_recharge_per_tick: f64,
    #[var]
    is_player: bool,
}

impl ShipPresentation {
    pub(crate) fn wrap(ship: &ShipStateWire) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_id: godot_i64(ship.ship_id),
            ship_type_name: ship.ship_type_name.as_str().into(),
            position: PackedFloat64Array::from([ship.position.x, ship.position.y, ship.position.z]),
            velocity: PackedFloat64Array::from([
                ship.velocity.dx,
                ship.velocity.dy,
                ship.velocity.dz,
            ]),
            max_speed: ship.max_speed,
            mass: ship.mass,
            inertia_modifier: ship.inertia_modifier,
            max_shield: f64::from(ship.max_shield),
            max_armor: f64::from(ship.max_armor),
            max_hull: f64::from(ship.max_hull),
            current_shield: f64::from(ship.current_shield),
            current_armor: f64::from(ship.current_armor),
            current_hull: f64::from(ship.current_hull),
            cap_max: f64::from(ship.cap_max),
            cap_recharge_per_tick: f64::from(ship.cap_recharge_per_tick),
            is_player: ship.is_player,
        })
    }
}

#[godot_api]
impl ShipPresentation {}

#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct InitialStatePresentation {
    #[var]
    ships: Array<Gd<ShipPresentation>>,
}

impl InitialStatePresentation {
    pub(crate) fn wrap(ships: &[ShipStateWire]) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ships: ships.iter().map(ShipPresentation::wrap).collect(),
        })
    }
}

#[godot_api]
impl InitialStatePresentation {}

#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct MotionCorrectionPresentation {
    #[var]
    ship_id: i64,
    #[var]
    position: PackedFloat64Array,
    #[var]
    velocity: PackedFloat64Array,
    #[var]
    tick: i64,
}

impl MotionCorrectionPresentation {
    pub(crate) fn wrap(
        ship_id: u64,
        position: dawn_wire::AbsPosWire,
        velocity: dawn_wire::VelWire,
        tick: u64,
    ) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            ship_id: godot_i64(ship_id),
            position: PackedFloat64Array::from([position.x, position.y, position.z]),
            velocity: PackedFloat64Array::from([velocity.dx, velocity.dy, velocity.dz]),
            tick: godot_i64(tick),
        })
    }
}

#[godot_api]
impl MotionCorrectionPresentation {}

#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct MarketOrder {
    #[var]
    order_id: i64,
    #[var]
    item_id: Gd<ItemIdentity>,
    #[var]
    side: GString,
    #[var]
    price: i64,
    #[var]
    quantity: i64,
    #[var]
    is_own: bool,
}

#[godot_api]
impl MarketOrder {}

#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct MarketSnapshot {
    #[var]
    balance: i64,
    #[var]
    orders: Array<Gd<MarketOrder>>,
    #[var]
    notice: GString,
}

impl MarketSnapshot {
    pub(crate) fn wrap(snapshot: &MarketSnapshotWire) -> Gd<Self> {
        let orders = snapshot
            .orders
            .iter()
            .map(|order| {
                let item_id = ItemId::try_from(order.item_id)
                    .expect("server message validation covers every Market Item identity");
                Gd::from_init_fn(|_base| MarketOrder {
                    order_id: godot_i64(order.order_id),
                    item_id: ItemIdentity::wrap(item_id),
                    side: order.side.as_str().into(),
                    price: godot_i64(order.price),
                    quantity: godot_i64(order.quantity),
                    is_own: order.is_own,
                })
            })
            .collect();
        Gd::from_init_fn(|_base| Self {
            balance: godot_i64(snapshot.balance),
            orders,
            notice: snapshot.notice.as_str().into(),
        })
    }
}

#[godot_api]
impl MarketSnapshot {}

pub(crate) fn godot_i64(value: u64) -> i64 {
    i64::try_from(value).expect("server message validation covers every Godot-facing u64")
}

pub(crate) fn position_components(position: dawn_wire::AbsPosWire) -> PackedFloat64Array {
    PackedFloat64Array::from([position.x, position.y, position.z])
}

pub(crate) fn velocity_components(velocity: dawn_wire::VelWire) -> PackedFloat64Array {
    PackedFloat64Array::from([velocity.dx, velocity.dy, velocity.dz])
}
