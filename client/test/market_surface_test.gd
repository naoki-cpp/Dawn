## Minimal scene-tree contract tests for the Market UI surface.
extends GdUnitTestSuite

const MarketSurface = preload("res://scripts/market_surface.gd")


class MarketOutcomeTarget:
	extends RefCounted

	var snapshot: MarketSnapshot

	func _on_market_snapshot(value: MarketSnapshot) -> void:
		snapshot = value


func test_market_surface_can_open_and_apply_a_snapshot() -> void:
	var hud := CanvasLayer.new()
	add_child(hud)
	var surface := MarketSurface.new()
	surface.build(hud, Callable(), Callable(), Callable())

	var target := MarketOutcomeTarget.new()
	var outcome: ServerMessageOutcome = ServerMessageDecoder.new().test_outcome("MarketSnapshot")
	assert_bool(outcome.dispatch(
		target, target, WorldSession.new(), PlayerLoadout.new(), -1
	)).is_true()

	assert_bool(surface.is_open()).is_false()
	assert_bool(surface.toggle()).is_true()
	surface.apply_snapshot(target.snapshot)
	assert_str(surface._balance_label.text).is_equal("Currency: 250")
	assert_str(surface._notice_label.text).is_equal("Ready")
	surface.set_open(false)
	assert_bool(surface.is_open()).is_false()
