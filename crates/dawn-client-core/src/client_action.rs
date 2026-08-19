use dawn_core::{
    ApproachTarget, CelestialBodyId, ClientRequest, JumpGateId, ShipId, ShipTypeId, StationId,
    WarpTarget,
};

const DOUBLE_CLICK_SEC: f64 = 0.4;
const DOUBLE_CLICK_PX_SQUARED: f64 = 100.0;

/// Keyboard meanings normalized by the engine adapter.
///
/// The numeric values are an adapter protocol, not Godot key codes. This
/// keeps input policy testable without importing an engine API into
/// `dawn-client-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKey {
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    Stop,
    Jump,
    Approach,
    Warp,
    Orbit,
    KeepAtRange,
    DecreaseKeepAtRange,
    IncreaseKeepAtRange,
    Inventory,
    Market,
    Dock,
    Undock,
    BuildPackagedShip,
    DisassembleShip,
    Disembark,
    TacticalOverlay,
}

impl ClientKey {
    /// Converts the stable adapter key code into a semantic client key.
    #[must_use]
    pub const fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            1 => Self::F1,
            2 => Self::F2,
            3 => Self::F3,
            4 => Self::F4,
            5 => Self::F5,
            6 => Self::F6,
            7 => Self::F7,
            8 => Self::F8,
            9 => Self::Stop,
            10 => Self::Jump,
            11 => Self::Approach,
            12 => Self::Warp,
            13 => Self::Orbit,
            14 => Self::KeepAtRange,
            15 => Self::DecreaseKeepAtRange,
            16 => Self::IncreaseKeepAtRange,
            17 => Self::Inventory,
            18 => Self::Market,
            19 => Self::Dock,
            20 => Self::Undock,
            21 => Self::BuildPackagedShip,
            22 => Self::DisassembleShip,
            23 => Self::Disembark,
            24 => Self::TacticalOverlay,
            _ => return None,
        })
    }

    #[must_use]
    const fn module_index(self) -> Option<u8> {
        Some(match self {
            Self::F1 => 0,
            Self::F2 => 1,
            Self::F3 => 2,
            Self::F4 => 3,
            Self::F5 => 4,
            Self::F6 => 5,
            Self::F7 => 6,
            Self::F8 => 7,
            _ => return None,
        })
    }
}

/// Mutually exclusive world selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    None,
    Ship(ShipId),
    Gate(JumpGateId),
    Body(CelestialBodyId),
    Station(StationId),
}

impl Selection {
    #[must_use]
    pub const fn is_navigation_target(self) -> bool {
        matches!(self, Self::Gate(_) | Self::Body(_) | Self::Station(_))
    }

    #[must_use]
    pub const fn ship_id(self) -> Option<ShipId> {
        match self {
            Self::Ship(id) => Some(id),
            _ => None,
        }
    }

    #[must_use]
    pub const fn gate_id(self) -> Option<JumpGateId> {
        match self {
            Self::Gate(id) => Some(id),
            _ => None,
        }
    }

    #[must_use]
    pub const fn body_id(self) -> Option<CelestialBodyId> {
        match self {
            Self::Body(id) => Some(id),
            _ => None,
        }
    }

    #[must_use]
    pub const fn station_id(self) -> Option<StationId> {
        match self {
            Self::Station(id) => Some(id),
            _ => None,
        }
    }

    #[must_use]
    const fn approach_target(self) -> Option<ApproachTarget> {
        match self {
            Self::Ship(id) => Some(ApproachTarget::Ship(id)),
            Self::Gate(id) => Some(ApproachTarget::Gate(id)),
            _ => None,
        }
    }

    #[must_use]
    const fn warp_target(self) -> Option<WarpTarget> {
        match self {
            Self::Gate(id) => Some(WarpTarget::Gate(id)),
            Self::Body(id) => Some(WarpTarget::Body(id)),
            Self::Station(id) => Some(WarpTarget::Station(id)),
            Self::None | Self::Ship(_) => None,
        }
    }
}

/// Effects that stay in the engine-owned presentation layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientLocalAction {
    ToggleModule { index: u8 },
    AdjustKeepAtRange { delta_km: f64 },
    ToggleInventoryPanel,
    ToggleMarketPanel,
    ToggleTacticalOverlay,
    DoubleClickMove { screen_x: f64, screen_y: f64 },
    SelectionChanged,
}

