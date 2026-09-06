//! Market request handling for the WebSocket serve loops (ADR-0034 §4).
//!
//! This module is the request-facing bridge between the Market authority and
//! Sector cargo ownership. `dawn-market` decides matching and Currency; the
//! sibling `market_settlement` module owns the one-sided cargo handoff, while
//! this module validates wire input and sends bounded snapshots to the client.

use dawn_core::{EntityId, ItemId, PlayerId, ShipId};
#[cfg(test)]
use dawn_market::MarketCommand;
use dawn_market::{MarketDb, MarketOrderView, OrderId, OrderSide, SettlementId, SettlementIntent};
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
    /// Ephemeral delivery cursor. `MarketDb` never owns or advances it.
    settlement_cursor: Option<SettlementId>,
    /// Whether the delivery owner should query the outbox.
    ///
    /// An empty observation enables the no-work optimization. A nonempty page
    /// keeps polling even when a particular Sector queues nothing because all
    /// rows on that page are unroutable there.
    /// All settlement-producing ledger mutations pass through this runtime;
    /// external SQLite writers are not observed while delivery is idle.
    settlement_polling_enabled: bool,
}

impl MarketRuntime {
    pub(crate) fn open(path: &str) -> rusqlite::Result<Self> {
        Ok(Self {
            db: MarketDb::open(path)?,
            settlement_cursor: None,
            // A restart inherits whatever the durable outbox already holds.
            settlement_polling_enabled: true,
        })
    }

