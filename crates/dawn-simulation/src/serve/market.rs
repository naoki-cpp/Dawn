//! Market request handling for the WebSocket serve loops (ADR-0034 §4).
//!
//! This module is the runtime bridge between the Market authority and Sector
//! cargo ownership. `dawn-market` decides matching and Currency; this module
//! validates untrusted wire input, applies the one-sided cargo commands, and
//! sends a bounded snapshot back to the client.

use dawn_actor::protocol::{MarketCommandWire, MarketOrderWire, MarketSnapshotWire};
use dawn_core::{EntityId, ItemId, PlayerId, RemoveItemCommand, ReturnItemCommand, ShipId};
use dawn_market::{
    InsufficientBalance, MarketDb, MarketOrderView, OrderId, OrderSide, PlaceOrderOutcome,
};
use dawn_sector::node::SimulationNode;

const MAX_MARKET_ORDERS: usize = 200;

#[derive(Debug, Clone, Copy)]
struct ParsedOrder {
    ship_id: ShipId,
    item_id: ItemId,
    side: OrderSide,
    price: u64,
    quantity: u64,
}

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
        match command {
            MarketCommandWire::RefreshMarketCommand {} => self.snapshot(player_id, ""),
            MarketCommandWire::PlaceMarketOrderCommand {
                ship_id,
                item_type,
                module_id,
                ship_type_id,
                side,
                price,
                quantity,
            } => match parse_order(
                ship_id,
                &item_type,
                module_id,
                ship_type_id,
                &side,
                price,
                quantity,
            ) {
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
        nodes: &mut [SimulationNode],
    ) -> MarketSnapshotWire {
        match command {
            MarketCommandWire::RefreshMarketCommand {} => self.snapshot(player_id, ""),
            MarketCommandWire::PlaceMarketOrderCommand {
                ship_id,
                item_type,
                module_id,
                ship_type_id,
                side,
                price,
                quantity,
            } => match parse_order(
                ship_id,
                &item_type,
                module_id,
                ship_type_id,
                &side,
                price,
                quantity,
            ) {
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
        if !node.owns_ship(player_id, order.ship_id) {
            return self.snapshot(player_id, "Ship is not owned by this player");
        }

        let removed = if order.side == OrderSide::Ask {
            node.remove_item_owned(RemoveItemCommand {
                player_id,
                ship_id: order.ship_id,
                item_id: order.item_id,
                quantity: order.quantity,
            })
        } else {
            true
        };
        if !removed {
            return self.snapshot(player_id, "Item not available");
        }

        let result = self.db.place_order(
            player_id,
            order.ship_id,
            order.item_id,
            order.side,
            order.price,
            order.quantity,
        );
        match result {
            Ok(Ok(outcome)) => {
                let settlement_ok = apply_settlement_single(node, &outcome);
                if settlement_ok {
                    self.snapshot(player_id, "Order placed")
                } else {
                    self.snapshot(player_id, "Order placed; settlement needs attention")
                }
            }
            Ok(Err(InsufficientBalance)) => {
                restore_ask_single(node, order, player_id);
                self.snapshot(player_id, "Insufficient Currency")
            }
            Err(_) => {
                restore_ask_single(node, order, player_id);
                self.snapshot(player_id, "Market database error")
            }
        }
    }

    fn place_cluster(
        &mut self,
        player_id: PlayerId,
        order: ParsedOrder,
        nodes: &mut [SimulationNode],
    ) -> MarketSnapshotWire {
        if find_node(nodes, player_id, order.ship_id).is_none() {
            return self.snapshot(player_id, "Ship is not owned by this player");
        }

        let removed = if order.side == OrderSide::Ask {
            find_node(nodes, player_id, order.ship_id).is_some_and(|node| {
                node.remove_item_owned(RemoveItemCommand {
                    player_id,
                    ship_id: order.ship_id,
                    item_id: order.item_id,
                    quantity: order.quantity,
                })
            })
        } else {
            true
        };
        if !removed {
            return self.snapshot(player_id, "Item not available");
        }

        let result = self.db.place_order(
            player_id,
            order.ship_id,
            order.item_id,
            order.side,
            order.price,
            order.quantity,
        );
        match result {
            Ok(Ok(outcome)) => {
                let settlement_ok = apply_settlement_cluster(nodes, &outcome);
                if settlement_ok {
                    self.snapshot(player_id, "Order placed")
                } else {
                    self.snapshot(player_id, "Order placed; settlement needs attention")
                }
            }
            Ok(Err(InsufficientBalance)) => {
                restore_ask_cluster(nodes, order, player_id);
                self.snapshot(player_id, "Insufficient Currency")
            }
            Err(_) => {
                restore_ask_cluster(nodes, order, player_id);
                self.snapshot(player_id, "Market database error")
            }
        }
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
        match self.db.cancel_order(player_id, order_id) {
            Ok(Some(cancelled)) => {
                let returned = cancelled
                    .return_item_command
                    .is_none_or(|command| node.return_item_owned(command));
                if returned {
                    self.snapshot(player_id, "Order cancelled")
                } else {
                    self.snapshot(player_id, "Order cancelled; item return needs attention")
                }
            }
            Ok(None) => self.snapshot(player_id, "Order not found"),
            Err(_) => self.snapshot(player_id, "Market database error"),
        }
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
        match self.db.cancel_order(player_id, order_id) {
            Ok(Some(cancelled)) => {
                let returned = cancelled.return_item_command.is_none_or(|command| {
                    find_node(nodes, command.player_id, command.ship_id)
                        .is_some_and(|node| node.return_item_owned(command))
                });
                if returned {
                    self.snapshot(player_id, "Order cancelled")
                } else {
                    self.snapshot(player_id, "Order cancelled; item return needs attention")
                }
            }
            Ok(None) => self.snapshot(player_id, "Order not found"),
            Err(_) => self.snapshot(player_id, "Market database error"),
        }
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
}

fn parse_order(
    raw_ship_id: u64,
    item_type: &str,
    module_id: u32,
    ship_type_id: u32,
    side: &str,
    price: u64,
    quantity: u64,
) -> Option<ParsedOrder> {
    if price == 0 || quantity == 0 || price.checked_mul(quantity).is_none() {
        return None;
    }
    let item_id = match item_type {
        "Module" => ItemId::Module(dawn_core::ModuleId(module_id)),
        "PackagedShip" => ItemId::PackagedShip(dawn_core::ShipTypeId(ship_type_id)),
        "ScrapMetal" => ItemId::ScrapMetal,
        _ => return None,
    };
    let order_side = match side {
        "Bid" => OrderSide::Bid,
        "Ask" => OrderSide::Ask,
        _ => return None,
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
    let (item_type, module_id, ship_type_id) = match order.item_id {
        ItemId::Module(module_id) => ("Module", module_id.0, 0),
        ItemId::PackagedShip(ship_type_id) => ("PackagedShip", 0, ship_type_id.0),
        ItemId::ScrapMetal => ("ScrapMetal", 0, 0),
    };
    let order_id = u64::try_from(order.order_id.0).ok()?;
    Some(MarketOrderWire {
        order_id,
        item_type: item_type.to_owned(),
        module_id,
        ship_type_id,
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

fn apply_settlement_single(node: &mut SimulationNode, outcome: &PlaceOrderOutcome) -> bool {
    outcome
        .credit_item_commands
        .iter()
        .all(|command| node.credit_item_owned(*command))
}

fn apply_settlement_cluster(nodes: &mut [SimulationNode], outcome: &PlaceOrderOutcome) -> bool {
    outcome.credit_item_commands.iter().all(|command| {
        find_node(nodes, command.player_id, command.ship_id)
            .is_some_and(|node| node.credit_item_owned(*command))
    })
}

fn restore_ask_single(node: &mut SimulationNode, order: ParsedOrder, player_id: PlayerId) {
    if order.side == OrderSide::Ask {
        let _ = node.return_item_owned(ReturnItemCommand {
            player_id,
            ship_id: order.ship_id,
            item_id: order.item_id,
            quantity: order.quantity,
        });
    }
}

fn restore_ask_cluster(nodes: &mut [SimulationNode], order: ParsedOrder, player_id: PlayerId) {
    if order.side == OrderSide::Ask {
        if let Some(node) = find_node(nodes, player_id, order.ship_id) {
            let _ = node.return_item_owned(ReturnItemCommand {
                player_id,
                ship_id: order.ship_id,
                item_id: order.item_id,
                quantity: order.quantity,
            });
        }
    }
}

fn find_node(
    nodes: &mut [SimulationNode],
    player_id: PlayerId,
    ship_id: ShipId,
) -> Option<&mut SimulationNode> {
    nodes
        .iter_mut()
        .find(|node| node.owns_ship(player_id, ship_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_validation_rejects_zero_and_overflowing_values() {
        assert!(parse_order(1, "ScrapMetal", 0, 0, "Ask", 0, 1).is_none());
        assert!(parse_order(1, "ScrapMetal", 0, 0, "Ask", 1, 0).is_none());
        assert!(parse_order(1, "ScrapMetal", 0, 0, "Ask", u64::MAX, 2).is_none());
        assert!(parse_order(1, "Unknown", 0, 0, "Ask", 1, 1).is_none());
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
}
