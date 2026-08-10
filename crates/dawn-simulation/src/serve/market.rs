//! Market request handling for the WebSocket serve loops (ADR-0034 §4).
//!
//! This module is the request-facing bridge between the Market authority and
//! Sector cargo ownership. `dawn-market` decides matching and Currency; the
//! sibling `market_settlement` module owns the one-sided cargo handoff, while
//! this module validates wire input and sends bounded snapshots to the client.

use dawn_core::{EntityId, ItemId, PlayerId, ShipId};
use dawn_market::{MarketDb, MarketOrderView, OrderId, OrderSide};
use dawn_sector::node::SimulationNode;
use dawn_wire::{
    ItemWire, MarketCommandWire, MarketOrderSide, MarketOrderWire, MarketSnapshotWire,
};

use super::market_settlement::{MarketSettlement, ParsedOrder};

const MAX_MARKET_ORDERS: usize = 200;
const MARKET_DOCK_REQUIRED_NOTICE: &str = "Dock at a station to use the Market";

/// Owns the persistent Market database for one serve process.
pub(crate) struct MarketRuntime {
    db: MarketDb,
}

impl MarketRuntime {
    pub(crate) fn open(path: &str) -> rusqlite::Result<Self> {
        Ok(Self {
            db: MarketDb::open(path)?,
        })
    }

    #[cfg(test)]
    fn open_in_memory() -> Self {
        Self {
            db: MarketDb::open_in_memory().expect("in-memory Market DB"),
        }
    }

    pub(crate) fn handle_single(
        &mut self,
        player_id: PlayerId,
        command: MarketCommandWire,
        node: &mut SimulationNode,
    ) -> MarketSnapshotWire {
        if node.player_docked_station(player_id).is_none() {
            return Self::market_unavailable_snapshot();
        }
        match command {
            MarketCommandWire::RefreshMarketCommand {} => self.snapshot(player_id, ""),
            MarketCommandWire::PlaceMarketOrderCommand {
                ship_id,
                item_id,
                side,
                price,
                quantity,
            } => match parse_order(ship_id, item_id, side, price, quantity) {
                Some(order) => self.place_single(player_id, order, node),
                None => self.snapshot(player_id, "Market order rejected"),
            },
            MarketCommandWire::CancelMarketOrderCommand { order_id } => {
                self.cancel_single(player_id, order_id, node)
            }
        }
    }

    pub(crate) fn handle_cluster(
        &mut self,
        player_id: PlayerId,
        command: MarketCommandWire,
        player_sector: usize,
        nodes: &mut [SimulationNode],
    ) -> MarketSnapshotWire {
        if nodes
            .get(player_sector)
            .is_none_or(|node| node.player_docked_station(player_id).is_none())
        {
            return Self::market_unavailable_snapshot();
        }
        match command {
            MarketCommandWire::RefreshMarketCommand {} => self.snapshot(player_id, ""),
            MarketCommandWire::PlaceMarketOrderCommand {
                ship_id,
                item_id,
                side,
                price,
                quantity,
            } => match parse_order(ship_id, item_id, side, price, quantity) {
                Some(order) => self.place_cluster(player_id, order, nodes),
                None => self.snapshot(player_id, "Market order rejected"),
            },
            MarketCommandWire::CancelMarketOrderCommand { order_id } => {
                self.cancel_cluster(player_id, order_id, nodes)
            }
        }
    }

    fn place_single(
        &mut self,
        player_id: PlayerId,
        order: ParsedOrder,
        node: &mut SimulationNode,
    ) -> MarketSnapshotWire {
        let result = MarketSettlement::place_single(&mut self.db, player_id, order, node);
        self.snapshot(player_id, result.notice())
    }

    fn place_cluster(
        &mut self,
        player_id: PlayerId,
        order: ParsedOrder,
        nodes: &mut [SimulationNode],
    ) -> MarketSnapshotWire {
        let result = MarketSettlement::place_cluster(&mut self.db, player_id, order, nodes);
        self.snapshot(player_id, result.notice())
    }

    fn cancel_single(
        &mut self,
        player_id: PlayerId,
        raw_order_id: u64,
        node: &mut SimulationNode,
    ) -> MarketSnapshotWire {
        let Some(order_id) = order_id_from_wire(raw_order_id) else {
            return self.snapshot(player_id, "Market order rejected");
        };
        let result = MarketSettlement::cancel_single(&mut self.db, player_id, order_id, node);
        self.snapshot(player_id, result.notice())
    }

