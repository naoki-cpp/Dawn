use godot::prelude::*;

/// The finite set of client-side interaction outcomes understood by
/// `main.gd`. Keeping this as a Rust enum prevents the Godot boundary from
/// smuggling action names and payload keys through a Dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientIntentKind {
    None,
    ToggleModule,
    Stop,
    Jump,
    ApproachGate,
    ApproachShip,
    WarpToGate,
    WarpToBody,
    WarpToStation,
    OrbitGate,
    OrbitShip,
    KeepAtRangeGate,
    KeepAtRangeShip,
    AdjustKeepAtRange,
    ToggleInventoryPanel,
    ToggleMarketPanel,
    Dock,
    Undock,
    BuildPackagedShip,
    DisassembleShip,
    Disembark,
    ToggleTacticalOverlay,
    DoubleClickMove,
    SelectionChanged,
    LockOn,
}

/// Typed result of client input interpretation.
///
/// The object exposes semantic predicates and accessors instead of a string
/// tag plus a collection of optional Dictionary fields. Accessors are only
/// meaningful for the corresponding predicate, which keeps each dispatch arm
/// explicit without manufacturing sentinel values for unrelated fields.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ClientIntent {
    kind: ClientIntentKind,
    id: Option<i64>,
    value: Option<f64>,
}

impl ClientIntent {
    fn new(kind: ClientIntentKind, id: Option<i64>, value: Option<f64>) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { kind, id, value })
    }

    fn simple(kind: ClientIntentKind) -> Gd<Self> {
        Self::new(kind, None, None)
    }

    fn with_id(kind: ClientIntentKind, id: i64) -> Gd<Self> {
        if id < 0 {
            return Self::simple(ClientIntentKind::None);
        }
        Self::new(kind, Some(id), None)
    }
}

#[godot_api]
impl ClientIntent {
    #[func]
    fn none() -> Gd<Self> {
        Self::simple(ClientIntentKind::None)
    }

