//! Thin GDExtension adapter for the core HUD read model.

use dawn_client_core::{HudReadModel as CoreHudReadModel, HudSceneFacts as CoreHudSceneFacts};
use godot::prelude::*;

use crate::loadout_gd::PlayerLoadout;
use crate::module_row_gd::ModuleRow;
use crate::session_record_gd::ShipHealth;
use crate::world_session_gd::WorldSession;

#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct HudSceneFacts {
    #[var]
    connected: bool,
    #[var]
    has_player_speed: bool,
    #[var]
    player_speed_units: f64,
    #[var]
    target_known: bool,
    #[var]
    has_target_distance: bool,
    #[var]
    target_distance_units: f64,
    #[var]
    #[init(val = -1)]
    nearby_gate_id: i64,
    #[var]
    nearby_station_ids: Array<i64>,
    #[var]
    jump_notice: GString,
    #[var]
    #[init(val = -1)]
    selected_gate_id: i64,
    #[var]
    #[init(val = -1)]
    selected_body_id: i64,
    #[var]
    #[init(val = -1)]
    selected_station_id: i64,
    #[var]
    #[init(val = -1)]
    selected_target_id: i64,
    #[var]
    has_selected_gate_distance: bool,
    #[var]
    selected_gate_distance_units: f64,
    #[var]
    #[init(val = 10.0)]
    keep_at_range_km: f64,
}

impl HudSceneFacts {
    fn core(&self) -> CoreHudSceneFacts {
        CoreHudSceneFacts {
            connected: self.connected,
            player_speed_units: self.has_player_speed.then_some(self.player_speed_units),
            target_known: self.target_known,
            target_distance_units: self
                .has_target_distance
                .then_some(self.target_distance_units),
            nearby_gate_id: self.nearby_gate_id,
            nearby_station_ids: self.nearby_station_ids.iter_shared().collect(),
            jump_notice: self.jump_notice.to_string(),
            selected_gate_id: self.selected_gate_id,
            selected_body_id: self.selected_body_id,
            selected_station_id: self.selected_station_id,
            selected_target_id: self.selected_target_id,
            selected_gate_distance_units: self
                .has_selected_gate_distance
                .then_some(self.selected_gate_distance_units),
            keep_at_range_km: self.keep_at_range_km,
        }
    }
}

#[godot_api]
impl HudSceneFacts {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct HudStatusPanel {
    #[var]
    connected: bool,
    #[var]
    ship_type_name: GString,
    #[var]
    system_name: GString,
    #[var]
    speed_text: GString,
}

#[godot_api]
impl HudStatusPanel {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct HudShipStatusPanel {
    #[var]
    player_ship_id: i64,
    #[var]
    shield: f64,
    #[var]
    max_shield: f64,
    #[var]
    armor: f64,
    #[var]
    max_armor: f64,
    #[var]
    hull: f64,
    #[var]
    max_hull: f64,
    #[var]
    cap_current: f64,
    #[var]
    cap_max: f64,
}

#[godot_api]
impl HudShipStatusPanel {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct HudTargetPanel {
    #[var]
    lock_target_id: i64,
    #[var]
    target_known: bool,
    #[var]
    distance_text: GString,
    #[var]
    target_hp: Variant,
}

#[godot_api]
impl HudTargetPanel {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct HudStatsPanel {
    #[var]
    text: GString,
}

#[godot_api]
impl HudStatsPanel {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct HudChangeSet {
    #[var]
    status_changed: bool,
    #[var]
    ship_status_changed: bool,
    #[var]
    target_changed: bool,
    #[var]
    modules_changed: bool,
    #[var]
    module_structure_changed: bool,
    #[var]
    stats_changed: bool,
}

#[godot_api]
impl HudChangeSet {}

#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct HudSnapshot {
    #[var]
    status: Gd<HudStatusPanel>,
    #[var]
    ship_status: Gd<HudShipStatusPanel>,
    #[var]
    target: Gd<HudTargetPanel>,
    #[var]
    modules: Array<Gd<ModuleRow>>,
    #[var]
    stats: Gd<HudStatsPanel>,
    #[var]
    changes: Gd<HudChangeSet>,
}

impl HudSnapshot {
    pub(crate) fn wrap(frame: &dawn_client_core::HudFrame) -> Gd<Self> {
        let snapshot = &frame.snapshot;
        let health = snapshot.ship_status.health;
        let target_hp = snapshot
            .target
            .health
            .map(|health| ShipHealth::wrap(snapshot.target.lock_target_id, health).to_variant())
            .unwrap_or_else(Variant::nil);
        Gd::from_init_fn(|_base| Self {
            status: Gd::from_init_fn(|_base| HudStatusPanel {
                connected: snapshot.status.connected,
                ship_type_name: snapshot.status.ship_type_name.as_str().into(),
                system_name: snapshot.status.system_name.as_str().into(),
                speed_text: snapshot.status.speed_text.as_str().into(),
            }),
            ship_status: Gd::from_init_fn(|_base| HudShipStatusPanel {
                player_ship_id: snapshot.ship_status.player_ship_id,
                shield: health.shield,
                max_shield: health.max_shield,
                armor: health.armor,
                max_armor: health.max_armor,
                hull: health.hull,
                max_hull: health.max_hull,
                cap_current: snapshot.ship_status.cap_current,
                cap_max: snapshot.ship_status.cap_max,
            }),
            target: Gd::from_init_fn(|_base| HudTargetPanel {
                lock_target_id: snapshot.target.lock_target_id,
                target_known: snapshot.target.target_known,
                distance_text: snapshot.target.distance_text.as_str().into(),
                target_hp,
            }),
            modules: snapshot
                .modules
                .iter()
                .cloned()
                .map(ModuleRow::wrap)
                .collect(),
            stats: Gd::from_init_fn(|_base| HudStatsPanel {
                text: snapshot.stats.text.as_str().into(),
            }),
            changes: Gd::from_init_fn(|_base| HudChangeSet {
                status_changed: frame.changes.status_changed,
                ship_status_changed: frame.changes.ship_status_changed,
                target_changed: frame.changes.target_changed,
                modules_changed: frame.changes.modules_changed,
                module_structure_changed: frame.changes.module_structure_changed,
                stats_changed: frame.changes.stats_changed,
            }),
        })
    }
}

#[godot_api]
impl HudSnapshot {}

#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct HudReadModel {
    model: CoreHudReadModel,
}

#[godot_api]
impl HudReadModel {
    #[func]
    fn reset(&mut self) {
        self.model.reset();
    }

    #[func]
    fn project(
        &mut self,
        session: Gd<WorldSession>,
        loadout: Gd<PlayerLoadout>,
        facts: Gd<HudSceneFacts>,
    ) -> Gd<HudSnapshot> {
        let session = session.bind();
        let loadout = loadout.bind();
        let facts = facts.bind();
        let frame = self
            .model
            .project(session.core_ref(), loadout.core_ref(), &facts.core());
        HudSnapshot::wrap(&frame)
    }
}