    fn cancel_cluster(
        &mut self,
        player_id: PlayerId,
        raw_order_id: u64,
        nodes: &mut [SimulationNode],
    ) -> MarketSnapshotWire {
        let Some(order_id) = order_id_from_wire(raw_order_id) else {
            return self.snapshot(player_id, "Market order rejected");
        };
        let result = MarketSettlement::cancel_cluster(&mut self.db, player_id, order_id, nodes);
        self.snapshot(player_id, result.notice())
    }

    fn snapshot(&self, player_id: PlayerId, notice: &str) -> MarketSnapshotWire {
        let balance = self.db.currency_balance(player_id).unwrap_or(0);
        let orders = self
            .db
            .open_orders_for(player_id)
            .map(|orders| {
                orders
                    .into_iter()
                    .take(MAX_MARKET_ORDERS)
                    .filter_map(|order| market_order_wire(order, player_id))
                    .collect()
            })
            .unwrap_or_default();
        MarketSnapshotWire {
            balance,
            orders,
            notice: notice.to_owned(),
        }
    }

    fn market_unavailable_snapshot() -> MarketSnapshotWire {
        MarketSnapshotWire {
            balance: 0,
            orders: Vec::new(),
            notice: MARKET_DOCK_REQUIRED_NOTICE.to_owned(),
        }
    }
}

fn parse_order(
    raw_ship_id: u64,
    item_id: ItemWire,
    side: MarketOrderSide,
    price: u64,
    quantity: u64,
) -> Option<ParsedOrder> {
    if price == 0 || quantity == 0 || price.checked_mul(quantity).is_none() {
        return None;
    }
    let item_id = ItemId::try_from(item_id).ok()?;
    let order_side = match side {
        MarketOrderSide::Bid => OrderSide::Bid,
        MarketOrderSide::Ask => OrderSide::Ask,
    };
    Some(ParsedOrder {
        ship_id: ShipId(EntityId::from_raw(raw_ship_id)),
        item_id,
        side: order_side,
        price,
        quantity,
    })
}

fn order_id_from_wire(raw_order_id: u64) -> Option<OrderId> {
    i64::try_from(raw_order_id).ok().map(OrderId)
}

fn market_order_wire(order: MarketOrderView, player_id: PlayerId) -> Option<MarketOrderWire> {
    let order_id = u64::try_from(order.order_id.0).ok()?;
    Some(MarketOrderWire {
        order_id,
        item_id: order.item_id.into(),
        side: match order.side {
            OrderSide::Bid => "Bid",
            OrderSide::Ask => "Ask",
        }
        .to_owned(),
        price: order.price,
        quantity: order.quantity_remaining,
        is_own: order.player_id == player_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_validation_rejects_zero_and_overflowing_values() {
        assert!(parse_order(1, ItemWire::ScrapMetal, MarketOrderSide::Ask, 0, 1).is_none());
        assert!(parse_order(1, ItemWire::ScrapMetal, MarketOrderSide::Ask, 1, 0).is_none());
        assert!(parse_order(1, ItemWire::ScrapMetal, MarketOrderSide::Ask, u64::MAX, 2).is_none());
        assert!(parse_order(
            1,
            ItemWire::Module { module_id: 0 },
            MarketOrderSide::Ask,
            1,
            1
        )
        .is_none());
    }

    #[test]
    fn snapshot_is_bounded_and_marks_the_callers_orders() {
        let mut runtime = MarketRuntime::open_in_memory();
        runtime
            .db
            .place_order(
                PlayerId(1),
                ShipId(EntityId::from_raw(1)),
                ItemId::ScrapMetal,
                OrderSide::Ask,
                100,
                2,
            )
            .unwrap()
            .unwrap();

        let snapshot = runtime.snapshot(PlayerId(1), "");
        assert_eq!(snapshot.orders.len(), 1);
        assert!(snapshot.orders[0].is_own);
    }

    #[test]
    fn market_requests_are_rejected_when_the_player_is_not_docked() {
        let mut runtime = MarketRuntime::open_in_memory();
        let mut node = SimulationNode::new(
            dawn_core::NodeId(0),
            dawn_core::SectorId(0),
            dawn_core::SectorBounds::centered(dawn_core::SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );

        let snapshot = runtime.handle_single(
            PlayerId(1),
            MarketCommandWire::RefreshMarketCommand {},
            &mut node,
        );

        assert_eq!(snapshot.notice, MARKET_DOCK_REQUIRED_NOTICE);
        assert_eq!(snapshot.balance, 0);
        assert!(snapshot.orders.is_empty());
    }
}
