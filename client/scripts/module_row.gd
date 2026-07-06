## module_row.gd
##
## Typed shape for one row of PlayerLoadout's "modules" array (a fitted
## module slot). Preloaded rather than referenced by global class_name, same
## as player_loadout.gd itself -- see PlayerLoadout's own header comment for
## the headless-test class-cache reason.
##
## Note on typing: because this script has no class_name, it has no way to
## name its own type inside its own body (external files bind it via
## `const ModuleRow = preload(...)` instead). So from_json()/clone()/equals()
## below use bare `new()` (always resolves to "this script's class") and
## untyped `Variant`/`Object` where a self-reference would otherwise be
## needed -- everywhere else (player_loadout.gd, hud_manager.gd, tests, ...)
## the normal `const ModuleRow = preload(...)` + `-> ModuleRow` typing works.
##
## `from_json()` is the only way to construct one: it validates that every
## wire-sourced key is present and returns null (after a push_error) if not,
## so a schema mismatch fails loudly instead of silently defaulting.
extends RefCounted

var slot: String = ""
var index: int = 0
var module_id: int = 0
var name: String = "?"
var kind: String = ""
var is_active: bool = false
var is_active_module: bool = false
var cap_cost_per_cycle: float = 0.0
var cycle_time_ticks: int = 10
var stat_delta: Dictionary = {}

## Client-local runtime state, never read from the wire. Set to their rest
## values here and mutated later by PlayerLoadout.apply_module_activation().
var cycle_remaining: int = 0
var forced_reason: String = ""

const REQUIRED_KEYS: Array[String] = [
	"slot", "index", "module_id", "name", "kind",
	"is_active", "is_active_module",
	"cap_cost_per_cycle", "cycle_time_ticks", "stat_delta",
]


## Returns null if `src` is missing a required wire key. `stat_delta`'s
## sub-keys (weapon_range_add, repair_range_add, ...) are not validated here
## -- they already default-fill individually and new ones land over time
## (ADR-0034's repair_range_add is one such addition).
static func from_json(src: Dictionary) -> Variant:
	for key: String in REQUIRED_KEYS:
		if not src.has(key):
			push_error(
				"ModuleRow.from_json: invalid module row, missing '%s' -- dropping. keys=%s"
				% [key, src.keys()]
			)
			return null

	var row = new()
	row.slot = src["slot"] as String
	row.index = src["index"] as int
	row.module_id = src["module_id"] as int
	row.name = src["name"] as String
	row.kind = src["kind"] as String
	row.is_active = src["is_active"] as bool
	row.is_active_module = src["is_active_module"] as bool
	row.cap_cost_per_cycle = src["cap_cost_per_cycle"] as float
	row.cycle_time_ticks = src["cycle_time_ticks"] as int

	var d: Dictionary = src["stat_delta"] as Dictionary
	row.stat_delta = {
		"weapon_range_add": d.get("weapon_range_add", 0.0) as float,
		"falloff_range_add": d.get("falloff_range_add", 0.0) as float,
		"tackle_range_add": d.get("tackle_range_add", 0.0) as float,
		"repair_range_add": d.get("repair_range_add", 0.0) as float,
	}
	return row


## An independent copy, not an alias -- ModuleRow is mutated in place
## (apply_module_activation, capacitor cycling), so a naive
## Array.duplicate(true) on an Array[ModuleRow] would keep the same object
## references and silently corrupt any snapshot taken for change detection
## (see hud_surface.gd's _prev_modules).
func clone() -> Variant:
	var copy = new()
	copy.slot = slot
	copy.index = index
	copy.module_id = module_id
	copy.name = name
	copy.kind = kind
	copy.is_active = is_active
	copy.is_active_module = is_active_module
	copy.cap_cost_per_cycle = cap_cost_per_cycle
	copy.cycle_time_ticks = cycle_time_ticks
	copy.stat_delta = stat_delta.duplicate()
	copy.cycle_remaining = cycle_remaining
	copy.forced_reason = forced_reason
	return copy


## Field-value equality. Object's own `==`/`!=` is reference identity, not
## value comparison, so anything diffing two ModuleRow snapshots (rather
## than comparing an object to itself) must go through this instead.
func equals(other: Object) -> bool:
	if other == null:
		return false
	return slot == other.slot \
		and index == other.index \
		and module_id == other.module_id \
		and name == other.name \
		and kind == other.kind \
		and is_active == other.is_active \
		and is_active_module == other.is_active_module \
		and cap_cost_per_cycle == other.cap_cost_per_cycle \
		and cycle_time_ticks == other.cycle_time_ticks \
		and stat_delta == other.stat_delta \
		and cycle_remaining == other.cycle_remaining \
		and forced_reason == other.forced_reason