    #[cfg(test)]
    fn open_in_memory() -> Self {
        Self {
            db: MarketDb::open_in_memory().expect("in-memory Market DB"),
            settlement_cursor: None,
            settlement_polling_enabled: true,
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
        self.settlement_polling_enabled = true;
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
        self.settlement_polling_enabled = true;
        self.snapshot(player_id, result.notice())
    }

    fn cancel(&mut self, player_id: PlayerId, raw_order_id: u64) -> MarketSnapshotWire {
        let Some(order_id) = order_id_from_wire(raw_order_id) else {
            return self.snapshot(player_id, "Market order rejected");
        };
        let result = MarketSettlement::cancel(&mut self.db, player_id, order_id);
        self.settlement_polling_enabled = true;
        self.snapshot(player_id, result.notice())
    }

    fn scan_pending_settlement_page(&mut self) -> Vec<SettlementIntent> {
        if !self.settlement_polling_enabled {
            return Vec::new();
        }

        let page = match self
            .db
            .pending_settlements_page_after(self.settlement_cursor)
        {
            Ok(page) => page,
            Err(error) => {
                eprintln!("[Server] Market settlement scan failed: {error}");
                return Vec::new();
            }
        };

        if let Some(intent) = page.last() {
            self.settlement_cursor = Some(intent.id);
        } else {
            self.settlement_polling_enabled = false;
        }
        page
    }

    /// Drain pending Market settlement intents whose destination ship
    /// `node` currently owns/docks into inputs for this tick's `FrameInput`.
    /// The delivery owner scans one bounded page before routing it (issue #315).
    ///
    /// A nonempty but unroutable page advances the delivery cursor and remains
    /// eligible for polling on the next tick. An empty observation enables the
    /// no-work optimization; a read failure is retried on the next tick.
    pub(crate) fn drain_settlements<N: RuntimeNodeView>(
        &mut self,
        node: &N,
    ) -> Vec<QueuedSettlement> {
        let pending = self.scan_pending_settlement_page();
        MarketSettlement::queue_pending_inputs(&mut self.db, &pending, node.runtime_node())
    }

    /// Scan one bounded page once and route that same page to every Sector in
    /// stable caller order. The shared page is what makes clustered delivery
    /// independent of which Sector happens to be visited first.
    pub(crate) fn drain_cluster_settlements<N: RuntimeNodeView>(
        &mut self,
        nodes: &[N],
    ) -> Vec<Vec<QueuedSettlement>> {
        let pending = self.scan_pending_settlement_page();

        let mut routed = Vec::with_capacity(nodes.len());
        for node in nodes {
            routed.push(MarketSettlement::queue_pending_inputs(
                &mut self.db,
                &pending,
                node.runtime_node(),
            ));
        }
        routed
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
    use dawn_core::{ClientRequest, CreditItemCommand, NodeId, SectorBounds, SectorId, StationId};
    use dawn_market::SettlementStatus;
    use dawn_sector::transit::{LocalRuntimeConsensus, RuntimeTickOutput};
    use dawn_sector::transition::{FrameInput, MarketSettlementStatus};
    use dawn_server::runtime_frame::{
        OwnedRaftRuntimeConsensus, RuntimeFrameHost, RuntimeFramePolicy,
    };
    use dawn_storage::InMemoryJournal;

    fn docked_node(node_id: NodeId, player_id: PlayerId, scrap: u64) -> (SimulationNode, ShipId) {
        let mut node = SimulationNode::new(
            node_id,
            SectorId(node_id.0),
            SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            std::sync::Arc::new(dawn_sector::galaxy::Galaxy::demo()),
            crate::test_catalog(),
        );
        let station_id = StationId(u32::from(node_id.0));
        let position = node
            .station(station_id)
            .expect("local demo station")
            .position;
        let ship_id = node.spawn_player_ship_at_pub(player_id, position);
        node.apply_client_request(
            player_id,
            ClientRequest::Dock {
                station: station_id,
            },
            &mut Vec::new(),
        )
        .unwrap();
        assert!(node.credit_item_owned(CreditItemCommand {
            player_id,
            ship_id,
            item_id: ItemId::ScrapMetal,
            quantity: scrap,
            settlement_id: 1_000_000,
        }));
        let _ = node.drain_pending_events();
        (node, ship_id)
    }

    fn place_asks(db: &mut MarketDb, player_id: PlayerId, ship_id: ShipId, count: usize) {
        for _ in 0..count {
            db.execute(MarketCommand::PlaceOrder {
                player_id,
                ship_id,
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Ask,
                price: 100,
                quantity: 1,
            })
            .unwrap();
        }
    }

    fn commit_settlements(
        host: &mut RuntimeFrameHost<InMemoryJournal, LocalRuntimeConsensus>,
        queued: &[QueuedSettlement],
        acknowledged: &[u64],
    ) -> RuntimeTickOutput {
        let inputs: Vec<_> = queued.iter().map(QueuedSettlement::input).collect();
        host.run_frame(FrameInput {
            lock_commands: &[],
            authenticated_requests: &[],
            market_settlements: &inputs,
            acknowledged_settlements: acknowledged,
        })
        .unwrap()
    }

    fn assert_committed_scrap(output: &RuntimeTickOutput, ship_id: ShipId, quantity: u64) {
        let cargo = output.events.iter().rev().find_map(|event| match event {
            dawn_core::DomainEvent::ShipFitted(fitted) if fitted.ship_id == ship_id => Some(
                fitted
                    .inventory
                    .get(&ItemId::ScrapMetal)
                    .copied()
                    .unwrap_or(0),
            ),
            _ => None,
        });
        assert_eq!(cargo, Some(quantity), "a committed cargo event is required");
    }

    #[test]
    fn single_delivery_reaches_an_eligible_second_page_after_an_unroutable_page() {
        let mut market = MarketRuntime::open_in_memory();
        let player = PlayerId(1);
        let (node, ship) = docked_node(NodeId(0), player, 1_000);
        let mut host = RuntimeFrameHost::new(
            node,
            InMemoryJournal::new(),
            LocalRuntimeConsensus,
            RuntimeFramePolicy::local_durable(0),
        );
        place_asks(
            &mut market.db,
            PlayerId(9),
            ShipId::new(NodeId(9), 1),
            1_000,
        );
        place_asks(&mut market.db, player, ship, 1_000);

        assert!(market.drain_settlements(&host).is_empty());
        assert!(market.settlement_polling_enabled);
        assert_eq!(market.settlement_cursor, Some(SettlementId(1_000)));
        // Independent observation must not consume the delivery owner's page.
        assert_eq!(
            market
                .db
                .pending_settlements_page_after(None)
                .unwrap()
                .len(),
            1_000
        );
        let queued = market.drain_settlements(&host);
        assert_eq!(queued.len(), 1_000);
        assert_eq!(queued.first().unwrap().input().settlement_id(), 1_001);
        assert_eq!(queued.last().unwrap().input().settlement_id(), 2_000);

        let output = commit_settlements(&mut host, &queued, &[]);
        assert_eq!(output.tick_result.market_settlement_outcomes.len(), 1_000);
        assert!(output
            .tick_result
            .market_settlement_outcomes
            .iter()
            .all(|outcome| outcome.status == MarketSettlementStatus::Applied));
        assert_committed_scrap(&output, ship, 0);
        let ack =
            market.acknowledge_settlements(&queued, &output.tick_result.market_settlement_outcomes);
        assert_eq!(ack.decided_settlement_ids.len(), 1_000);
        assert_eq!(
            market
                .db
                .settlement(SettlementId(2_000))
                .unwrap()
                .unwrap()
                .status,
            SettlementStatus::Applied
        );
        assert_eq!(
            market
                .db
                .settlement(SettlementId(1))
                .unwrap()
                .unwrap()
                .status,
            SettlementStatus::Pending
        );
        assert!(market.drain_settlements(&host).is_empty());
        assert!(
            market.settlement_polling_enabled,
            "unowned rows still require polling"
        );
    }

    #[test]
    fn empty_observation_keeps_cursor_and_skips_reads_until_a_ledger_mutation() {
        let mut market = MarketRuntime::open_in_memory();
        let player = PlayerId(1);
        let (node, ship) = docked_node(NodeId(0), player, 5);
        place_asks(&mut market.db, player, ship, 1);
        let queued = market.drain_settlements(&node);
        let mut host = RuntimeFrameHost::new(
            node,
            InMemoryJournal::new(),
            LocalRuntimeConsensus,
            RuntimeFramePolicy::local_durable(0),
        );
        let output = commit_settlements(&mut host, &queued, &[]);
        market.acknowledge_settlements(&queued, &output.tick_result.market_settlement_outcomes);
        assert!(market.drain_settlements(&host).is_empty());
        assert!(!market.settlement_polling_enabled);
        assert_eq!(market.settlement_cursor, Some(SettlementId(1)));

        // Bypass the runtime only to prove idle delivery does not query SQL:
        // a read would observe this row. Production ledger writes wake polling.
        place_asks(&mut market.db, player, ship, 1);
        assert!(market.drain_settlements(&host).is_empty());
        assert_eq!(market.settlement_cursor, Some(SettlementId(1)));
        market.handle_single(
            player,
            MarketCommandWire::PlaceMarketOrderCommand {
                ship_id: ship.raw(),
                item_id: ItemWire::ScrapMetal,
                side: MarketOrderSide::Ask,
                price: 100,
                quantity: 1,
            },
            &host,
        );
        let queued = market.drain_settlements(&host);
        assert_eq!(
            queued
                .iter()
                .map(|q| q.input().settlement_id())
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        let output = commit_settlements(&mut host, &queued, &[]);
        market.acknowledge_settlements(&queued, &output.tick_result.market_settlement_outcomes);
        assert!(market.drain_settlements(&host).is_empty());
        assert!(!market.settlement_polling_enabled);

        market.handle_single(
            player,
            MarketCommandWire::CancelMarketOrderCommand { order_id: 1 },
            &host,
        );
        let returned = market.drain_settlements(&host);
        assert_eq!(
            returned.len(),
            1,
            "cancellation must wake idle delivery too"
        );
        let output = commit_settlements(&mut host, &returned, &[]);
        assert_committed_scrap(&output, ship, 3);
    }

    #[test]
    fn settlement_read_failure_preserves_cursor_and_retries_the_eligible_page() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("market.sqlite");
        let mut market = MarketRuntime::open(path.to_str().unwrap()).unwrap();
        let fault = rusqlite::Connection::open(&path).unwrap();
        let (node, ship) = docked_node(NodeId(0), PlayerId(1), 1);
        place_asks(
            &mut market.db,
            PlayerId(9),
            ShipId::new(NodeId(9), 1),
            1_000,
        );
        place_asks(&mut market.db, PlayerId(1), ship, 1);
        assert!(market.drain_settlements(&node).is_empty());
        assert_eq!(market.settlement_cursor, Some(SettlementId(1_000)));

        fault
            .execute(
                "ALTER TABLE settlements RENAME TO unavailable_settlements",
                [],
            )
            .unwrap();
        assert!(market.drain_settlements(&node).is_empty());
        assert!(market.drain_cluster_settlements(std::slice::from_ref(&node))[0].is_empty());
        assert!(market.settlement_polling_enabled);
        assert_eq!(market.settlement_cursor, Some(SettlementId(1_000)));
        fault
            .execute(
                "ALTER TABLE unavailable_settlements RENAME TO settlements",
                [],
            )
            .unwrap();

        let queued = market.drain_settlements(&node);
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].input().settlement_id(), 1_001);
    }

