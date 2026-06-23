## unit_format.gd
##
## Single conversion/display module for real-world units (ADR-0029 §1.5: "実値表示
## の内部↔表示変換は単一モジュールに集約する"). Speeds and distances now span many
## orders of magnitude -- sublight is metres/second, warp cruises at a sizeable
## fraction of an AU/second -- so a single "m/s" or "km" string stops being legible
## at one end or the other. These pick the coarsest unit that still reads as a
## sensible-looking number.
##
## Static/stateless: call directly as UnitFormat.format_speed(...), no instance.

extends RefCounted

const AU_M : float = 1.495978707e11  ## Metres per AU (same constant as galaxy.rs).

## `mps`: speed in metres/second (already the server's real-unit convention,
## METERS_PER_UNIT = 1.0). Thresholds chosen so the displayed number stays in a
## comfortable 1-4 digit range: sublight ship speeds are m/s, intra-system
## sublight cruisers/approach speeds land in km/s, and only warp (a real
## fraction of an AU every second) needs AU/s.
static func format_speed(mps: float) -> String:
	var a := absf(mps)
	if a < 1000.0:
		return "%d m/s" % int(mps)
	if a < 0.01 * AU_M:
		return "%.2f km/s" % (mps / 1000.0)
	return "%.3f AU/s" % (mps / AU_M)

## `meters`: distance in metres. Same escalation as format_speed, one step
## coarser per unit (distances we display are typically larger than the speed
## covering them in one second).
static func format_distance(meters: float) -> String:
	var a := absf(meters)
	if a < 1000.0:
		return "%d m" % int(meters)
	if a < 0.01 * AU_M:
		return "%.1f km" % (meters / 1000.0)
	return "%.3f AU" % (meters / AU_M)
