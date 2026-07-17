//! Execution seam for Market-to-Sector item settlement.
//!
//! `dawn-market` owns matching and Currency, while `dawn-sector` owns the
//! authoritative cargo. This module keeps the handoff order and its
//! compensation rules in one place. Single-node and cluster serving are two
//! concrete adapters over the same settlement interface.
//!
//! The SQLite transaction and the Sector event store are still separate
//! authorities, so this module does not claim cross-store atomicity. It
//! attempts to prevent a failed multi-buyer credit from leaving a partial
//! cargo mutation, and reports the remaining crash window explicitly to the
//! caller.

use dawn_core::{
    CreditItemCommand, ItemId, PlayerId, RemoveItemCommand, ReturnItemCommand, ShipId,
};
use dawn_market::{InsufficientBalance, MarketDb, OrderId, OrderSide, PlaceOrderOutcome};
use dawn_sector::node::SimulationNode;

/// Result visible to the serve loop after the Market database and Sector
/// cargo handoff have been attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MarketSettlementResult {
    Completed(&'static str),
    Rejected(&'static str),
    NeedsAttention(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ParsedOrder {
    pub(super) ship_id: ShipId,
    pub(super) item_id: ItemId,
    pub(super) side: OrderSide,
    pub(super) price: u64,
    pub(super) quantity: u64,
}

impl MarketSettlementResult {
    pub(super) fn notice(self) -> &'static str {
        match self {
            Self::Completed(notice) => notice,
            Self::Rejected(notice) | Self::NeedsAttention(notice) => notice,
        }
    }
}

/// The small interface the settlement algorithm needs from a Sector runtime.
///
/// The production adapters below differ only in how they find the owning
/// `SimulationNode`; the order of cargo mutation and compensation is shared.
trait SettlementTarget {
    fn owns_ship(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool;
    fn remove_item(&mut self, command: RemoveItemCommand) -> bool;
    fn return_item(&mut self, command: ReturnItemCommand) -> bool;
    fn credit_item(&mut self, command: CreditItemCommand) -> bool;
}

struct SingleTarget<'a> {
    node: &'a mut SimulationNode,
}

struct ClusterTarget<'a> {
    nodes: &'a mut [SimulationNode],
}

impl SettlementTarget for SingleTarget<'_> {
    fn owns_ship(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        self.node.owns_ship(player_id, ship_id)
    }

    fn remove_item(&mut self, command: RemoveItemCommand) -> bool {
        self.node.remove_item_owned(command)
    }

    fn return_item(&mut self, command: ReturnItemCommand) -> bool {
        self.node.return_item_owned(command)
    }

    fn credit_item(&mut self, command: CreditItemCommand) -> bool {
        self.node.credit_item_owned(command)
    }
}

impl SettlementTarget for ClusterTarget<'_> {
    fn owns_ship(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
        find_node(self.nodes, player_id, ship_id).is_some()
    }

    fn remove_item(&mut self, command: RemoveItemCommand) -> bool {
        find_node(self.nodes, command.player_id, command.ship_id)
            .is_some_and(|node| node.remove_item_owned(command))
    }

    fn return_item(&mut self, command: ReturnItemCommand) -> bool {
        find_node(self.nodes, command.player_id, command.ship_id)
            .is_some_and(|node| node.return_item_owned(command))
    }

    fn credit_item(&mut self, command: CreditItemCommand) -> bool {
        find_node(self.nodes, command.player_id, command.ship_id)
            .is_some_and(|node| node.credit_item_owned(command))
    }
}

pub(super) struct MarketSettlement;

impl MarketSettlement {
    pub(super) fn place_single(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
        node: &mut SimulationNode,
    ) -> MarketSettlementResult {
        let mut target = SingleTarget { node };
        Self::place(db, player_id, order, &mut target)
    }

    pub(super) fn place_cluster(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
        nodes: &mut [SimulationNode],
    ) -> MarketSettlementResult {
        let mut target = ClusterTarget { nodes };
        Self::place(db, player_id, order, &mut target)
    }

    pub(super) fn cancel_single(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
        node: &mut SimulationNode,
    ) -> MarketSettlementResult {
        let mut target = SingleTarget { node };
        Self::cancel(db, player_id, order_id, &mut target)
    }

    pub(super) fn cancel_cluster(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
        nodes: &mut [SimulationNode],
    ) -> MarketSettlementResult {
        let mut target = ClusterTarget { nodes };
        Self::cancel(db, player_id, order_id, &mut target)
    }

