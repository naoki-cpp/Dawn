//! Engine-independent HUD read model.
//!
//! `HudReadModel` is the single policy boundary between client state and the
//! Godot HUD. It owns display formatting, contextual HUD text, panel shape,
//! and value-based change decisions. The GDExtension crate only converts the
//! resulting values into Godot objects; it does not reconstruct a frame or
//! compare mutable Godot objects.

use crate::{ClientRules, HealthState, ModuleRow, PlayerLoadoutMsg, WorldSessionState};

const AU_METERS: f64 = 1.495_978_707e11;

#[derive(Debug, Clone, PartialEq)]
pub struct HudSceneFacts {
    pub connected: bool,
    pub player_speed_units: Option<f64>,
    pub target_known: bool,
    pub target_distance_units: Option<f64>,
    pub nearby_gate_id: i64,
    pub nearby_station_ids: Vec<i64>,
    pub jump_notice: String,
    pub selected_gate_id: i64,
    pub selected_body_id: i64,
    pub selected_station_id: i64,
    pub selected_target_id: i64,
    pub selected_gate_distance_units: Option<f64>,
    pub keep_at_range_km: f64,
}

impl Default for HudSceneFacts {
    fn default() -> Self {
        Self {
            connected: false,
            player_speed_units: None,
            target_known: false,
            target_distance_units: None,
            nearby_gate_id: -1,
            nearby_station_ids: Vec::new(),
            jump_notice: String::new(),
            selected_gate_id: -1,
            selected_body_id: -1,
            selected_station_id: -1,
            selected_target_id: -1,
            selected_gate_distance_units: None,
            keep_at_range_km: 10.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudStatusPanel {
    pub connected: bool,
    pub ship_type_name: String,
    pub system_name: String,
    pub speed_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HudShipStatusPanel {
    pub player_ship_id: i64,
    pub health: HealthState,
    pub cap_current: f64,
    pub cap_max: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudTargetPanel {
    pub lock_target_id: i64,
    pub target_known: bool,
    pub distance_text: String,
    pub health: Option<HealthState>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudStatsPanel {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudSnapshot {
    pub status: HudStatusPanel,
    pub ship_status: HudShipStatusPanel,
    pub target: HudTargetPanel,
    pub modules: Vec<ModuleRow>,
    pub stats: HudStatsPanel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HudChangeSet {
    pub status_changed: bool,
    pub ship_status_changed: bool,
    pub target_changed: bool,
    pub modules_changed: bool,
    pub module_structure_changed: bool,
    pub stats_changed: bool,
}

impl HudChangeSet {
    const fn first_frame() -> Self {
        Self {
            status_changed: true,
            ship_status_changed: true,
            target_changed: true,
            modules_changed: true,
            module_structure_changed: true,
            stats_changed: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HudFrame {
    pub snapshot: HudSnapshot,
    pub changes: HudChangeSet,
}

#[derive(Debug, Default)]
pub struct HudReadModel {
    previous: Option<HudSnapshot>,
}

impl HudReadModel {
    pub fn reset(&mut self) {
        self.previous = None;
    }

    #[must_use]
    pub fn project(
        &mut self,
        session: &WorldSessionState,
        loadout: Option<&PlayerLoadoutMsg>,
        facts: &HudSceneFacts,
    ) -> HudFrame {
        let snapshot = build_snapshot(session, loadout, facts);
        let changes = match self.previous.as_ref() {
            None => HudChangeSet::first_frame(),
            Some(previous) => HudChangeSet {
                status_changed: previous.status != snapshot.status,
                ship_status_changed: previous.ship_status != snapshot.ship_status,
                target_changed: previous.target != snapshot.target,
                modules_changed: previous.modules != snapshot.modules,
                module_structure_changed: active_module_signature(&previous.modules)
                    != active_module_signature(&snapshot.modules),
                stats_changed: previous.stats != snapshot.stats,
            },
        };
        self.previous = Some(snapshot.clone());
        HudFrame { snapshot, changes }
    }
}

fn format_speed(meters_per_second: f64) -> String {
    if !meters_per_second.is_finite() {
        return "-".to_owned();
    }
    let magnitude = meters_per_second.abs();
    if magnitude < 1_000.0 {
        return format!("{} m/s", meters_per_second.trunc() as i64);
    }
    if magnitude < 0.01 * AU_METERS {
        return format!("{:.2} km/s", meters_per_second / 1_000.0);
    }
    format!("{:.3} AU/s", meters_per_second / AU_METERS)
}

fn format_distance(meters: f64) -> String {
    if !meters.is_finite() {
        return "—".to_owned();
    }
    let magnitude = meters.abs();
    if magnitude < 1_000.0 {
        return format!("{} m", meters.trunc() as i64);
    }
    if magnitude < 0.01 * AU_METERS {
        return format!("{:.1} km", meters / 1_000.0);
    }
    format!("{:.3} AU", meters / AU_METERS)
}

fn build_snapshot(
    session: &WorldSessionState,
    loadout: Option<&PlayerLoadoutMsg>,
    facts: &HudSceneFacts,
) -> HudSnapshot {
    let target_known = session.player_lock_target() >= 0 && facts.target_known;
    let target_distance = target_known
        .then_some(facts.target_distance_units)
        .flatten()
        .filter(|distance| distance.is_finite())
        .map(format_distance)
        .unwrap_or_else(|| "—".to_owned());
    let target = HudTargetPanel {
        lock_target_id: session.player_lock_target(),
        target_known,
        distance_text: target_distance,
        health: target_known
            .then(|| {
                session
                    .ship_hp()
                    .get(&session.player_lock_target())
                    .copied()
            })
            .flatten(),
    };
    let speed_text = facts
        .player_speed_units
        .filter(|speed| speed.is_finite())
        .map(format_speed)
        .unwrap_or_else(|| "-".to_owned());
    let modules = loadout
        .map(|loadout| loadout.modules.clone())
        .unwrap_or_default();

    HudSnapshot {
        status: HudStatusPanel {
            connected: facts.connected,
            ship_type_name: session.player_ship_type_name().to_owned(),
            system_name: session.current_system_name().to_owned(),
            speed_text,
        },
        ship_status: HudShipStatusPanel {
            player_ship_id: session.player_ship_id(),
            health: session.player_health(),
            cap_current: session.cap_current(),
            cap_max: session.cap_max(),
        },
        target,
        modules,
        stats: HudStatsPanel {
            text: build_stats_text(session, facts),
        },
    }
}

fn build_stats_text(session: &WorldSessionState, facts: &HudSceneFacts) -> String {
    let jump_line = if facts.nearby_gate_id >= 0 {
        format!("\n[J] Jump Gate #{}", facts.nearby_gate_id)
    } else {
        String::new()
    };
    let jump_line = format!("{}{}", jump_line, non_empty_line(&facts.jump_notice));

    let station_line = if session.is_docked() {
        let station_id = session.docked_station_id();
        let station_name = if session.docked_station_name().is_empty() {
            format!("Station #{station_id}")
        } else {
            session.docked_station_name().to_owned()
        };
        if session.player_ship_id() >= 0 {
            format!(
                "\nDocked: {station_name}\n[U] Undock  [B] Build Magpie\n[Y] Disassemble ship  [X] Disembark"
            )
        } else {
            format!("\nDisembarked at: {station_name}\n(no active ship)")
        }
    } else if facts.nearby_station_ids.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = facts
            .nearby_station_ids
            .iter()
            .map(|id| station_display_name(session, *id))
            .collect();
        let nearest = names.first().cloned().unwrap_or_default();
        if names.len() == 1 {
            format!("\nNearby: {nearest}\n[D] Dock at {nearest}")
        } else {
            format!(
                "\nNearby: {}\n[D] Dock at {nearest} (nearest)",
                names.join(", ")
            )
        }
    };

    let keep_at_range_km = if facts.keep_at_range_km.is_finite() {
        facts.keep_at_range_km
    } else {
        10.0
    };
    let keep_at_range_hint = format!(
        "\n[O] Orbit  [K] Keep at {:.0} km  ([/]  adjust)",
        keep_at_range_km
    );
    let approach_line = if facts.selected_gate_id >= 0 {
        let mut line = format!(
            "\n[A] Approach Gate #{}{}",
            facts.selected_gate_id, keep_at_range_hint
        );
        if let Some(distance) = facts
            .selected_gate_distance_units
            .filter(|distance| distance.is_finite())
        {
            if distance >= ClientRules::min_warp_distance() {
                line.push_str("\n[W] Warp  [J] Warp+Jump");
            } else if distance >= 0.0 {
                line.push_str("\n[W] too close to warp");
            }
        }
        line
    } else if facts.selected_body_id >= 0 {
        let name = session
            .bodies()
            .iter()
            .find(|body| body.body_id == facts.selected_body_id)
            .map(|body| body.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("");
        let name = if name.is_empty() {
            format!("Body #{}", facts.selected_body_id)
        } else {
            name.to_owned()
        };
        format!("\n[W] Warp to {name}")
    } else if facts.selected_station_id >= 0 {
        format!(
            "\n[W] Warp to {}",
            station_display_name(session, facts.selected_station_id)
        )
    } else if facts.selected_target_id >= 0 {
        format!(
            "\n[A] Approach #{}{}",
            facts.selected_target_id, keep_at_range_hint
        )
    } else {
        String::new()
    };

    format!(
        "Ships: {}\nTick: {}{}{}\n\n[Click] Select  [DoubleClick] Thrust\n[RightClick] Lock{}",
        session.ship_count(),
        session.current_tick(),
        approach_line,
        station_line,
        jump_line
    )
}

fn station_display_name(session: &WorldSessionState, station_id: i64) -> String {
    session
        .stations()
        .iter()
        .find(|station| station.station_id == station_id)
        .map(|station| station.name.as_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("Station #{station_id}"))
}

fn non_empty_line(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        format!("\n{value}")
    }
}

fn active_module_signature(modules: &[ModuleRow]) -> Vec<(u32, String)> {
    modules
        .iter()
        .filter(|module| module.is_active_module)
        .map(|module| (module.module_id, module.slot.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CelestialBodyInput, NavigationInput, PositionInput, ShipInput, StationInput};
    use dawn_core::ModuleKind;

    fn player_state() -> WorldSessionState {
        let mut state = WorldSessionState::default();
        state.register_ship(
            7,
            ShipInput {
                is_player: true,
                ship_type_name: "Magpie".to_owned(),
                max_shield: 500.0,
                max_armor: 300.0,
                max_hull: 200.0,
                current_shield: Some(250.0),
                current_armor: Some(300.0),
                current_hull: Some(200.0),
                cap_max: 100.0,
                cap_recharge_per_tick: 10.0,
            },
            7,
        );
        state
    }

    fn module(id: u32, active: bool, active_module: bool) -> ModuleRow {
        ModuleRow {
            slot: "High".to_owned(),
            index: id,
            module_id: id,
            name: format!("Module {id}"),
            kind: ModuleKind::Weapon,
            is_active: active,
            is_active_module: active_module,
            cap_cost_per_cycle: 0.0,
            cycle_time_ticks: 10,
            stat_delta: dawn_core::StatDelta::ZERO,
            cycle_remaining: 0,
            forced_reason: String::new(),
        }
    }

    #[test]
    fn formatting_keeps_existing_unit_boundaries() {
        assert_eq!(format_speed(250.0), "250 m/s");
        assert_eq!(format_speed(999.0), "999 m/s");
        assert_eq!(format_speed(1_000.0), "1.00 km/s");
        assert_eq!(format_speed(1_500.0), "1.50 km/s");
        assert_eq!(format_speed(0.5 * AU_METERS), "0.500 AU/s");
        assert_eq!(format_distance(500.0), "500 m");
        assert_eq!(format_distance(2_500.0), "2.5 km");
        assert_eq!(format_distance(1.2 * AU_METERS), "1.200 AU");
        assert_eq!(format_speed(f64::NAN), "-");
        assert_eq!(format_distance(f64::INFINITY), "—");
    }

    #[test]
    fn projection_preserves_contextual_hud_text_and_invalid_fallbacks() {
        let state = player_state();
        let facts = HudSceneFacts {
            connected: true,
            player_speed_units: Some(120.0),
            target_distance_units: None,
            nearby_gate_id: 3,
            jump_notice: "No target locked".to_owned(),
            selected_target_id: 9,
            ..HudSceneFacts::default()
        };
        let mut model = HudReadModel::default();
        let frame = model.project(&state, None, &facts);
        assert_eq!(frame.snapshot.status.speed_text, "120 m/s");
        assert_eq!(frame.snapshot.target.distance_text, "—");
        assert!(frame.snapshot.stats.text.contains("Jump Gate #3"));
        assert!(frame.snapshot.stats.text.contains("No target locked"));
        assert!(frame.snapshot.stats.text.contains("Approach #9"));
    }

    #[test]
    fn change_decisions_are_value_based_and_track_module_structure_separately() {
        let state = player_state();
        let loadout = PlayerLoadoutMsg {
            modules: vec![module(1, false, true)],
            ..PlayerLoadoutMsg::default()
        };
        let mut model = HudReadModel::default();
        let first = model.project(&state, Some(&loadout), &HudSceneFacts::default());
        assert!(first.changes.modules_changed);
        assert!(first.changes.module_structure_changed);

        let equal = model.project(&state, Some(&loadout), &HudSceneFacts::default());
        assert!(!equal.changes.modules_changed);
        assert!(!equal.changes.module_structure_changed);

        let mut active = loadout.clone();
        active.modules[0].is_active = true;
        let state_change = model.project(&state, Some(&active), &HudSceneFacts::default());
        assert!(state_change.changes.modules_changed);
        assert!(!state_change.changes.module_structure_changed);

        active.modules.push(module(2, false, true));
        let structure_change = model.project(&state, Some(&active), &HudSceneFacts::default());
        assert!(structure_change.changes.modules_changed);
        assert!(structure_change.changes.module_structure_changed);
    }

    #[test]
    fn target_health_is_only_projected_when_the_scene_still_knows_the_target() {
        let mut state = player_state();
        state.register_ship(
            9,
            ShipInput {
                is_player: false,
                ship_type_name: "Target".to_owned(),
                max_shield: 100.0,
                max_armor: 100.0,
                max_hull: 100.0,
                ..ShipInput::default()
            },
            7,
        );
        state.apply_target_locked(7, 9);
        let visible = HudSceneFacts {
            target_known: true,
            target_distance_units: Some(3_200.0),
            ..HudSceneFacts::default()
        };
        let hidden = HudSceneFacts::default();
        let mut model = HudReadModel::default();
        assert!(model
            .project(&state, None, &visible)
            .snapshot
            .target
            .health
            .is_some());
        let hidden_target = model.project(&state, None, &hidden).snapshot.target;
        assert!(!hidden_target.target_known);
        assert!(hidden_target.health.is_none());
        assert_eq!(hidden_target.distance_text, "—");
    }

    #[test]
    fn unknown_target_cannot_expose_a_stale_distance() {
        let state = player_state();
        let facts = HudSceneFacts {
            target_known: false,
            target_distance_units: Some(3_200.0),
            ..HudSceneFacts::default()
        };

        let target = HudReadModel::default()
            .project(&state, None, &facts)
            .snapshot
            .target;

        assert_eq!(target.distance_text, "—");
        assert!(target.health.is_none());
    }

    #[test]
    fn station_and_body_names_have_display_fallbacks() {
        let mut state = player_state();
        state.ingest_navigation(NavigationInput {
            stations: vec![StationInput {
                station_id: 4,
                name: String::new(),
                position: PositionInput::default(),
                docking_radius: 1.0,
            }],
            celestial_bodies: vec![CelestialBodyInput {
                id: 6,
                name: String::new(),
                ..CelestialBodyInput::default()
            }],
            ..NavigationInput::default()
        });
        let facts = HudSceneFacts {
            nearby_station_ids: vec![4],
            selected_body_id: 6,
            ..HudSceneFacts::default()
        };
        let mut model = HudReadModel::default();
        let text = model.project(&state, None, &facts).snapshot.stats.text;
        assert!(text.contains("Station #4"));
        assert!(text.contains("Warp to Body #6"));
    }
}
