use super::{parse_required, read_required, validation::*, CatalogError};
use dawn_core::ship_type::{
    ShipBaseStats, ShipClass, ShipTypeDefinition, ShipTypeId, SlotLayout,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

pub(crate) fn load_ship_types_file(
    path: impl AsRef<Path>,
) -> Result<Vec<ShipTypeDefinition>, CatalogError> {
    let path = path.as_ref();
    let source = read_required("ship-type", path)?;
    let parsed: ShipTypesFile = parse_required("ship-type", path, &source)?;
    let ship_types = parsed
        .ship_types
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    validate_ship_types(&ship_types, path)?;
    Ok(ship_types)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShipTypesFile {
    ship_types: Vec<ShipTypeEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShipTypeEntry {
    id: u32,
    name: String,
    class: ShipClass,
    slot_layout: SlotLayoutEntry,
    base_stats: BaseStatsEntry,
    buildable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotLayoutEntry {
    high: u8,
    mid: u8,
    low: u8,
    rig: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseStatsEntry {
    max_speed: f64,
    mass: f64,
    inertia_modifier: f64,
    max_shield: f32,
    max_armor: f32,
    max_hull: f32,
    lock_time: u64,
    max_locks: u32,
    cap_max: f32,
    cap_recharge_per_tick: f32,
    sig_radius: f32,
}

impl From<ShipTypeEntry> for ShipTypeDefinition {
    fn from(entry: ShipTypeEntry) -> Self {
        Self {
            id: ShipTypeId(entry.id),
            name: entry.name,
            class: entry.class,
            slot_layout: SlotLayout {
                high: entry.slot_layout.high,
                mid: entry.slot_layout.mid,
                low: entry.slot_layout.low,
                rig: entry.slot_layout.rig,
            },
            buildable: entry.buildable,
            base_stats: ShipBaseStats {
                max_speed: entry.base_stats.max_speed,
                mass: entry.base_stats.mass,
                inertia_modifier: entry.base_stats.inertia_modifier,
                max_shield: entry.base_stats.max_shield,
                max_armor: entry.base_stats.max_armor,
                max_hull: entry.base_stats.max_hull,
                lock_time: entry.base_stats.lock_time,
                max_locks: entry.base_stats.max_locks,
                cap_max: entry.base_stats.cap_max,
                cap_recharge_per_tick: entry.base_stats.cap_recharge_per_tick,
                sig_radius: entry.base_stats.sig_radius,
            },
        }
    }
}


fn validate_ship_types(
    ship_types: &[ShipTypeDefinition],
    path: &Path,
) -> Result<(), CatalogError> {
    if ship_types.is_empty() {
        return validation_error(
            "ship-type",
            path,
            "catalog must contain at least one ship type",
        );
    }

    let mut ids = HashSet::new();
    for ship_type in ship_types {
        if ship_type.id.0 == 0 {
            return validation_error("ship-type", path, "ship type id 0 is reserved");
        }
        if !ids.insert(ship_type.id) {
            return validation_error(
                "ship-type",
                path,
                format!("duplicate ship type id {}", ship_type.id.0),
            );
        }
        if ship_type.name.trim().is_empty() {
            return validation_error(
                "ship-type",
                path,
                format!("ship type id {} has an empty name", ship_type.id.0),
            );
        }
        if u16::from(ship_type.slot_layout.high)
            + u16::from(ship_type.slot_layout.mid)
            + u16::from(ship_type.slot_layout.low)
            + u16::from(ship_type.slot_layout.rig)
            == 0
        {
            return validation_error(
                "ship-type",
                path,
                format!("ship type id {} has no fitting slots", ship_type.id.0),
            );
        }

        let stats = &ship_type.base_stats;
        for (field, value) in [
            ("max_speed", stats.max_speed),
            ("mass", stats.mass),
            ("inertia_modifier", stats.inertia_modifier),
        ] {
            validate_positive_f64("ship-type", path, ship_type.id.0, field, value)?;
        }
        for (field, value) in [
            ("max_shield", stats.max_shield),
            ("max_armor", stats.max_armor),
            ("max_hull", stats.max_hull),
            ("cap_max", stats.cap_max),
            ("cap_recharge_per_tick", stats.cap_recharge_per_tick),
            ("sig_radius", stats.sig_radius),
        ] {
            validate_positive_f32("ship-type", path, ship_type.id.0, field, value)?;
        }
        if stats.lock_time == 0 {
            return validation_error(
                "ship-type",
                path,
                format!("ship type id {} has lock_time 0", ship_type.id.0),
            );
        }
        if stats.max_locks == 0 {
            return validation_error(
                "ship-type",
                path,
                format!("ship type id {} has max_locks 0", ship_type.id.0),
            );
        }
    }
    Ok(())
}

