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