    fn place<T: SettlementTarget>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
        target: &mut T,
    ) -> MarketSettlementResult {
        if !target.owns_ship(player_id, order.ship_id) {
            return MarketSettlementResult::Rejected("Ship is not owned by this player");
        }

        // Ask cargo is reserved before the Market transaction. A database
        // rejection can therefore return it without ever exposing an Ask
        // whose source cargo was not removed.
        let removed = if order.side == OrderSide::Ask {
            target.remove_item(RemoveItemCommand {
                player_id,
                ship_id: order.ship_id,
                item_id: order.item_id,
                quantity: order.quantity,
            })
        } else {
            true
        };
        if !removed {
            return MarketSettlementResult::Rejected("Item not available");
        }

        match db.place_order(
            player_id,
            order.ship_id,
            order.item_id,
            order.side,
            order.price,
            order.quantity,
        ) {
            Ok(Ok(outcome)) => {
                if apply_credit_commands(target, &outcome) {
                    MarketSettlementResult::Completed("Order placed")
                } else {
                    MarketSettlementResult::NeedsAttention(
                        "Order placed; settlement needs attention",
                    )
                }
            }
            Ok(Err(InsufficientBalance)) => {
                let returned = restore_ask(target, order, player_id);
                if returned {
                    MarketSettlementResult::Rejected("Insufficient Currency")
                } else {
                    MarketSettlementResult::NeedsAttention(
                        "Insufficient Currency; item return needs attention",
                    )
                }
            }
            Err(_) => {
                let returned = restore_ask(target, order, player_id);
                if returned {
                    MarketSettlementResult::Rejected("Market database error")
                } else {
                    MarketSettlementResult::NeedsAttention(
                        "Market database error; item return needs attention",
                    )
                }
            }
        }
    }

    fn cancel<T: SettlementTarget>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
        target: &mut T,
    ) -> MarketSettlementResult {
        match db.cancel_order(player_id, order_id) {
            Ok(Some(cancelled)) => {
                let returned = cancelled
                    .return_item_command
                    .is_none_or(|command| target.return_item(command));
                if returned {
                    MarketSettlementResult::Completed("Order cancelled")
                } else {
                    MarketSettlementResult::NeedsAttention(
                        "Order cancelled; item return needs attention",
                    )
                }
            }
            Ok(None) => MarketSettlementResult::Rejected("Order not found"),
            Err(_) => MarketSettlementResult::Rejected("Market database error"),
        }
    }
}

/// Apply all buyer credits as one local handoff. If a later destination is
/// unavailable, remove the credits already applied so Sector cargo does not
/// remain partially settled. The Market transaction is already committed at
/// this point; the caller must surface the returned attention state if either
/// the credit or its compensation fails.
fn apply_credit_commands<T: SettlementTarget>(target: &mut T, outcome: &PlaceOrderOutcome) -> bool {
    let mut applied = Vec::new();
    for command in &outcome.credit_item_commands {
        if target.credit_item(*command) {
            applied.push(*command);
            continue;
        }

        for applied_command in applied.into_iter().rev() {
            let _ = target.remove_item(RemoveItemCommand {
                player_id: applied_command.player_id,
                ship_id: applied_command.ship_id,
                item_id: applied_command.item_id,
                quantity: applied_command.quantity,
            });
        }
        return false;
    }
    true
}

