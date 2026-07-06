## item_row.gd
##
## Typed shape shared by PlayerLoadout's "inventory" and "station_inventory"
## arrays -- both carry the same Item row shape (ADR-0034 generalized
## InventoryComp to Module / PackagedShip / ScrapMetal, so these rows must
## stay generic enough for all three). Preloaded rather than referenced by
## global class_name, same as player_loadout.gd itself.
##
## Note on typing: because this script has no class_name, it has no way to
## name its own type inside its own body -- see module_row.gd's header for
## the full explanation. from_json() below uses bare `new()` and an untyped
## `Variant` return for that reason.
##
## `from_json()` is the only way to construct one: it validates that every
## key is present and returns null (after a push_error) if not, so a schema
## mismatch fails loudly instead of silently defaulting.
extends RefCounted

var item_type: String = "Module"
var module_id: int = 0
var ship_type_id: int = 0
var name: String = "?"
var kind: String = ""
var slot: String = ""
var count: int = 1

const REQUIRED_KEYS: Array[String] = [
	"item_type", "module_id", "ship_type_id", "name", "kind", "slot", "count",
]


static func from_json(src: Dictionary) -> Variant:
	for key: String in REQUIRED_KEYS:
		if not src.has(key):
			push_error(
				"ItemRow.from_json: invalid item row, missing '%s' -- dropping. keys=%s"
				% [key, src.keys()]
			)
			return null

	var row = new()
	row.item_type = src["item_type"] as String
	row.module_id = src["module_id"] as int
	row.ship_type_id = src["ship_type_id"] as int
	row.name = src["name"] as String
	row.kind = src["kind"] as String
	row.slot = src["slot"] as String
	row.count = src["count"] as int
	return row
