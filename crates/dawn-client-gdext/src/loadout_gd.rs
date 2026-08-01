use dawn_client_core::PlayerLoadoutMsg;
use godot::prelude::*;

use crate::client_outcome::validate_player_loadout_godot_ranges;
use crate::item_row_gd::ItemRow;
use crate::module_row_gd::{parse_kind, ModuleRow};
use crate::owned_ship_row_gd::OwnedShipRow;

/// `dawn_core::ModuleKind` (the server-authoritative enum, via `dawn-wire`)
/// -> `dawn_client_core::ModuleKind` (the client's own decoupled mirror with
/// an `Unknown` fallback, ADR-0039). Both crates are foreign to
/// `dawn-client-gdext`, so this lives here rather than as a `From` impl
/// (orphan rule).
fn wire_module_kind(kind: dawn_core::ModuleKind) -> dawn_client_core::ModuleKind {
    match kind {
        dawn_core::ModuleKind::Weapon => dawn_client_core::ModuleKind::Weapon,
        dawn_core::ModuleKind::ShieldBooster => dawn_client_core::ModuleKind::ShieldBooster,
        dawn_core::ModuleKind::ArmorRepairer => dawn_client_core::ModuleKind::ArmorRepairer,
        dawn_core::ModuleKind::Propulsion => dawn_client_core::ModuleKind::Propulsion,
        dawn_core::ModuleKind::Sensor => dawn_client_core::ModuleKind::Sensor,
        dawn_core::ModuleKind::Rig => dawn_client_core::ModuleKind::Rig,
        dawn_core::ModuleKind::Tackle => dawn_client_core::ModuleKind::Tackle,
        dawn_core::ModuleKind::RemoteShieldBooster => {
            dawn_client_core::ModuleKind::RemoteShieldBooster
        }
        dawn_core::ModuleKind::RemoteArmorRepairer => {
            dawn_client_core::ModuleKind::RemoteArmorRepairer
        }
    }
}

/// Converts the wire-schema `PlayerLoadoutWire` (`dawn-wire`, ADR-0042 2a)
/// into this crate's richer client-side `PlayerLoadoutMsg`
/// (`dawn-client-core`). A plain function rather than a `From` impl -- both
/// types are foreign to this crate, so the orphan rule forbids it anyway,
/// and this adapter-layer conversion is exactly what `dawn-client-gdext` is
/// for.
fn wire_to_loadout_msg(wire: dawn_wire::PlayerLoadoutWire) -> PlayerLoadoutMsg {
    PlayerLoadoutMsg {
        tick: wire.tick,
        modules: wire.modules.into_iter().map(wire_to_module_row).collect(),
        inventory: wire.inventory.into_iter().map(wire_to_item_row).collect(),
        station_inventory: wire
            .station_inventory
            .into_iter()
            .map(wire_to_item_row)
            .collect(),
        docked_station_id: wire.docked_station_id,
        docked_station_name: wire.docked_station_name,
        slot_capacity: dawn_client_core::SlotCapacity {
            high: wire.slot_capacity.high as u32,
            mid: wire.slot_capacity.mid as u32,
            low: wire.slot_capacity.low as u32,
            rig: wire.slot_capacity.rig as u32,
        },
        active_ship_id: wire.active_ship_id,
        owned_ships: wire
            .owned_ships
            .into_iter()
            .map(|s| dawn_client_core::OwnedShipRow {
                ship_id: s.ship_id,
                ship_type_id: s.ship_type_id,
                ship_type_name: s.ship_type_name,
                docked_station_id: s.docked_station_id,
                is_active: s.is_active,
            })
            .collect(),
    }
}

fn wire_to_module_row(row: dawn_wire::ModuleRowWire) -> dawn_client_core::ModuleRow {
    dawn_client_core::ModuleRow {
        slot: row.slot,
        index: row.index,
        module_id: row.module_id,
        name: row.name,
        kind: wire_module_kind(row.kind),
        is_active: row.is_active,
        is_active_module: row.is_active_module,
        cap_cost_per_cycle: row.cap_cost_per_cycle as f64,
        cycle_time_ticks: u32::try_from(row.cycle_time_ticks)
            .expect("PlayerLoadout range validation covers cycle_time_ticks"),
        // dawn_wire::ModuleRowWire.stat_delta is already dawn_core::StatDelta
        // -- the same type dawn_client_core::ModuleRow.stat_delta now uses
        // directly, so there's nothing to convert.
        stat_delta: row.stat_delta,
        cycle_remaining: 0,
        forced_reason: String::new(),
    }
}