    #[func]
    fn toggle_module(module_index: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::ToggleModule, module_index)
    }

    #[func]
    fn stop() -> Gd<Self> {
        Self::simple(ClientIntentKind::Stop)
    }

    #[func]
    fn jump(gate_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::Jump, gate_id)
    }

    #[func]
    fn approach_gate(gate_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::ApproachGate, gate_id)
    }

    #[func]
    fn approach_ship(ship_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::ApproachShip, ship_id)
    }

    #[func]
    fn warp_to_gate(gate_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::WarpToGate, gate_id)
    }

    #[func]
    fn warp_to_body(body_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::WarpToBody, body_id)
    }

    #[func]
    fn warp_to_station(station_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::WarpToStation, station_id)
    }

    #[func]
    fn orbit_gate(gate_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::OrbitGate, gate_id)
    }

    #[func]
    fn orbit_ship(ship_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::OrbitShip, ship_id)
    }

    #[func]
    fn keep_at_range_gate(gate_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::KeepAtRangeGate, gate_id)
    }

    #[func]
    fn keep_at_range_ship(ship_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::KeepAtRangeShip, ship_id)
    }

    #[func]
    fn adjust_keep_at_range(delta_km: f64) -> Gd<Self> {
        if !delta_km.is_finite() {
            return Self::none();
        }
        Self::new(ClientIntentKind::AdjustKeepAtRange, None, Some(delta_km))
    }

    #[func]
    fn toggle_inventory_panel() -> Gd<Self> {
        Self::simple(ClientIntentKind::ToggleInventoryPanel)
    }

    #[func]
    fn toggle_market_panel() -> Gd<Self> {
        Self::simple(ClientIntentKind::ToggleMarketPanel)
    }

    #[func]
    fn dock(station_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::Dock, station_id)
    }

    #[func]
    fn undock() -> Gd<Self> {
        Self::simple(ClientIntentKind::Undock)
    }

    #[func]
    fn build_packaged_ship(station_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::BuildPackagedShip, station_id)
    }

    #[func]
    fn disassemble_ship(station_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::DisassembleShip, station_id)
    }

    #[func]
    fn disembark() -> Gd<Self> {
        Self::simple(ClientIntentKind::Disembark)
    }

    #[func]
    fn toggle_tactical_overlay() -> Gd<Self> {
        Self::simple(ClientIntentKind::ToggleTacticalOverlay)
    }

    #[func]
    fn double_click_move() -> Gd<Self> {
        Self::simple(ClientIntentKind::DoubleClickMove)
    }

    #[func]
    fn selection_changed() -> Gd<Self> {
        Self::simple(ClientIntentKind::SelectionChanged)
    }

    #[func]
    fn lock_on(ship_id: i64) -> Gd<Self> {
        Self::with_id(ClientIntentKind::LockOn, ship_id)
    }

    #[func]
    fn is_none(&self) -> bool {
        self.kind == ClientIntentKind::None
    }

    #[func]
    fn is_toggle_module(&self) -> bool {
        self.kind == ClientIntentKind::ToggleModule
    }

    #[func]
    fn is_stop(&self) -> bool {
        self.kind == ClientIntentKind::Stop
    }

    #[func]
    fn is_jump(&self) -> bool {
        self.kind == ClientIntentKind::Jump
    }

    #[func]
    fn is_approach_gate(&self) -> bool {
        self.kind == ClientIntentKind::ApproachGate
    }

    #[func]
    fn is_approach_ship(&self) -> bool {
        self.kind == ClientIntentKind::ApproachShip
    }

    #[func]
    fn is_warp_to_gate(&self) -> bool {
        self.kind == ClientIntentKind::WarpToGate
    }

    #[func]
    fn is_warp_to_body(&self) -> bool {
        self.kind == ClientIntentKind::WarpToBody
    }

    #[func]
    fn is_warp_to_station(&self) -> bool {
        self.kind == ClientIntentKind::WarpToStation
    }

    #[func]
    fn is_orbit_gate(&self) -> bool {
        self.kind == ClientIntentKind::OrbitGate
    }

    #[func]
    fn is_orbit_ship(&self) -> bool {
        self.kind == ClientIntentKind::OrbitShip
    }

    #[func]
    fn is_keep_at_range_gate(&self) -> bool {
        self.kind == ClientIntentKind::KeepAtRangeGate
    }

    #[func]
    fn is_keep_at_range_ship(&self) -> bool {
        self.kind == ClientIntentKind::KeepAtRangeShip
    }

    #[func]
    fn is_adjust_keep_at_range(&self) -> bool {
        self.kind == ClientIntentKind::AdjustKeepAtRange
    }

    #[func]
    fn is_toggle_inventory_panel(&self) -> bool {
        self.kind == ClientIntentKind::ToggleInventoryPanel
    }

    #[func]
    fn is_toggle_market_panel(&self) -> bool {
        self.kind == ClientIntentKind::ToggleMarketPanel
    }

    #[func]
    fn is_dock(&self) -> bool {
        self.kind == ClientIntentKind::Dock
    }

    #[func]
    fn is_undock(&self) -> bool {
        self.kind == ClientIntentKind::Undock
    }

    #[func]
    fn is_build_packaged_ship(&self) -> bool {
        self.kind == ClientIntentKind::BuildPackagedShip
    }

    #[func]
    fn is_disassemble_ship(&self) -> bool {
        self.kind == ClientIntentKind::DisassembleShip
    }

    #[func]
    fn is_disembark(&self) -> bool {
        self.kind == ClientIntentKind::Disembark
    }

    #[func]
    fn is_toggle_tactical_overlay(&self) -> bool {
        self.kind == ClientIntentKind::ToggleTacticalOverlay
    }

    #[func]
    fn is_double_click_move(&self) -> bool {
        self.kind == ClientIntentKind::DoubleClickMove
    }

    #[func]
    fn is_selection_changed(&self) -> bool {
        self.kind == ClientIntentKind::SelectionChanged
    }

    #[func]
    fn is_lock_on(&self) -> bool {
        self.kind == ClientIntentKind::LockOn
    }

    #[func]
    fn module_index(&self) -> i64 {
        self.id.unwrap_or_default()
    }

    #[func]
    fn gate_id(&self) -> i64 {
        self.id.unwrap_or_default()
    }

    #[func]
    fn ship_id(&self) -> i64 {
        self.id.unwrap_or_default()
    }

    #[func]
    fn body_id(&self) -> i64 {
        self.id.unwrap_or_default()
    }

    #[func]
    fn station_id(&self) -> i64 {
        self.id.unwrap_or_default()
    }

    #[func]
    fn delta_km(&self) -> f64 {
        self.value.unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionKind {
    None,
    Ship,
    Gate,
    Body,
    Station,
}

/// Mutually exclusive world selection state shared by keyboard and mouse
/// interaction. A selection can contain exactly one domain identity.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ClientSelection {
    kind: SelectionKind,
    id: Option<i64>,
}

impl ClientSelection {
    fn new(kind: SelectionKind, id: Option<i64>) -> Gd<Self> {
        if id.is_some_and(|id| id < 0) {
            return Self::new(SelectionKind::None, None);
        }
        Gd::from_init_fn(|_base| Self { kind, id })
    }
}

#[godot_api]
impl ClientSelection {
    #[func]
    fn none() -> Gd<Self> {
        Self::new(SelectionKind::None, None)
    }

    #[func]
    fn ship(ship_id: i64) -> Gd<Self> {
        Self::new(SelectionKind::Ship, Some(ship_id))
    }

    #[func]
    fn gate(gate_id: i64) -> Gd<Self> {
        Self::new(SelectionKind::Gate, Some(gate_id))
    }

    #[func]
    fn body(body_id: i64) -> Gd<Self> {
        Self::new(SelectionKind::Body, Some(body_id))
    }

    #[func]
    fn station(station_id: i64) -> Gd<Self> {
        Self::new(SelectionKind::Station, Some(station_id))
    }

    #[func]
    fn is_none(&self) -> bool {
        self.kind == SelectionKind::None
    }

    #[func]
    fn is_ship(&self) -> bool {
        self.kind == SelectionKind::Ship
    }

    #[func]
    fn is_gate(&self) -> bool {
        self.kind == SelectionKind::Gate
    }

    #[func]
    fn is_body(&self) -> bool {
        self.kind == SelectionKind::Body
    }

    #[func]
    fn is_station(&self) -> bool {
        self.kind == SelectionKind::Station
    }

    #[func]
    fn id(&self) -> i64 {
        self.id.unwrap_or_default()
    }
}