    #[test]
    fn failed_market_ack_retries_without_repeating_cargo_or_retiring_the_guard() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("market.sqlite");
        let mut market = MarketRuntime::open(path.to_str().unwrap()).unwrap();
        let fault = rusqlite::Connection::open(&path).unwrap();
        let player = PlayerId(1);
        let (node, ship) = docked_node(NodeId(0), player, 1);
        let mut host = RuntimeFrameHost::new(
            node,
            InMemoryJournal::new(),
            LocalRuntimeConsensus,
            RuntimeFramePolicy::local_durable(0),
        );
        // One applies and the second is refused for insufficient cargo.
        place_asks(&mut market.db, player, ship, 2);
        let queued = market.drain_settlements(&host);
        let output = commit_settlements(&mut host, &queued, &[]);
        assert_committed_scrap(&output, ship, 0);
        assert_eq!(
            output
                .tick_result
                .market_settlement_outcomes
                .iter()
                .map(|outcome| outcome.status)
                .collect::<Vec<_>>(),
            vec![
                MarketSettlementStatus::Applied,
                MarketSettlementStatus::Rejected
            ]
        );

        fault
            .execute_batch(
                "CREATE TRIGGER fail_settlement_ack BEFORE UPDATE ON settlements BEGIN
                SELECT RAISE(ABORT, 'injected ACK write failure');
             END;",
            )
            .unwrap();
        let ack =
            market.acknowledge_settlements(&queued, &output.tick_result.market_settlement_outcomes);
        assert!(ack.decided_settlement_ids.is_empty());
        assert!(
            ack.rejected_players.is_empty(),
            "a failed ledger decision cannot report compensation"
        );
        assert_eq!(
            market
                .db
                .pending_settlements_page_after(None)
                .unwrap()
                .len(),
            2
        );
        assert!(host
            .node()
            .take_snapshot_at(0)
            .node_state
            .applied_market_settlements
            .contains(&1));

