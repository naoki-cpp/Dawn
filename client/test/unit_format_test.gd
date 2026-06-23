## unit_format_test.gd
##
## Tests for unit_format.gd (ADR-0029 §1.5: single real-unit conversion
## module). Speeds/distances now span m/s up to a real fraction of an AU/s,
## so a single fixed unit stops reading as a sensible number at one end --
## these lock in the escalation thresholds (m/s -> km/s -> AU/s and the
## distance equivalent) and the boundary behaviour around each threshold.
extends GdUnitTestSuite

const UnitFormat = preload("res://scripts/unit_format.gd")
const AU_M: float = 1.495978707e11


func test_speed_under_1000_mps_displays_as_meters_per_second() -> void:
	assert_str(UnitFormat.format_speed(250.0)).is_equal("250 m/s")


func test_speed_in_the_kilometers_per_second_range_displays_as_km_s() -> void:
	# 1500 m/s = 1.5 km/s, comfortably inside the km/s band.
	assert_str(UnitFormat.format_speed(1500.0)).is_equal("1.50 km/s")


func test_warp_speed_displays_as_au_per_second() -> void:
	# Half an AU per second -- representative of true-AU warp cruise speed.
	var mps: float = 0.5 * AU_M
	assert_str(UnitFormat.format_speed(mps)).is_equal("0.500 AU/s")


func test_speed_just_under_the_mps_threshold_does_not_escalate() -> void:
	assert_str(UnitFormat.format_speed(999.0)).is_equal("999 m/s")


func test_speed_just_at_the_mps_threshold_escalates_to_km_s() -> void:
	assert_str(UnitFormat.format_speed(1000.0)).is_equal("1.00 km/s")


func test_distance_under_1000_m_displays_as_meters() -> void:
	assert_str(UnitFormat.format_distance(500.0)).is_equal("500 m")


func test_distance_in_the_kilometer_range_displays_as_km() -> void:
	assert_str(UnitFormat.format_distance(2500.0)).is_equal("2.5 km")


func test_distance_at_au_scale_displays_as_au() -> void:
	var meters: float = 1.2 * AU_M
	assert_str(UnitFormat.format_distance(meters)).is_equal("1.200 AU")