fn wire_to_item_row(row: dawn_wire::ItemRowWire) -> dawn_client_core::ItemRow {
    dawn_client_core::ItemRow {
        item_id: dawn_core::ItemId::try_from(row.item_id)
            .expect("server emitted an invalid Item wire identity"),
        name: row.name,
        kind: row.kind,
        slot: row.slot,
        count: row.count,
    }
}

/// Type-erased Dictionary retained only for `toggle_at()`'s small,
/// closed activation-intent boundary.
type Dict = Dictionary<Variant, Variant>;

/// Client-side deep module for the server's `PlayerLoadout` wire message
/// (ADR-0039/ADR-0040/ADR-0042). It owns typed Rust state and exposes typed
/// Godot read projections through `ModuleRow`, `ItemRow`, `OwnedShipRow`, and
/// narrow scalar accessors for dock context and weapon ranges. Existing
/// boundary sentinels remain explicit; `toggle_at()` is the only deliberately
/// retained Dictionary because it represents one small, closed command intent.
#[derive(GodotClass)]
#[class(init, base=RefCounted)]
pub struct PlayerLoadout {
    loadout: Option<PlayerLoadoutMsg>,
}

#[godot_api]
impl PlayerLoadout {
    pub(crate) fn core_mut(&mut self) -> Option<&mut PlayerLoadoutMsg> {
        self.loadout.as_mut()
    }

    #[func]
    fn reset(&mut self) {
        self.loadout = None;
    }

    /// Test/debug-only convenience: builds state directly from a hand-built
    /// JSON fixture, bypassing the wire entirely. Never called by
    /// `connection.gd` in production (that always goes through
    /// `apply_wire_bytes`, ADR-0042) -- this exists so GdUnit4 tests can set
    /// up a `PlayerLoadout` fixture without constructing real postcard bytes.
    #[func]
    fn apply_payload(&mut self, json: GString) -> bool {
        match serde_json::from_str::<PlayerLoadoutMsg>(&json.to_string()) {
            Ok(loadout) => {
                self.loadout = Some(loadout);
                true
            }
            Err(err) => {
                godot_error!("PlayerLoadout.apply_payload: {err}");
                false
            }
        }
    }

    /// Decodes a `ServerMessage::PlayerLoadout` binary WebSocket frame
    /// (ADR-0042) directly into this crate's typed state -- bypassing the
    /// generic `Dictionary` decode path (`ServerMessageDecoder`) entirely,
    /// since Rust-to-Rust decode preserves exact int/float types with no
    /// GDScript `JSON.parse_string` lossiness to work around. The decoded
    /// values are checked against every narrower Godot/client representation
    /// before state is replaced. Returns `false` and leaves the previous state
    /// untouched if the frame or one of those boundary values is invalid.
    #[func]
    fn apply_wire_bytes(&mut self, bytes: PackedByteArray) -> bool {
        match dawn_wire::ServerMessage::decode(bytes.as_slice()) {
            Ok(dawn_wire::ServerMessage::PlayerLoadout(wire)) => {
                if let Err(error) = validate_player_loadout_godot_ranges(&wire) {
                    godot_error!("PlayerLoadout.apply_wire_bytes: {error}");
                    return false;
                }
                self.loadout = Some(wire_to_loadout_msg(wire));
                true
            }
            Ok(_) => {
                godot_error!("PlayerLoadout.apply_wire_bytes: not a PlayerLoadout message");
                false
            }
            Err(err) => {
                godot_error!("PlayerLoadout.apply_wire_bytes: {err}");
                false
            }
        }
    }

    #[func]
    fn tick(&self) -> i64 {
        self.loadout
            .as_ref()
            .map(|loadout| {
                i64::try_from(loadout.tick)
                    .expect("PlayerLoadout range validation covers the tick")
            })
            .unwrap_or(0)
    }

