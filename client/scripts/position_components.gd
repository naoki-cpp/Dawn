## Small wire-shape adapter for absolute positions.
##
## WorldSession state now lives in dawn-client-core. These helpers only accept
## the legacy Dictionary/Variant shapes used by visual marker metadata and
## older event fixtures, then normalize them to f64 component arrays.
class_name PositionComponents
extends RefCounted


static func from_dict(d: Dictionary, key: String) -> PackedFloat64Array:
	var v: Dictionary = d.get(key, {}) as Dictionary
	return PackedFloat64Array([
		v.get("x", 0.0) as float,
		v.get("y", 0.0) as float,
		v.get("z", 0.0) as float])


static func from_value(value: Variant) -> PackedFloat64Array:
	if value is PackedFloat64Array:
		var packed := value as PackedFloat64Array
		if packed.size() >= 3:
			return packed
	if value is Vector3:
		var legacy := value as Vector3
		return PackedFloat64Array([legacy.x, legacy.y, legacy.z])
	return PackedFloat64Array([0.0, 0.0, 0.0])
