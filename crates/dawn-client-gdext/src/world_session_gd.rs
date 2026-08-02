use dawn_client_core::{WorldSessionEffect, WorldSessionState, WorldSessionUpdate};
use godot::prelude::*;

type Dict = Dictionary<Variant, Variant>;

use crate::loadout_gd::PlayerLoadout;
use crate::session_record_gd::{
    BuildableShipType, CapacitorStatus, CelestialBodyRecord, GateRecord, ShipHealth, StationRecord,
};

/// Godot adapter for the pure `dawn-client-core::WorldSessionState` model.
///
/// This class owns no Node3D references. `main.gd` keeps the visual ship-node
/// registry and uses the returned outcomes to apply scene-tree side effects.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct WorldSession {
    state: WorldSessionState,
}

impl WorldSession {
    pub(crate) fn apply_update(
        &mut self,
        update: WorldSessionUpdate,
        loadout: Option<&mut dawn_client_core::PlayerLoadoutMsg>,
    ) -> WorldSessionEffect {
        self.state.apply_update(update, loadout)
    }

    pub(crate) fn station_name(&self, station_id: i64) -> String {
        self.state.station_name(station_id)
    }
}

#[godot_api]
impl WorldSession {
    #[func]
    fn reset(&mut self) {
        self.state.reset();
    }

    #[func]
    fn has_ship(&self, ship_id: i64) -> bool {
        self.state.has_ship(ship_id)
    }

    #[func]
    fn ship_count(&self) -> i64 {
        self.state.ship_count() as i64
    }

    /// `main.gd` writes `_player_ship_id`/`_player_lock_target` optimistically
    /// ahead of the server's confirming event, then reconciles against these
    /// after every event -- unlike every other field WorldSession tracks,
    /// which main.gd now reads directly at point of use instead of mirroring
    /// (ADR-0046).
    #[func]
    fn player_ship_id(&self) -> i64 {
        self.state.player_ship_id()
    }

    #[func]
    fn player_lock_target(&self) -> i64 {
        self.state.player_lock_target()
    }

    /// Advances explicitly client-owned prediction time. Server-driven tick
    /// changes remain internal to `ServerMessageOutcome::dispatch`.
    #[func]
    fn advance_client_ticks(&mut self, ticks: i64, mut loadout: Gd<PlayerLoadout>) {
        let mut loadout = loadout.bind_mut();
        self.state.advance_client_ticks(ticks, loadout.core_mut());
    }

    #[func]
    fn is_docked(&self) -> bool {
        self.state.is_docked()
    }

    /// Split out of a former `dock_status()` `Dictionary`: of its nine call
    /// sites, eight wanted exactly one of these values, so they now cost one
    /// method call instead of building a four-key bag to read one key out of.
    #[func]
    fn docked_station_id(&self) -> i64 {
        self.state.docked_station_id()
    }

    #[func]
    fn docked_station_name(&self) -> GString {
        self.state.docked_station_name().into()
    }

    #[func]
    fn latest_dock_state_tick(&self) -> i64 {
        self.state.latest_dock_state_tick()
    }

    #[func]
    fn player_ship_type_name(&self) -> GString {
        self.state.player_ship_type_name().into()
    }

    /// Bundles the player's shield/armor/hull current and max values --
    /// the HUD always reads all six together, so this is one call instead of
    /// six. Typed rather than a `Dictionary`: the HUD reads six fields off it
    /// every frame, and key strings put the record's shape in the caller.
    #[func]
    fn player_health(&self) -> Gd<ShipHealth> {
        ShipHealth::wrap(self.state.player_ship_id(), self.state.player_health())
    }

    #[func]
    fn current_tick(&self) -> i64 {
        self.state.current_tick()
    }

    #[func]
    fn current_system_name(&self) -> GString {
        self.state.current_system_name().into()
    }

    /// Bundles the capacitor's current/max/recharge values -- always read
    /// together by the HUD, same rationale as `player_health()`.
    #[func]
    fn capacitor_status(&self) -> Gd<CapacitorStatus> {
        CapacitorStatus::wrap(
            self.state.cap_current(),
            self.state.cap_max(),
            self.state.cap_recharge(),
        )
    }

    // `snapshot()` is gone. It projected the whole session into one 22-key
    // `Dictionary` for tests while production read the individual accessors,
    // so the two read paths could drift and only the test side would notice.
    // Tests now go through the same accessors production does (ADR-0046).

    /// One ship's HP layers, or `null` if the session has no such ship.
    ///
    /// Deliberately per-ship rather than a whole-map `ship_hp()`: the HUD's
    /// locked-target readout runs every frame and wants exactly one entry.
    #[func]
    fn ship_health(&self, ship_id: i64) -> Variant {
        match self.state.ship_hp().get(&ship_id) {
            Some(health) => ShipHealth::wrap(ship_id, *health).to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn opponent_ship_ids(&self) -> Array<i64> {
        self.state.opponent_ship_ids().iter().copied().collect()
    }

    #[func]
    fn gates(&self) -> Array<Gd<GateRecord>> {
        self.state.gates().iter().map(GateRecord::wrap).collect()
    }

    #[func]
    fn stations(&self) -> Array<Gd<StationRecord>> {
        self.state
            .stations()
            .iter()
            .map(StationRecord::wrap)
            .collect()
    }

    #[func]
    fn bodies(&self) -> Array<Gd<CelestialBodyRecord>> {
        self.state
            .bodies()
            .iter()
            .map(CelestialBodyRecord::wrap)
            .collect()
    }

    #[func]
    fn buildable_ship_types(&self) -> Array<Gd<BuildableShipType>> {
        self.state
            .buildable_ship_types()
            .iter()
            .map(BuildableShipType::wrap)
            .collect()
    }

    #[func]
    fn system_names(&self) -> Dict {
        let mut result = Dict::new();
        for (id, name) in self.state.system_names() {
            result.set(*id, name.clone());
        }
        result
    }
}
