//! Pure price-time matching and escrow decisions for the Market order book.
//!
//! `MarketDb` is the SQLite adapter around this module. Keeping the policy
//! here means the matching rules can be tested without a database and the
//! storage implementation only has to persist the resulting plan.

use dawn_core::{ItemId, PlayerId, ShipId};

use crate::order_book::OrderSide;

/// The incoming order presented to the matching policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IncomingOrder {
    pub(super) player_id: PlayerId,
    pub(super) ship_id: ShipId,
    pub(super) item_id: ItemId,
    pub(super) side: OrderSide,
    pub(super) price: u64,
    pub(super) quantity: u64,
}

/// The typed part of a resting database row needed by matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RestingOrder {
    pub(super) order_id: i64,
    pub(super) player_id: PlayerId,
    pub(super) ship_id: Option<ShipId>,
    pub(super) item_id: ItemId,
    pub(super) side: OrderSide,
    pub(super) quantity_remaining: u64,
    pub(super) price: u64,
    pub(super) escrowed_currency: u64,
}

/// One database mutation and settlement decision produced by matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MatchFill {
    pub(super) resting_order_id: i64,
    pub(super) quantity: u64,
    pub(super) resting_quantity_remaining: u64,
    pub(super) resting_escrowed_currency: u64,
    pub(super) buyer: PlayerId,
    pub(super) seller: PlayerId,
    pub(super) buyer_ship_id: Option<ShipId>,
    pub(super) price: u64,
    pub(super) seller_proceeds: u64,
}

/// All decisions needed to apply one incoming order to the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MatchPlan {
    pub(super) fills: Vec<MatchFill>,
    pub(super) remaining_quantity: u64,
    pub(super) buyer_refund: u64,
}

/// Select and price all fills for an incoming order.
///
/// Candidates are deliberately plain values supplied by the storage adapter.
/// This policy owns the important rules: opposite-side filtering, crossing,
/// best-price selection, time priority, partial fills, maker-price
/// settlement, and the incoming Bid's price-improvement refund.
pub(super) fn plan_matches(
    incoming: IncomingOrder,
    candidates: impl IntoIterator<Item = RestingOrder>,
) -> MatchPlan {
    let mut candidates: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| {
            candidate.item_id == incoming.item_id
                && candidate.side == incoming.side.opposite()
                && crosses(incoming, *candidate)
        })
        .collect();

    candidates.sort_by(|left, right| {
        let price_order = match incoming.side {
            // A buyer prefers the cheapest Ask.
            OrderSide::Bid => left.price.cmp(&right.price),
            // A seller prefers the highest Bid.
            OrderSide::Ask => right.price.cmp(&left.price),
        };
        price_order.then_with(|| left.order_id.cmp(&right.order_id))
    });

    let mut remaining_quantity = incoming.quantity;
    let mut buyer_refund = 0;
    let mut fills = Vec::new();

    for resting in candidates {
        if remaining_quantity == 0 {
            break;
        }

        let quantity = remaining_quantity.min(resting.quantity_remaining);
        remaining_quantity -= quantity;

        let (buyer, seller, buyer_ship_id) = match incoming.side {
            OrderSide::Bid => (
                incoming.player_id,
                resting.player_id,
                Some(incoming.ship_id),
            ),
            OrderSide::Ask => (resting.player_id, incoming.player_id, resting.ship_id),
        };
        let seller_proceeds = resting.price * quantity;
        if incoming.side == OrderSide::Bid {
            buyer_refund += (incoming.price - resting.price) * quantity;
        }

        fills.push(MatchFill {
            resting_order_id: resting.order_id,
            quantity,
            resting_quantity_remaining: resting.quantity_remaining - quantity,
            resting_escrowed_currency: if resting.side == OrderSide::Bid {
                resting
                    .escrowed_currency
                    .saturating_sub(resting.price * quantity)
            } else {
                0
            },
            buyer,
            seller,
            buyer_ship_id,
            price: resting.price,
            seller_proceeds,
        });
    }

    MatchPlan {
        fills,
        remaining_quantity,
        buyer_refund,
    }
}

