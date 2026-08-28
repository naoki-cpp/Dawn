//! Canonical identity conversions at the Godot and wire boundaries.

use dawn_core::{CelestialBodyId, EntityId, JumpGateId, ShipId, ShipTypeId, StationId};

pub(crate) fn ship_id_from_wire(raw: u64) -> ShipId {
    ShipId(EntityId::from_raw(raw))
}

pub(crate) fn ship_id_from_godot(raw: i64) -> Option<ShipId> {
    u64::try_from(raw).ok().map(ship_id_from_wire)
}

pub(crate) fn ship_id_to_godot(id: ShipId) -> i64 {
    i64::try_from(id.raw()).expect("validated server ship IDs fit Godot's signed 64-bit integer")
}

pub(crate) fn jump_gate_id_from_godot(raw: i64) -> Option<JumpGateId> {
    u32::try_from(raw).ok().map(JumpGateId)
}

pub(crate) fn celestial_body_id_from_godot(raw: i64) -> Option<CelestialBodyId> {
    u32::try_from(raw).ok().map(CelestialBodyId)
}

pub(crate) fn station_id_from_godot(raw: i64) -> Option<StationId> {
    u32::try_from(raw).ok().map(StationId)
}

pub(crate) fn ship_type_id_from_godot(raw: i64) -> Option<ShipTypeId> {
    u32::try_from(raw).ok().map(ShipTypeId)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_godot_ids_reject_negative_values() {
        assert_eq!(ship_id_from_godot(-1), None);
        assert_eq!(jump_gate_id_from_godot(-1), None);
        assert_eq!(celestial_body_id_from_godot(-1), None);
        assert_eq!(station_id_from_godot(-1), None);
        assert_eq!(ship_type_id_from_godot(-1), None);
    }

    #[test]
    fn ship_id_round_trips_through_the_godot_boundary() {
        let ship_id = ship_id_from_wire(i64::MAX as u64);
        assert_eq!(ship_id_from_godot(ship_id_to_godot(ship_id)), Some(ship_id));
    }
}
