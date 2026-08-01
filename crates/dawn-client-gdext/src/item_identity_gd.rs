use dawn_core::{ItemId, ModuleId, ShipTypeId};
use godot::prelude::*;

/// Canonical Item identity exposed to Godot as one typed object.
///
/// The Rust enum remains the source of truth. GDScript can inspect the
/// selected variant, but it never receives the former three-field
/// `item_type`/`module_id`/`ship_type_id` pseudo-union.
#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ItemIdentity {
    item_id: ItemId,
}

impl ItemIdentity {
    pub(crate) fn wrap(item_id: ItemId) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self { item_id })
    }

    pub(crate) const fn get(&self) -> ItemId {
        self.item_id
    }
}

#[godot_api]
impl ItemIdentity {
    #[func]
    fn kind(&self) -> GString {
        match self.item_id {
            ItemId::Module(_) => "Module".into(),
            ItemId::PackagedShip(_) => "PackagedShip".into(),
            ItemId::ScrapMetal => "ScrapMetal".into(),
        }
    }

    #[func]
    fn is_module(&self) -> bool {
        matches!(self.item_id, ItemId::Module(_))
    }

    #[func]
    fn is_packaged_ship(&self) -> bool {
        matches!(self.item_id, ItemId::PackagedShip(_))
    }

    #[func]
    fn is_scrap_metal(&self) -> bool {
        matches!(self.item_id, ItemId::ScrapMetal)
    }

    /// Returns the module ID for the Module variant, otherwise `null`.
    #[func]
    fn module_id(&self) -> Variant {
        match self.item_id {
            ItemId::Module(module_id) => i64::from(module_id.0).to_variant(),
            _ => Variant::nil(),
        }
    }

    /// Returns the ship-type ID for the PackagedShip variant, otherwise `null`.
    #[func]
    fn ship_type_id(&self) -> Variant {
        match self.item_id {
            ItemId::PackagedShip(ship_type_id) => i64::from(ship_type_id.0).to_variant(),
            _ => Variant::nil(),
        }
    }

    /// Canonical fixture/factory for a Module identity. Invalid or zero IDs
    /// return `null` instead of manufacturing an impossible Item identity.
    #[func]
    fn module(module_id: i64) -> Variant {
        let Ok(module_id) = u32::try_from(module_id) else {
            return Variant::nil();
        };
        if module_id == 0 {
            return Variant::nil();
        }
        Self::wrap(ItemId::Module(ModuleId(module_id))).to_variant()
    }

    /// Canonical fixture/factory for a PackagedShip identity.
    #[func]
    fn packaged_ship(ship_type_id: i64) -> Variant {
        let Ok(ship_type_id) = u32::try_from(ship_type_id) else {
            return Variant::nil();
        };
        if ship_type_id == 0 {
            return Variant::nil();
        }
        Self::wrap(ItemId::PackagedShip(ShipTypeId(ship_type_id))).to_variant()
    }

    #[func]
    fn scrap_metal() -> Gd<Self> {
        Self::wrap(ItemId::ScrapMetal)
    }
}