/// The one result crossing the input-to-command seam.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientAction {
    None,
    Request(ClientRequest),
    Local(ClientLocalAction),
}

/// Session values needed to interpret one normalized keypress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientActionContext {
    pub player_ship_id: Option<ShipId>,
    pub nearby_gate_id: Option<JumpGateId>,
    pub nearby_station_id: Option<StationId>,
    pub docked_station_id: Option<StationId>,
    pub keep_at_range_m: f64,
    pub buildable_ship_type_id: ShipTypeId,
}

impl ClientAction {
    #[must_use]
    pub const fn is_request(&self) -> bool {
        matches!(self, Self::Request(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LastClick {
    x: f64,
    y: f64,
    time_sec: f64,
}

/// Engine-independent client interaction policy.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ClientInteraction {
    selection: Selection,
    last_click: Option<LastClick>,
}

impl ClientInteraction {
    #[must_use]
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// Resolve a normalized key and current session context into one action.
    #[must_use]
    pub fn resolve_key_action(&self, key: ClientKey, context: ClientActionContext) -> ClientAction {
        if let Some(index) = key.module_index() {
            return ClientAction::Local(ClientLocalAction::ToggleModule { index });
        }

        let Some(player_ship_id) = context.player_ship_id else {
            return match key {
                ClientKey::Inventory => {
                    ClientAction::Local(ClientLocalAction::ToggleInventoryPanel)
                }
                ClientKey::Market if context.docked_station_id.is_some() => {
                    ClientAction::Local(ClientLocalAction::ToggleMarketPanel)
                }
                ClientKey::TacticalOverlay => {
                    ClientAction::Local(ClientLocalAction::ToggleTacticalOverlay)
                }
                _ => ClientAction::None,
            };
        };

        match key {
            ClientKey::Stop => ClientAction::Request(ClientRequest::Stop),
            ClientKey::Jump => self
                .selection
                .gate_id()
                .or(context.nearby_gate_id)
                .map(|gate| ClientAction::Request(ClientRequest::Jump { gate }))
                .unwrap_or(ClientAction::None),
            ClientKey::Approach => self
                .selection
                .approach_target()
                .map(|target| ClientAction::Request(ClientRequest::Approach { target }))
                .unwrap_or(ClientAction::None),
            ClientKey::Warp => self
                .selection
                .warp_target()
                .map(|target| ClientAction::Request(ClientRequest::Warp { target }))
                .unwrap_or(ClientAction::None),
            ClientKey::Orbit => self
                .selection
                .approach_target()
                .map(|target| {
                    ClientAction::Request(ClientRequest::Orbit {
                        target,
                        radius: None,
                    })
                })
                .unwrap_or(ClientAction::None),
            ClientKey::KeepAtRange => self
                .selection
                .approach_target()
                .map(|target| {
                    ClientAction::Request(ClientRequest::KeepAtRange {
                        target,
                        range: finite_positive(context.keep_at_range_m),
                    })
                })
                .unwrap_or(ClientAction::None),
            ClientKey::DecreaseKeepAtRange => {
                ClientAction::Local(ClientLocalAction::AdjustKeepAtRange { delta_km: -1.0 })
            }
            ClientKey::IncreaseKeepAtRange => {
                ClientAction::Local(ClientLocalAction::AdjustKeepAtRange { delta_km: 1.0 })
            }
            ClientKey::Inventory => ClientAction::Local(ClientLocalAction::ToggleInventoryPanel),
            ClientKey::Market if context.docked_station_id.is_some() => {
                ClientAction::Local(ClientLocalAction::ToggleMarketPanel)
            }
            ClientKey::Dock if context.docked_station_id.is_none() => context
                .nearby_station_id
                .map(|station| ClientAction::Request(ClientRequest::Dock { station }))
                .unwrap_or(ClientAction::None),
            ClientKey::Undock if context.docked_station_id.is_some() => {
                ClientAction::Request(ClientRequest::Undock)
            }
            ClientKey::BuildPackagedShip => context
                .docked_station_id
                .map(|station| {
                    ClientAction::Request(ClientRequest::BuildPackagedShip {
                        ship: player_ship_id,
                        station,
                        ship_type: context.buildable_ship_type_id,
                    })
                })
                .unwrap_or(ClientAction::None),
            ClientKey::DisassembleShip => context
                .docked_station_id
                .map(|station| {
                    ClientAction::Request(ClientRequest::DisassembleShip {
                        ship: player_ship_id,
                        station,
                    })
                })
                .unwrap_or(ClientAction::None),
            ClientKey::Disembark if context.docked_station_id.is_some() => {
                ClientAction::Request(ClientRequest::Disembark)
            }
            ClientKey::TacticalOverlay => {
                ClientAction::Local(ClientLocalAction::ToggleTacticalOverlay)
            }
            ClientKey::F1
            | ClientKey::F2
            | ClientKey::F3
            | ClientKey::F4
            | ClientKey::F5
            | ClientKey::F6
            | ClientKey::F7
            | ClientKey::F8
            | ClientKey::Market
            | ClientKey::Dock
            | ClientKey::Undock
            | ClientKey::Disembark => ClientAction::None,
        }
    }

    /// Resolve a world click. Hit testing remains in the engine adapter.
    #[must_use]
    pub fn primary_click(
        &mut self,
        screen_x: f64,
        screen_y: f64,
        now_sec: f64,
        camera_dragging: bool,
        player_ship_id: Option<ShipId>,
        hit: Selection,
    ) -> ClientAction {
        if player_ship_id.is_none() || !now_sec.is_finite() {
            return ClientAction::None;
        }

        let double_click = self.last_click.is_some_and(|previous| {
            now_sec - previous.time_sec < DOUBLE_CLICK_SEC
                && squared_distance(screen_x, screen_y, previous.x, previous.y)
                    < DOUBLE_CLICK_PX_SQUARED
        });
        if double_click {
            self.last_click = None;
            return if camera_dragging {
                ClientAction::None
            } else {
                ClientAction::Local(ClientLocalAction::DoubleClickMove { screen_x, screen_y })
            };
        }

        self.last_click = Some(LastClick {
            x: screen_x,
            y: screen_y,
            time_sec: now_sec,
        });
        if hit == Selection::None {
            ClientAction::None
        } else {
            self.selection = hit;
            ClientAction::Local(ClientLocalAction::SelectionChanged)
        }
    }

    #[must_use]
    pub fn lock_click(
        &self,
        player_ship_id: Option<ShipId>,
        hit_ship_id: Option<ShipId>,
    ) -> ClientAction {
        match (player_ship_id, hit_ship_id) {
            (Some(_), Some(target)) => ClientAction::Request(ClientRequest::LockOn { target }),
            _ => ClientAction::None,
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection = Selection::None;
    }

    pub fn clear_navigation_selection(&mut self) {
        if self.selection.is_navigation_target() {
            self.clear_selection();
        }
    }

    pub fn clear_target_if_matches(&mut self, ship_id: ShipId) {
        if self.selection.ship_id() == Some(ship_id) {
            self.clear_selection();
        }
    }
}

fn finite_positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn squared_distance(x: f64, y: f64, other_x: f64, other_y: f64) -> f64 {
    let dx = x - other_x;
    let dy = y - other_y;
    dx.mul_add(dx, dy * dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::EntityId;

    fn ship(raw: u64) -> ShipId {
        ShipId(EntityId::from_raw(raw))
    }

    fn gate(raw: u32) -> JumpGateId {
        JumpGateId(raw)
    }

    fn station(raw: u32) -> StationId {
        StationId(raw)
    }

    fn body(raw: u32) -> CelestialBodyId {
        CelestialBodyId(raw)
    }

    fn ship_type(raw: u32) -> ShipTypeId {
        ShipTypeId(raw)
    }

    fn context(
        player_ship_id: Option<ShipId>,
        nearby_gate_id: Option<JumpGateId>,
        nearby_station_id: Option<StationId>,
        docked_station_id: Option<StationId>,
        buildable_ship_type_id: ShipTypeId,
    ) -> ClientActionContext {
        ClientActionContext {
            player_ship_id,
            nearby_gate_id,
            nearby_station_id,
            docked_station_id,
            keep_at_range_m: 10_000.0,
            buildable_ship_type_id,
        }
    }

    #[test]
    fn normalized_key_codes_are_the_only_engine_boundary() {
        assert_eq!(ClientKey::from_code(1), Some(ClientKey::F1));
        assert_eq!(ClientKey::from_code(24), Some(ClientKey::TacticalOverlay));
        assert_eq!(ClientKey::from_code(99), None);
    }

    #[test]
    fn f_keys_remain_available_without_a_player_ship() {
        let interaction = ClientInteraction::default();
        assert_eq!(
            interaction
                .resolve_key_action(ClientKey::F3, context(None, None, None, None, ship_type(1)),),
            ClientAction::Local(ClientLocalAction::ToggleModule { index: 2 })
        );
    }

    #[test]
    fn navigation_prefers_selection_and_preserves_target_type() {
        let interaction = ClientInteraction {
            selection: Selection::Gate(gate(5)),
            ..Default::default()
        };
        assert_eq!(
            interaction.resolve_key_action(
                ClientKey::Jump,
                context(Some(ship(1)), Some(gate(9)), None, None, ship_type(1),),
            ),
            ClientAction::Request(ClientRequest::Jump { gate: gate(5) })
        );
        assert_eq!(
            interaction.resolve_key_action(
                ClientKey::Approach,
                context(Some(ship(1)), None, None, None, ship_type(1)),
            ),
            ClientAction::Request(ClientRequest::Approach {
                target: ApproachTarget::Gate(gate(5)),
            })
        );
    }

    #[test]
    fn warp_supports_all_static_navigation_targets() {
        for selection in [
            Selection::Gate(gate(5)),
            Selection::Body(body(3)),
            Selection::Station(station(4)),
        ] {
            let interaction = ClientInteraction {
                selection,
                ..Default::default()
            };
            assert!(matches!(
                interaction.resolve_key_action(
                    ClientKey::Warp,
                    context(Some(ship(1)), None, None, None, ship_type(1)),
                ),
                ClientAction::Request(ClientRequest::Warp { .. })
            ));
        }
    }

    #[test]
    fn docked_actions_use_the_active_ship_and_station_context() {
        let interaction = ClientInteraction::default();
        assert_eq!(
            interaction.resolve_key_action(
                ClientKey::BuildPackagedShip,
                context(Some(ship(7)), None, None, Some(station(3)), ship_type(11),),
            ),
            ClientAction::Request(ClientRequest::BuildPackagedShip {
                ship: ship(7),
                station: station(3),
                ship_type: ship_type(11),
            })
        );
    }

    #[test]
    fn a_shipless_docked_player_can_still_open_the_market() {
        let interaction = ClientInteraction::default();
        assert_eq!(
            interaction.resolve_key_action(
                ClientKey::Market,
                context(None, None, None, Some(station(3)), ship_type(1)),
            ),
            ClientAction::Local(ClientLocalAction::ToggleMarketPanel)
        );
    }

    #[test]
    fn primary_click_owns_selection_and_double_click_timing() {
        let mut interaction = ClientInteraction::default();
        assert_eq!(
            interaction.primary_click(
                100.0,
                50.0,
                1.0,
                false,
                Some(ship(7)),
                Selection::Ship(ship(42))
            ),
            ClientAction::Local(ClientLocalAction::SelectionChanged)
        );
        assert_eq!(interaction.selection(), Selection::Ship(ship(42)));
        assert_eq!(
            interaction.primary_click(100.0, 50.0, 1.2, false, Some(ship(7)), Selection::None),
            ClientAction::Local(ClientLocalAction::DoubleClickMove {
                screen_x: 100.0,
                screen_y: 50.0,
            })
        );
    }

    #[test]
    fn lock_click_becomes_a_typed_request() {
        let interaction = ClientInteraction::default();
        assert_eq!(
            interaction.lock_click(Some(ship(7)), Some(ship(99))),
            ClientAction::Request(ClientRequest::LockOn { target: ship(99) })
        );
        assert_eq!(
            interaction.lock_click(None, Some(ship(99))),
            ClientAction::None
        );
    }
}
