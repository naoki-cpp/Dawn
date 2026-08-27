//! Market request handling for the WebSocket serve loops (ADR-0034 §4).
//!
//! This module is the request-facing bridge between the Market authority and
//! Sector cargo ownership. `dawn-market` decides matching and Currency; the
//! sibling `market_settlement` module owns the one-sided cargo handoff, while
//! this module validates wire input and sends bounded snapshots to the client.

use dawn_core::{EntityId, ItemId, PlayerId, ShipId};
#[cfg(test)]
use dawn_market::MarketCommand;
use dawn_market::{MarketDb, MarketOrderView, OrderId, OrderSide};
use dawn_protocol::{
    ItemWire, MarketCommandWire, MarketOrderSide, MarketOrderWire, MarketSnapshotWire,
};
use dawn_sector::node::SimulationNode;

use super::market_settlement::{
    MarketSettlement, ParsedOrder, QueuedSettlement, SettlementAcknowledgement,
};
use dawn_sector::transition::MarketSettlementOutcome;
use dawn_server::runtime_frame::RuntimeNodeView;

const MAX_MARKET_ORDERS: usize = 200;
/// SQLite persists Market numeric fields as signed 64-bit INTEGER values.
/// Reject larger wire values before they enter the Market transition.
const MAX_MARKET_ORDER_VALUE: u64 = i64::MAX as u64;
const MARKET_DOCK_REQUIRED_NOTICE: &str = "Dock at a station to use the Market";

/// Owns the persistent Market database for one serve process.
pub(crate) struct MarketRuntime {
    db: MarketDb,
    /// Whether the settlement outbox may hold work for a Sector to drain.
    ///
    /// `drain_settlements` runs on every tick of the serve loop, and its
    /// first act is a SQLite query. Ledger mutations are the only thing that
    /// can enqueue a settlement, so an idle Market can skip the query
    /// entirely instead of paying it 10x/sec/Sector forever. It stays set
    /// while any settlement is still pending (e.g. its ship is elsewhere),
    /// so a deferred one is never stranded.
    settlements_possible: bool,
}

impl MarketRuntime {
    pub(crate) fn open(path: &str) -> rusqlite::Result<Self> {
        Ok(Self {
            db: MarketDb::open(path)?,
            // A restart inherits whatever the durable outbox already holds.
            settlements_possible: true,
        })
    }

    #[cfg(test)]
    fn open_in_memory() -> Self {
        Self {
            db: MarketDb::open_in_memory().expect("in-memory Market DB"),
            settlements_possible: true,
        }
    }