    #[func]
    fn active_ship_id(&self) -> i64 {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.active_ship_id)
            .map(|id| {
                i64::try_from(id)
                    .expect("PlayerLoadout range validation covers the active ship ID")
            })
            .unwrap_or(-1)
    }

    #[func]
    fn owned_ships(&self) -> Array<Gd<OwnedShipRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for ship in &loadout.owned_ships {
                out.push(&OwnedShipRow::wrap(ship.clone()));
            }
        }
        out
    }

    #[func]
    fn docked_station_id(&self) -> i64 {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.docked_station_id)
            .map(i64::from)
            .unwrap_or(-1)
    }

    #[func]
    fn docked_station_name(&self) -> GString {
        self.loadout
            .as_ref()
            .and_then(|loadout| loadout.docked_station_name.as_deref())
            .unwrap_or_default()
            .into()
    }

    #[func]
    fn is_docked(&self) -> bool {
        self.loadout
            .as_ref()
            .is_some_and(PlayerLoadoutMsg::is_docked)
    }

    #[func]
    fn modules(&self) -> Array<Gd<ModuleRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for row in &loadout.modules {
                out.push(&ModuleRow::wrap(row.clone()));
            }
        }
        out
    }

    #[func]
    fn inventory(&self) -> Array<Gd<ItemRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for row in &loadout.inventory {
                out.push(&ItemRow::wrap(row.clone()));
            }
        }
        out
    }

    #[func]
    fn station_inventory(&self) -> Array<Gd<ItemRow>> {
        let mut out = Array::new();
        if let Some(loadout) = &self.loadout {
            for row in &loadout.station_inventory {
                out.push(&ItemRow::wrap(row.clone()));
            }
        }
        out
    }

    #[func]
    fn apply_module_activation(&mut self, module_id: i64, active: bool, forced_reason: GString) {
        let Some(loadout) = &mut self.loadout else {
            return;
        };
        let Ok(module_id) = u32::try_from(module_id) else {
            return;
        };
        loadout.apply_module_activation(module_id, active, forced_reason.to_string());
    }

    /// Small closed intent boundary kept as a Dictionary deliberately:
    /// `{}` means "nothing to toggle"; otherwise fixed keys describe one command.
    #[func]
    fn toggle_at(&self, active_index: i64) -> Dict {
        let mut d = Dict::new();
        let Some(loadout) = &self.loadout else {
            return d;
        };
        let Ok(index) = usize::try_from(active_index) else {
            return d;
        };
        let Some(intent) = loadout.toggle_at(index) else {
            return d;
        };
        d.set("module_id", intent.module_id as i64);
        d.set("slot", intent.slot);
        d.set("kind", crate::module_row_gd::kind_str(intent.kind));
        d.set("is_active", intent.is_active);
        d.set("requires_target", intent.requires_target);
        d.set("effective_range", intent.effective_range.unwrap_or(-1.0));
        d
    }

    #[func]
    fn weapon_optimal_range(&self) -> f64 {
        self.loadout
            .as_ref()
            .map(PlayerLoadoutMsg::weapon_ranges)
            .map(|(optimal, _)| optimal)
            .unwrap_or(0.0)
    }

    #[func]
    fn weapon_falloff_range(&self) -> f64 {
        self.loadout
            .as_ref()
            .map(PlayerLoadoutMsg::weapon_ranges)
            .map(|(_, falloff)| falloff)
            .unwrap_or(0.0)
    }

    /// `-1.0` if `kind` has no range concept, matching the old
    /// `player_loadout.gd::effective_range_for_activation`'s sentinel.
    #[func]
    fn effective_range_for_activation(&self, kind: GString, module_id: i64) -> f64 {
        let Some(loadout) = &self.loadout else {
            return -1.0;
        };
        let Ok(module_id) = u32::try_from(module_id) else {
            return -1.0;
        };
        loadout
            .effective_range_for_activation(parse_kind(&kind.to_string()), module_id)
            .unwrap_or(-1.0)
    }

    #[func]
    fn simulate_capacitor_ticks(
        &mut self,
        cap_current: f64,
        cap_max: f64,
        cap_recharge: f64,
        ticks: i64,
    ) -> f64 {
        let Some(loadout) = &mut self.loadout else {
            return cap_current;
        };
        let ticks = u32::try_from(ticks).unwrap_or(0);
        loadout.simulate_capacitor_ticks(cap_current, cap_max, cap_recharge, ticks)
    }

    /// Static form of `simulate_capacitor_ticks`, operating on an
    /// ad-hoc module list instead of this instance's own state. Used by
    /// `WorldSessionState::simulate_cap` for the "no `PlayerLoadout` instance
    /// yet, but I have a raw module list" case -- mirrors the old
    /// `player_loadout.gd::simulate_modules_capacitor_ticks` static function.
    #[func]
    fn simulate_modules_capacitor_ticks(
        modules: Array<Gd<ModuleRow>>,
        cap_current: f64,
        cap_max: f64,
        cap_recharge: f64,
        ticks: i64,
    ) -> f64 {
        let ticks = u32::try_from(ticks).unwrap_or(0);
        let mut core_rows: Vec<_> = modules
            .iter_shared()
            .map(|gd| gd.bind().inner_clone())
            .collect();
        let cap = dawn_client_core::simulate_modules_capacitor_ticks(
            &mut core_rows,
            cap_current,
            cap_max,
            cap_recharge,
            ticks,
        );
        for (mut gd, row) in modules.iter_shared().zip(core_rows) {
            gd.bind_mut()
                .apply_simulated_cycle_remaining(row.cycle_remaining);
        }
        cap
    }
}