fn crosses(incoming: IncomingOrder, resting: RestingOrder) -> bool {
    match incoming.side {
        OrderSide::Bid => resting.price <= incoming.price,
        OrderSide::Ask => resting.price >= incoming.price,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::NodeId;

    fn item() -> ItemId {
        ItemId::ScrapMetal
    }

    fn ship(id: u64) -> ShipId {
        ShipId::new(NodeId(0), id)
    }

    fn incoming(side: OrderSide, price: u64, quantity: u64) -> IncomingOrder {
        IncomingOrder {
            player_id: PlayerId(9),
            ship_id: ship(9),
            item_id: item(),
            side,
            price,
            quantity,
        }
    }

    fn resting(
        order_id: i64,
        player_id: u64,
        side: OrderSide,
        price: u64,
        quantity: u64,
    ) -> RestingOrder {
        RestingOrder {
            order_id,
            player_id: PlayerId(player_id),
            ship_id: Some(ship(player_id)),
            item_id: item(),
            side,
            quantity_remaining: quantity,
            price,
            escrowed_currency: if side == OrderSide::Bid {
                price * quantity
            } else {
                0
            },
        }
    }

    #[test]
    fn bid_chooses_the_cheapest_crossing_ask_before_time_priority() {
        let plan = plan_matches(
            incoming(OrderSide::Bid, 120, 5),
            [
                resting(1, 1, OrderSide::Ask, 110, 5),
                resting(2, 2, OrderSide::Ask, 100, 5),
            ],
        );

        assert_eq!(plan.fills.len(), 1);
        assert_eq!(plan.fills[0].resting_order_id, 2);
        assert_eq!(plan.fills[0].price, 100);
    }

    #[test]
    fn ask_chooses_the_highest_crossing_bid_before_time_priority() {
        let plan = plan_matches(
            incoming(OrderSide::Ask, 90, 5),
            [
                resting(1, 1, OrderSide::Bid, 100, 5),
                resting(2, 2, OrderSide::Bid, 110, 5),
            ],
        );

        assert_eq!(plan.fills.len(), 1);
        assert_eq!(plan.fills[0].resting_order_id, 2);
        assert_eq!(plan.fills[0].price, 110);
    }

    #[test]
    fn equal_prices_use_the_earlier_order_id_and_preserve_partial_remainder() {
        let plan = plan_matches(
            incoming(OrderSide::Bid, 100, 8),
            [
                resting(2, 2, OrderSide::Ask, 100, 3),
                resting(1, 1, OrderSide::Ask, 100, 3),
            ],
        );

        assert_eq!(plan.fills.len(), 2);
        assert_eq!(plan.fills[0].resting_order_id, 1);
        assert_eq!(plan.fills[1].resting_order_id, 2);
        assert_eq!(plan.remaining_quantity, 2);
        assert_eq!(plan.buyer_refund, 0);
    }

    #[test]
    fn bid_price_improvement_is_refunded_and_resting_bid_escrow_is_consumed() {
        let plan = plan_matches(
            incoming(OrderSide::Bid, 120, 3),
            [resting(1, 1, OrderSide::Ask, 100, 3)],
        );

        assert_eq!(plan.buyer_refund, 60);
        assert_eq!(plan.fills[0].seller_proceeds, 300);

        let plan = plan_matches(
            incoming(OrderSide::Ask, 90, 2),
            [resting(1, 1, OrderSide::Bid, 100, 3)],
        );
        assert_eq!(plan.fills[0].resting_escrowed_currency, 100);
        assert_eq!(plan.buyer_refund, 0);
    }

    #[test]
    fn non_crossing_and_different_item_orders_are_ignored() {
        let mut other_item = resting(2, 2, OrderSide::Ask, 80, 5);
        other_item.item_id = ItemId::PackagedShip(dawn_core::ShipTypeId(7));

        let plan = plan_matches(
            incoming(OrderSide::Bid, 90, 5),
            [resting(1, 1, OrderSide::Ask, 100, 5), other_item],
        );

        assert!(plan.fills.is_empty());
        assert_eq!(plan.remaining_quantity, 5);
    }
}