fn restore_ask<T: SettlementTarget>(
    target: &mut T,
    order: ParsedOrder,
    player_id: PlayerId,
) -> bool {
    if order.side != OrderSide::Ask {
        return true;
    }
    target.return_item(ReturnItemCommand {
        player_id,
        ship_id: order.ship_id,
        item_id: order.item_id,
        quantity: order.quantity,
    })
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
    use dawn_core::NodeId;

    fn ship(raw: u64) -> ShipId {
        ShipId::new(NodeId(0), raw)
    }

    fn order(ship_id: ShipId, side: OrderSide, quantity: u64) -> ParsedOrder {
        ParsedOrder {
            ship_id,
            item_id: ItemId::ScrapMetal,
            side,
            price: 100,
            quantity,
        }
    }

    #[derive(Default)]
    struct FakeTarget {
        owned_ships: Vec<(PlayerId, ShipId)>,
        cargo: Vec<(PlayerId, ShipId, ItemId, u64)>,
        fail_credit_after: Option<usize>,
        credit_attempts: usize,
    }

    impl FakeTarget {
        fn count(&self, player_id: PlayerId, ship_id: ShipId, item_id: ItemId) -> u64 {
            self.cargo
                .iter()
                .find(|(owner, ship, item, _)| {
                    *owner == player_id && *ship == ship_id && *item == item_id
                })
                .map(|(_, _, _, count)| *count)
                .unwrap_or(0)
        }

        fn add(&mut self, player_id: PlayerId, ship_id: ShipId, item_id: ItemId, quantity: u64) {
            if let Some((_, _, _, count)) = self.cargo.iter_mut().find(|(owner, ship, item, _)| {
                *owner == player_id && *ship == ship_id && *item == item_id
            }) {
                *count += quantity;
            } else {
                self.cargo.push((player_id, ship_id, item_id, quantity));
            }
        }
    }

    impl SettlementTarget for FakeTarget {
        fn owns_ship(&mut self, player_id: PlayerId, ship_id: ShipId) -> bool {
            self.owned_ships
                .iter()
                .any(|(owner, owned_ship)| *owner == player_id && *owned_ship == ship_id)
        }

        fn remove_item(&mut self, command: RemoveItemCommand) -> bool {
            let Some((_, _, _, count)) = self.cargo.iter_mut().find(|(owner, ship, item, _)| {
                *owner == command.player_id && *ship == command.ship_id && *item == command.item_id
            }) else {
                return false;
            };
            if *count < command.quantity {
                return false;
            }
            *count -= command.quantity;
            true
        }

        fn return_item(&mut self, command: ReturnItemCommand) -> bool {
            if !self.owns_ship(command.player_id, command.ship_id) {
                return false;
            }
            self.add(
                command.player_id,
                command.ship_id,
                command.item_id,
                command.quantity,
            );
            true
        }

        fn credit_item(&mut self, command: CreditItemCommand) -> bool {
            self.credit_attempts += 1;
            if self
                .fail_credit_after
                .is_some_and(|limit| self.credit_attempts > limit)
            {
                return false;
            }
            self.add(
                command.player_id,
                command.ship_id,
                command.item_id,
                command.quantity,
            );
            true
        }
    }

    #[test]
    fn placing_an_ask_reserves_cargo_before_it_rests() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let seller_ship = ship(1);
        let mut target = FakeTarget {
            owned_ships: vec![(seller, seller_ship)],
            cargo: vec![(seller, seller_ship, ItemId::ScrapMetal, 5)],
            ..Default::default()
        };

        let result = MarketSettlement::place(
            &mut db,
            seller,
            order(seller_ship, OrderSide::Ask, 5),
            &mut target,
        );

        assert_eq!(result, MarketSettlementResult::Completed("Order placed"));
        assert_eq!(target.count(seller, seller_ship, ItemId::ScrapMetal), 0);
        assert_eq!(db.open_orders_for(seller).unwrap().len(), 1);
    }

    #[test]
    fn failed_multi_buyer_credit_compensates_earlier_cargo_changes() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller_a = PlayerId(1);
        let seller_b = PlayerId(2);
        let buyer = PlayerId(3);
        let seller_ship_a = ship(1);
        let seller_ship_b = ship(2);
        let buyer_ship = ship(3);
        let mut target = FakeTarget {
            owned_ships: vec![
                (seller_a, seller_ship_a),
                (seller_b, seller_ship_b),
                (buyer, buyer_ship),
            ],
            cargo: vec![
                (seller_a, seller_ship_a, ItemId::ScrapMetal, 2),
                (seller_b, seller_ship_b, ItemId::ScrapMetal, 2),
            ],
            fail_credit_after: Some(1),
            ..Default::default()
        };

        assert_eq!(
            MarketSettlement::place(
                &mut db,
                seller_a,
                order(seller_ship_a, OrderSide::Ask, 2),
                &mut target,
            ),
            MarketSettlementResult::Completed("Order placed")
        );
        assert_eq!(
            MarketSettlement::place(
                &mut db,
                seller_b,
                order(seller_ship_b, OrderSide::Ask, 2),
                &mut target,
            ),
            MarketSettlementResult::Completed("Order placed")
        );

        db.credit_currency(buyer, 1_000).unwrap();
        let result = MarketSettlement::place(
            &mut db,
            buyer,
            order(buyer_ship, OrderSide::Bid, 4),
            &mut target,
        );

        assert_eq!(
            result,
            MarketSettlementResult::NeedsAttention("Order placed; settlement needs attention")
        );
        assert_eq!(
            target.count(buyer, buyer_ship, ItemId::ScrapMetal),
            0,
            "a failed later credit must not leave the earlier credit applied"
        );
        assert_eq!(target.credit_attempts, 2);
        assert!(db.open_orders_for(buyer).unwrap().is_empty());
    }

    #[test]
    fn cancel_reports_when_the_sector_cannot_return_the_ask() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let seller_ship = ship(1);
        let mut target = FakeTarget {
            owned_ships: vec![(seller, seller_ship)],
            cargo: vec![(seller, seller_ship, ItemId::ScrapMetal, 1)],
            ..Default::default()
        };

        MarketSettlement::place(
            &mut db,
            seller,
            order(seller_ship, OrderSide::Ask, 1),
            &mut target,
        );
        let order_id = db.open_orders_for(seller).unwrap()[0].order_id;

        // Removing ownership is the external failure that the runtime must
        // surface after the Market row has already been cancelled.
        target.owned_ships.clear();
        let result = MarketSettlement::cancel(&mut db, seller, order_id, &mut target);

        assert_eq!(
            result,
            MarketSettlementResult::NeedsAttention("Order cancelled; item return needs attention")
        );
        assert!(db.open_orders_for(seller).unwrap().is_empty());
    }
}