#[cfg(test)]
mod tests {
    //! Contract test (ADR-0039/ADR-0042): proves the real server's
    //! `PlayerLoadoutWire` (`dawn_sector`'s `build_player_loadout_json`)
    //! still converts into `dawn_client_core::PlayerLoadoutMsg` via
    //! [`wire_to_loadout_msg`]. `dawn-sector` is a dev-dependency only --
    //! this does not add a runtime dependency edge, so it doesn't resurrect
    //! the previously-rejected `dawn-proto` shared-schema crate.
    //!
    //! If this test starts failing, the server's wire shape drifted from
    //! this crate's conversion -- update `wire_to_loadout_msg`/
    //! `wire_to_module_row`/`wire_to_item_row` to match.
    use super::*;
    use dawn_core::{FitModuleCommand, NodeId, Position, SectorBounds, SectorId, SlotKind};
    use dawn_sector::node::SimulationNode;

    fn test_node() -> SimulationNode {
        let mut node = SimulationNode::new(
            NodeId(0),
            SectorId(0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = dawn_sector::game_data::GameDataCatalog::load_from_paths(
            root.join(dawn_sector::game_data::PRODUCTION_MODULES_PATH),
            root.join(dawn_sector::game_data::PRODUCTION_SHIP_TYPES_PATH),
        )
        .expect("repository game-data catalog");
        catalog.register_into(&mut node);
        node
    }

    #[test]
    fn player_loadout_json_from_a_freshly_spawned_ship_converts_into_player_loadout_msg() {
        let mut node = test_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        let wire = node
            .build_player_loadout_json(ship_id)
            .expect("freshly spawned ship has a loadout payload");
        validate_player_loadout_godot_ranges(&wire).expect("production loadout fits client ranges");
        let loadout = wire_to_loadout_msg(wire);

        assert_eq!(loadout.active_ship_id, Some(ship_id.raw()));
        assert!(!loadout.is_docked());
        // A freshly spawned player ship starts with one of every registered
        // module fitted (see spawner_logic.rs's default loadout) plus a
        // starter packaged ship in inventory (9B) -- exercise both row kinds
        // through the real wire shape, not just module rows.
        assert!(!loadout.modules.is_empty());
        assert!(!loadout.inventory.is_empty());
    }

    #[test]
    fn player_loadout_json_with_a_fitted_module_converts_the_stat_delta() {
        let mut node = test_node();
        let player_id = node.next_player_id();
        let ship_id = node.spawn_player_ship_at_pub(player_id, Position::ORIGIN);

        let module_id = dawn_sector::modules::MODULE_RAILGUN_SMALL;
        let fitted = node.fit_module(FitModuleCommand {
            ship_id,
            slot: SlotKind::High,
            module_id,
        });
        assert!(
            fitted,
            "fitting a registered module onto a known ship must succeed"
        );

        let wire = node
            .build_player_loadout_json(ship_id)
            .expect("ship has a loadout payload");
        validate_player_loadout_godot_ranges(&wire).expect("production loadout fits client ranges");
        let loadout = wire_to_loadout_msg(wire);

        let row = loadout
            .modules
            .iter()
            .find(|m| m.module_id == module_id.0)
            .expect("the module just fitted appears as a row");
        // Just needs to be a real f64 (i.e. the field existed and converted)
        // -- the specific value is the module registry's concern, not this
        // conversion's.
        let _ = row.stat_delta.weapon_range_add;
    }
}