    pub(crate) fn handle_single<N: RuntimeNodeView>(
        &mut self,
        player_id: PlayerId,
        command: MarketCommandWire,
        node: &N,
    ) -> MarketSnapshotWire {
        if node
            .runtime_node()
            .player_docked_station(player_id)
            .is_none()
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
                Some(order) => self.place_single(player_id, order, node),
                None => self.snapshot(player_id, "Market order rejected"),
            },
            MarketCommandWire::CancelMarketOrderCommand { order_id } => {
                self.cancel(player_id, order_id)
            }
        }
    }

    pub(crate) fn handle_cluster<N: RuntimeNodeView>(
        &mut self,
        player_id: PlayerId,
        command: MarketCommandWire,
        player_sector: usize,
        nodes: &[N],
    ) -> MarketSnapshotWire {
        if nodes.get(player_sector).is_none_or(|node| {
            node.runtime_node()
                .player_docked_station(player_id)
                .is_none()
        }) {
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
                Some(order) => self.place_cluster(player_id, order, player_sector, nodes),
                None => self.snapshot(player_id, "Market order rejected"),
            },
            MarketCommandWire::CancelMarketOrderCommand { order_id } => {
                self.cancel(player_id, order_id)
            }
        }
    }

    fn place_single<N: RuntimeNodeView>(
        &mut self,
        player_id: PlayerId,
        order: ParsedOrder,
        node: &N,
    ) -> MarketSnapshotWire {
        if !can_place_order(node.runtime_node(), player_id, order.ship_id) {
            return self.snapshot(player_id, "Market order rejected");
        }
        let result = MarketSettlement::place(&mut self.db, player_id, order);
        self.settlements_possible = true;
        self.snapshot(player_id, result.notice())
    }

    fn place_cluster<N: RuntimeNodeView>(
        &mut self,
        player_id: PlayerId,
        order: ParsedOrder,
        player_sector: usize,
        nodes: &[N],
    ) -> MarketSnapshotWire {
        if !nodes
            .get(player_sector)
            .is_some_and(|node| can_place_order(node.runtime_node(), player_id, order.ship_id))
        {
            return self.snapshot(player_id, "Market order rejected");
        }
        let result = MarketSettlement::place(&mut self.db, player_id, order);
        self.settlements_possible = true;
        self.snapshot(player_id, result.notice())
    }

    fn cancel(&mut self, player_id: PlayerId, raw_order_id: u64) -> MarketSnapshotWire {
        let Some(order_id) = order_id_from_wire(raw_order_id) else {
            return self.snapshot(player_id, "Market order rejected");
        };
        let result = MarketSettlement::cancel(&mut self.db, player_id, order_id);
        self.settlements_possible = true;
        self.snapshot(player_id, result.notice())
    }

    /// Drain pending Market settlement intents whose destination ship
    /// `node` currently owns/docks into inputs for this tick's `FrameInput`
    /// (issue #315). Call once per tick, per Sector, before `run_frame`.
    ///
    /// Returns immediately without touching SQLite when no ledger mutation
    /// has happened since the outbox was last observed empty.
    pub(crate) fn drain_settlements<N: RuntimeNodeView>(
        &mut self,
        node: &N,
    ) -> Vec<QueuedSettlement> {
        if !self.settlements_possible {
            return Vec::new();
        }
        let queued = MarketSettlement::drain_pending_inputs(&mut self.db, node.runtime_node());
        // Only stand down once the outbox is genuinely empty. A settlement
        // this Sector skipped (its ship is elsewhere) is still pending, so
        // keep polling until some Sector takes it.
        if queued.is_empty() && !MarketSettlement::has_pending(&self.db) {
            self.settlements_possible = false;
        }
        queued
    }

    /// Report a committed tick's `MarketSettlementOutcome`s back to the
    /// Market ledger for the settlements this Sector drained into it.
    ///
    /// Returns the identities the ledger durably decided -- feed them into
    /// the next frame's `FrameInput::acknowledged_settlements` so the Sector
    /// retires them from its idempotency guard -- plus the players whose
    /// settlement was refused, so the caller can tell them.
    pub(crate) fn acknowledge_settlements(
        &mut self,
        queued: &[QueuedSettlement],
        outcomes: &[MarketSettlementOutcome],
    ) -> SettlementAcknowledgement {
        MarketSettlement::acknowledge_outcomes(&mut self.db, queued, outcomes)
    }

    /// A fresh snapshot carrying a deferred settlement outcome's notice, for
    /// pushing to a player whose settlement resolved after their request had
    /// already been answered with "settlement pending".
    pub(crate) fn settlement_rejected_snapshot(&self, player_id: PlayerId) -> MarketSnapshotWire {
        self.snapshot(player_id, "Market settlement failed; order compensated")
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
    if price == 0
        || quantity == 0
        || price > MAX_MARKET_ORDER_VALUE
        || quantity > MAX_MARKET_ORDER_VALUE
        || price.checked_mul(quantity).is_none()
    {
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

fn can_place_order(node: &SimulationNode, player_id: PlayerId, ship_id: ShipId) -> bool {
    let Some(player_station) = node.player_docked_station(player_id) else {
        return false;
    };
    node.owns_ship(player_id, ship_id) && node.docked_station(ship_id) == Some(player_station)
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
            ItemWire::ScrapMetal,
            MarketOrderSide::Ask,
            1,
            MAX_MARKET_ORDER_VALUE + 1
        )
        .is_none());
        assert!(parse_order(
            1,
            ItemWire::ScrapMetal,
            MarketOrderSide::Ask,
            MAX_MARKET_ORDER_VALUE + 1,
            1
        )
        .is_none());
        assert!(parse_order(
            1,
            ItemWire::ScrapMetal,
            MarketOrderSide::Ask,
            MAX_MARKET_ORDER_VALUE,
            1
        )
        .is_some());
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
        runtime.db.credit_currency(PlayerId(1), 1000).unwrap();
        runtime
            .db
            .execute(MarketCommand::PlaceOrder {
                player_id: PlayerId(1),
                ship_id: ShipId(EntityId::from_raw(1)),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 100,
                quantity: 2,
            })
            .unwrap();

        let snapshot = runtime.snapshot(PlayerId(1), "");
        assert_eq!(snapshot.orders.len(), 1);
        assert!(snapshot.orders[0].is_own);
    }

    #[test]
    fn market_requests_are_rejected_when_the_player_is_not_docked() {
        let mut runtime = MarketRuntime::open_in_memory();
        let node = SimulationNode::new(
            dawn_core::NodeId(0),
            dawn_core::SectorId(0),
            dawn_core::SectorBounds::centered(dawn_core::SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );

        let snapshot = runtime.handle_single(
            PlayerId(1),
            MarketCommandWire::RefreshMarketCommand {},
            &node,
        );

        assert_eq!(snapshot.notice, MARKET_DOCK_REQUIRED_NOTICE);
        assert_eq!(snapshot.balance, 0);
        assert!(snapshot.orders.is_empty());
    }

    #[test]
    fn market_order_rejects_a_ship_not_owned_by_the_player() {
        let mut runtime = MarketRuntime::open_in_memory();
        runtime.db.credit_currency(PlayerId(1), 100).unwrap();
        let mut node = SimulationNode::new(
            dawn_core::NodeId(0),
            dawn_core::SectorId(0),
            dawn_core::SectorBounds::centered(dawn_core::SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );
        let foreign_ship = node.spawn_player_ship_at_pub(PlayerId(2), dawn_core::Position::ORIGIN);

        let result = runtime.place_single(
            PlayerId(1),
            ParsedOrder {
                ship_id: foreign_ship,
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 100,
                quantity: 1,
            },
            &node,
        );

        assert_eq!(result.notice, "Market order rejected");
        assert!(runtime.db.open_orders_for(PlayerId(1)).unwrap().is_empty());
        assert_eq!(runtime.db.currency_balance(PlayerId(1)).unwrap(), 100);
    }

    #[test]
    fn market_order_rejects_an_undocked_owned_ship() {
        let mut runtime = MarketRuntime::open_in_memory();
        runtime.db.credit_currency(PlayerId(1), 100).unwrap();
        let mut node = SimulationNode::new(
            dawn_core::NodeId(0),
            dawn_core::SectorId(0),
            dawn_core::SectorBounds::centered(dawn_core::SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );
        let station_position = node
            .station(dawn_core::StationId(0))
            .expect("demo station exists")
            .position;
        let _docked_ship = node.spawn_player_ship_at_pub(PlayerId(1), station_position);
        node.apply_client_request(
            PlayerId(1),
            dawn_core::ClientRequest::Dock {
                station: dawn_core::StationId(0),
            },
            &mut Vec::new(),
        )
        .unwrap();
        let ship_id = node.spawn_player_ship_at_pub(PlayerId(1), dawn_core::Position::ORIGIN);

        let result = runtime.place_single(
            PlayerId(1),
            ParsedOrder {
                ship_id,
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 100,
                quantity: 1,
            },
            &node,
        );

        assert_eq!(result.notice, "Market order rejected");
        assert!(runtime.db.open_orders_for(PlayerId(1)).unwrap().is_empty());
        assert_eq!(runtime.db.currency_balance(PlayerId(1)).unwrap(), 100);
    }
}
