## Minimal scene-tree contract tests for the Market UI surface.
extends GdUnitTestSuite

const MarketSurface = preload("res://scripts/market_surface.gd")


func test_market_surface_can_open_and_apply_a_snapshot() -> void:
	var hud := CanvasLayer.new()
	add_child(hud)
	var surface := MarketSurface.new()
	surface.build(hud, Callable(), Callable(), Callable())

	assert_bool(surface.is_open()).is_false()
	assert_bool(surface.toggle()).is_true()
	surface.apply_snapshot({
		"balance": 250,
		"orders": [],
		"notice": "Order placed",
	})
	assert_str(surface._balance_label.text).is_equal("Currency: 250")
	assert_str(surface._notice_label.text).is_equal("Order placed")
	surface.set_open(false)
	assert_bool(surface.is_open()).is_false()
