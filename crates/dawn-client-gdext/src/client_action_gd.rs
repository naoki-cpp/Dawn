use dawn_client_core::{
    ClientAction as CoreClientAction, ClientActionContext,
    ClientInteraction as CoreClientInteraction, ClientKey, ClientLocalAction, Selection,
};
use godot::prelude::*;

use crate::client_command_gd::{request_result_from_request, ClientCommandResult};
use crate::id_boundary::{
    celestial_body_id_from_godot, jump_gate_id_from_godot, ship_id_from_godot,
    ship_type_id_from_godot, station_id_from_godot,
};

const ACTION_NONE: i64 = 0;
const ACTION_REQUEST: i64 = 1;
const ACTION_LOCAL: i64 = 2;

const LOCAL_NONE: i64 = 0;
const LOCAL_TOGGLE_MODULE: i64 = 1;
const LOCAL_ADJUST_KEEP_AT_RANGE: i64 = 2;
const LOCAL_TOGGLE_INVENTORY: i64 = 3;
const LOCAL_TOGGLE_MARKET: i64 = 4;
const LOCAL_TOGGLE_TACTICAL_OVERLAY: i64 = 5;
const LOCAL_DOUBLE_CLICK_MOVE: i64 = 6;
const LOCAL_SELECTION_CHANGED: i64 = 7;

/// One typed result of client input interpretation.
///
/// Network actions retain the domain `ClientRequest` until this boundary,
/// where the existing command encoder validates and serializes them. Local
/// effects expose only the payload needed by the Godot presentation layer.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ClientAction {
    action: CoreClientAction,
}

impl ClientAction {
    pub(crate) fn from_core(action: CoreClientAction) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { action })
    }
}

#[godot_api]
impl ClientAction {
    #[func]
    fn kind(&self) -> i64 {
        match self.action {
            CoreClientAction::None => ACTION_NONE,
            CoreClientAction::Request(_) => ACTION_REQUEST,
            CoreClientAction::Local(_) => ACTION_LOCAL,
        }
    }

    #[func]
    fn local_kind(&self) -> i64 {
        match self.action {
            CoreClientAction::Local(ClientLocalAction::ToggleModule { .. }) => LOCAL_TOGGLE_MODULE,
            CoreClientAction::Local(ClientLocalAction::AdjustKeepAtRange { .. }) => {
                LOCAL_ADJUST_KEEP_AT_RANGE
            }
            CoreClientAction::Local(ClientLocalAction::ToggleInventoryPanel) => {
                LOCAL_TOGGLE_INVENTORY
            }
            CoreClientAction::Local(ClientLocalAction::ToggleMarketPanel) => LOCAL_TOGGLE_MARKET,
            CoreClientAction::Local(ClientLocalAction::ToggleTacticalOverlay) => {
                LOCAL_TOGGLE_TACTICAL_OVERLAY
            }
            CoreClientAction::Local(ClientLocalAction::DoubleClickMove { .. }) => {
                LOCAL_DOUBLE_CLICK_MOVE
            }
            CoreClientAction::Local(ClientLocalAction::SelectionChanged) => LOCAL_SELECTION_CHANGED,
            CoreClientAction::None | CoreClientAction::Request(_) => LOCAL_NONE,
        }
    }

    #[func]
    fn request_result(&self) -> Gd<ClientCommandResult> {
        match &self.action {
            CoreClientAction::Request(request) => request_result_from_request(request.clone()),
            CoreClientAction::None | CoreClientAction::Local(_) => {
                ClientCommandResult::failure("not_network_action", "client action is not a request")
            }
        }
    }

    #[func]
    fn is_stop_request(&self) -> bool {
        matches!(
            self.action,
            CoreClientAction::Request(dawn_core::ClientRequest::Stop)
        )
    }

    #[func]
    fn target_ship_id(&self) -> i64 {
        match self.action {
            CoreClientAction::Request(dawn_core::ClientRequest::LockOn { target }) => {
                target.raw() as i64
            }
            _ => -1,
        }
    }

    #[func]
    fn module_index(&self) -> i64 {
        match self.action {
            CoreClientAction::Local(ClientLocalAction::ToggleModule { index }) => i64::from(index),
            _ => -1,
        }
    }