        fault
            .execute_batch("DROP TRIGGER fail_settlement_ack")
            .unwrap();
        let retry = market.drain_settlements(&host);
        assert_eq!(
            retry
                .iter()
                .map(QueuedSettlement::input)
                .collect::<Vec<_>>(),
            queued
                .iter()
                .map(QueuedSettlement::input)
                .collect::<Vec<_>>()
        );
        let retried = commit_settlements(&mut host, &retry, &ack.decided_settlement_ids);
        assert_eq!(
            retried.tick_result.market_settlement_outcomes,
            output.tick_result.market_settlement_outcomes
        );
        assert!(
            !retried.events.iter().any(|event| matches!(event,
            dawn_core::DomainEvent::ShipFitted(fitted) if fitted.ship_id == ship)),
            "duplicate delivery must not emit a second cargo mutation"
        );
        let ack =
            market.acknowledge_settlements(&retry, &retried.tick_result.market_settlement_outcomes);
        assert_eq!(ack.decided_settlement_ids, vec![1, 2]);
        assert_eq!(ack.rejected_players, vec![player]);
        assert_eq!(
            market
                .db
                .settlement(SettlementId(1))
                .unwrap()
                .unwrap()
                .status,
            SettlementStatus::Applied
        );
        assert_eq!(
            market
                .db
                .settlement(SettlementId(2))
                .unwrap()
                .unwrap()
                .status,
            SettlementStatus::Terminal
        );
        assert!(host
            .node()
            .take_snapshot_at(0)
            .node_state
            .applied_market_settlements
            .contains(&1));
        commit_settlements(&mut host, &[], &ack.decided_settlement_ids);
        assert!(!host
            .node()
            .take_snapshot_at(0)
            .node_state
            .applied_market_settlements
            .contains(&1));
        assert!(market.drain_settlements(&host).is_empty());
    }

    #[test]
    fn settlement_waits_while_undocked_and_retries_after_a_committed_redock() {
        let mut market = MarketRuntime::open_in_memory();
        let player = PlayerId(1);
        let (node, ship) = docked_node(NodeId(0), player, 1);
        let mut host = RuntimeFrameHost::new(
            node,
            InMemoryJournal::new(),
            LocalRuntimeConsensus,
            RuntimeFramePolicy::local_durable(0),
        );
        place_asks(&mut market.db, player, ship, 1);
        let request = |request| dawn_sector::transition::AuthenticatedClientRequest {
            session_index: 0,
            player_id: player,
            request,
        };
        host.run_frame(FrameInput {
            authenticated_requests: &[request(ClientRequest::Undock)],
            ..FrameInput::lock_only(&[])
        })
        .unwrap();
        assert_eq!(host.node().docked_station(ship), None);
        assert!(market.drain_settlements(&host).is_empty());
        assert!(market.settlement_polling_enabled);
        assert_eq!(
            market
                .db
                .settlement(SettlementId(1))
                .unwrap()
                .unwrap()
                .status,
            SettlementStatus::Pending
        );

        host.run_frame(FrameInput {
            authenticated_requests: &[request(ClientRequest::Dock {
                station: StationId(0),
            })],
            ..FrameInput::lock_only(&[])
        })
        .unwrap();
        assert_eq!(host.node().docked_station(ship), Some(StationId(0)));
        let queued = market.drain_settlements(&host);
        assert_eq!(queued.len(), 1);
        let output = commit_settlements(&mut host, &queued, &[]);
        assert_committed_scrap(&output, ship, 0);
        assert_eq!(
            output.tick_result.market_settlement_outcomes[0].status,
            MarketSettlementStatus::Applied
        );
        let ack =
            market.acknowledge_settlements(&queued, &output.tick_result.market_settlement_outcomes);
        assert_eq!(ack.decided_settlement_ids, vec![1]);
    }

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

    #[tokio::test]
    async fn clustered_delivery_routes_one_shared_page_to_each_sector() {
        let ids = [NodeId(0), NodeId(1), NodeId(2)];
        let (endpoints, _partitioned) = crate::cluster::spawn_raft_actors(&ids);
        let mut hosts = Vec::new();
        let mut eligible_ships = Vec::new();

        for (&node_id, (raft, committed_rx)) in ids.iter().zip(endpoints) {
            let player_id = PlayerId(1_000 + u64::from(node_id.0));
            let (node, ship_id) = docked_node(node_id, player_id, 2);
            eligible_ships.push((player_id, ship_id));
            hosts.push(RuntimeFrameHost::new(
                node,
                InMemoryJournal::new(),
                OwnedRaftRuntimeConsensus::new(raft, committed_rx),
                RuntimeFramePolicy::local_durable(0),
            ));
        }

        let mut market = MarketRuntime::open_in_memory();
        for index in 1..=2_000_u64 {
            let (player_id, ship_id) = match index {
                1_500 => eligible_ships[1],
                2_000 => eligible_ships[2],
                _ => (PlayerId(10_000 + index), ShipId::new(NodeId(9), index)),
            };
            place_asks(&mut market.db, player_id, ship_id, 1);
        }

        let mut sessions = Vec::new();
        let mut player_sector = std::collections::HashMap::new();
        let ship_player = std::collections::HashMap::new();
        let mut aoi_delivery = super::super::AoiDelivery::new(super::super::AOI_CELL_SIZE);
        let mut decided_settlements = vec![Vec::new(); hosts.len()];
        let requests: Vec<Vec<dawn_sector::transition::AuthenticatedClientRequest>> =
            (0..hosts.len()).map(|_| Vec::new()).collect();

        let mut run_tick = |market: &mut MarketRuntime| {
            super::super::runtime::run_cluster_runtime_tick(
                super::super::runtime::ClusterRuntimeTickContext {
                    hosts: &mut hosts,
                    sessions: &mut sessions,
                    player_sector: &mut player_sector,
                    ship_player: &ship_player,
                    aoi_delivery: &mut aoi_delivery,
                    market,
                    decided_settlements: &mut decided_settlements,
                },
                &requests,
            )
        };
        let first_tick = run_tick(&mut market);
        assert!(first_tick
            .iter()
            .all(|output| output.tick_result.market_settlement_outcomes.is_empty()));

        let second_tick = run_tick(&mut market);
        assert!(second_tick[0]
            .tick_result
            .market_settlement_outcomes
            .is_empty());
        for (sector, id) in [(1, 1_500), (2, 2_000)] {
            assert_eq!(
                second_tick[sector].tick_result.market_settlement_outcomes,
                vec![MarketSettlementOutcome {
                    settlement_id: id,
                    status: MarketSettlementStatus::Applied
                }]
            );
            assert_committed_scrap(&second_tick[sector], eligible_ships[sector].1, 1);
            assert_eq!(
                market
                    .db
                    .settlement(SettlementId(id as i64))
                    .unwrap()
                    .unwrap()
                    .status,
                SettlementStatus::Applied
            );
        }

        // More backlog spans another boundary; each fixed-order owner must
        // receive its eligible work, including the first Sector in the loop.
        for index in 2_001..=3_001_u64 {
            let (player_id, ship_id) = match index {
                2_001 => eligible_ships[2],
                2_500 => eligible_ships[0],
                3_001 => eligible_ships[1],
                _ => (PlayerId(10_000 + index), ShipId::new(NodeId(9), index)),
            };
            place_asks(&mut market.db, player_id, ship_id, 1);
        }
        for expected in [vec![(0, 2_500, 1), (2, 2_001, 0)], vec![(1, 3_001, 0)]] {
            let outputs = run_tick(&mut market);
            assert_eq!(
                outputs
                    .iter()
                    .map(|o| o.tick_result.market_settlement_outcomes.len())
                    .sum::<usize>(),
                expected.len()
            );
            for (sector, id, remaining) in expected {
                assert_eq!(
                    outputs[sector].tick_result.market_settlement_outcomes,
                    vec![MarketSettlementOutcome {
                        settlement_id: id,
                        status: MarketSettlementStatus::Applied
                    }]
                );
                assert_committed_scrap(&outputs[sector], eligible_ships[sector].1, remaining);
                assert_eq!(
                    market
                        .db
                        .settlement(SettlementId(id as i64))
                        .unwrap()
                        .unwrap()
                        .status,
                    SettlementStatus::Applied
                );
            }
        }
        for id in [1, 1_000, 2_999] {
            assert_eq!(
                market
                    .db
                    .settlement(SettlementId(id))
                    .unwrap()
                    .unwrap()
                    .status,
                SettlementStatus::Pending,
                "unowned work remains pending"
            );
        }
    }
}
