use dawn_client_core::ItemRow as CoreItemRow;
use godot::prelude::*;

use crate::item_identity_gd::ItemIdentity;

/// GDScript-facing projection of one typed client Item row.
///
/// Identity crosses the Rust/Godot boundary as one `ItemIdentity` object. The
/// former `item_type`/`module_id`/`ship_type_id` pseudo-union is deliberately
/// not exposed.
#[derive(GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct ItemRow {
    #[var]
    item_id: Gd<ItemIdentity>,
    #[var]
    name: GString,
    #[var]
    kind: GString,
    #[var]
    slot: GString,
    #[var]
    count: i64,
}

impl ItemRow {
    pub(crate) fn wrap(row: CoreItemRow) -> Gd<Self> {
        Gd::from_init_fn(|_base| Self {
            item_id: ItemIdentity::wrap(row.item_id),
            name: (&row.name).into(),
            kind: (&row.kind).into(),
            slot: (&row.slot).into(),
            count: i64::try_from(row.count)
                .expect("PlayerLoadout range validation covers item counts"),
        })
    }
}

#[godot_api]
impl ItemRow {
    /// Debug-only typed fixture for GdUnit. Production callers receive ItemRow
    /// values only from the PlayerLoadout projection and cannot reconstruct a
    /// legacy scalar identity shape.
    #[cfg(debug_assertions)]
    #[func]
    fn test_fixture(
        item_id: Gd<ItemIdentity>,
        name: GString,
        kind: GString,
        slot: GString,
        count: i64,
    ) -> Variant {
        let Ok(count) = u64::try_from(count) else {
            return Variant::nil();
        };
        Self::wrap(CoreItemRow {
            item_id: item_id.bind().get(),
            name: name.to_string(),
            kind: kind.to_string(),
            slot: slot.to_string(),
            count,
        })
        .to_variant()
    }
}