    #[func]
    fn delta_km(&self) -> f64 {
        match self.action {
            CoreClientAction::Local(ClientLocalAction::AdjustKeepAtRange { delta_km }) => delta_km,
            _ => 0.0,
        }
    }

    #[func]
    fn screen_position(&self) -> Vector2 {
        match self.action {
            CoreClientAction::Local(ClientLocalAction::DoubleClickMove { screen_x, screen_y }) => {
                Vector2::new(screen_x as f32, screen_y as f32)
            }
            _ => Vector2::ZERO,
        }
    }
}

/// Godot adapter for the engine-independent interaction policy.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct ClientInteraction {
    core: CoreClientInteraction,
}

#[godot_api]
impl ClientInteraction {
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn resolve_key_action(
        &self,
        key_code: i64,
        player_ship_id: i64,
        nearby_gate_id: i64,
        nearby_station_id: i64,
        docked_station_id: i64,
        keep_at_range_m: f64,
        buildable_ship_type_id: i64,
    ) -> Gd<ClientAction> {
        let Some(key) = ClientKey::from_code(key_code) else {
            return ClientAction::from_core(CoreClientAction::None);
        };
        let Some(buildable_ship_type_id) = ship_type_id_from_godot(buildable_ship_type_id) else {
            return ClientAction::from_core(CoreClientAction::None);
        };
        ClientAction::from_core(self.core.resolve_key_action(
            key,
            ClientActionContext {
                player_ship_id: ship_id_from_godot(player_ship_id),
                nearby_gate_id: jump_gate_id_from_godot(nearby_gate_id),
                nearby_station_id: station_id_from_godot(nearby_station_id),
                docked_station_id: station_id_from_godot(docked_station_id),
                keep_at_range_m,
                buildable_ship_type_id,
            },
        ))
    }

    #[func]
    #[allow(clippy::too_many_arguments)]
    fn primary_click(
        &mut self,
        screen_pos: Vector2,
        now_sec: f64,
        camera_dragging: bool,
        player_ship_id: i64,
        hit_ship_id: i64,
        hit_gate_id: i64,
        hit_body_id: i64,
        hit_station_id: i64,
    ) -> Gd<ClientAction> {
        let hit = if let Some(id) = ship_id_from_godot(hit_ship_id) {
            Selection::Ship(id)
        } else if let Some(id) = jump_gate_id_from_godot(hit_gate_id) {
            Selection::Gate(id)
        } else if let Some(id) = station_id_from_godot(hit_station_id) {
            Selection::Station(id)
        } else if let Some(id) = celestial_body_id_from_godot(hit_body_id) {
            Selection::Body(id)
        } else {
            Selection::None
        };
        ClientAction::from_core(self.core.primary_click(
            f64::from(screen_pos.x),
            f64::from(screen_pos.y),
            now_sec,
            camera_dragging,
            ship_id_from_godot(player_ship_id),
            hit,
        ))
    }

    #[func]
    fn lock_click(&self, player_ship_id: i64, hit_ship_id: i64) -> Gd<ClientAction> {
        ClientAction::from_core(self.core.lock_click(
            ship_id_from_godot(player_ship_id),
            ship_id_from_godot(hit_ship_id),
        ))
    }

    #[func]
    fn selected_target_id(&self) -> i64 {
        self.core
            .selection()
            .ship_id()
            .map_or(-1, |id| id.raw() as i64)
    }

    #[func]
    fn selected_gate_id(&self) -> i64 {
        self.core
            .selection()
            .gate_id()
            .map_or(-1, |id| i64::from(id.0))
    }

    #[func]
    fn selected_body_id(&self) -> i64 {
        self.core
            .selection()
            .body_id()
            .map_or(-1, |id| i64::from(id.0))
    }

    #[func]
    fn selected_station_id(&self) -> i64 {
        self.core
            .selection()
            .station_id()
            .map_or(-1, |id| i64::from(id.0))
    }

    #[func]
    fn clear_selection(&mut self) {
        self.core.clear_selection();
    }

    #[func]
    fn clear_navigation_selection(&mut self) {
        self.core.clear_navigation_selection();
    }

    #[func]
    fn clear_target_if_matches(&mut self, raw_ship_id: i64) {
        if let Some(ship_id) = ship_id_from_godot(raw_ship_id) {
            self.core.clear_target_if_matches(ship_id);
        }
    }
}
