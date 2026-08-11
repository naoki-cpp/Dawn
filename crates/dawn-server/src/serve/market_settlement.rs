//! Market-to-Sector settlement delivery adapter.
//!
//! `dawn-market` emits durable value intents. This module is the only place
//! that translates those intents into Sector-local inventory mutations and
//! acknowledges them back to the Market database. Missing cluster nodes leave
//! the outbox row pending; a duplicate acknowledgement is harmless because
//! the Market policy treats non-pending settlement rows as already decided.

use dawn_core::{
    CreditItemCommand, ItemId, PlayerId, RemoveItemCommand, ReturnItemCommand, ShipId,
};
use dawn_market::{
    MarketCommand, MarketDb, MarketError, MarketTransition, OrderId, OrderSide, SettlementEffect,
    SettlementIntent,
};
use dawn_sector::node::SimulationNode;

use crate::runtime_frame::{RuntimeNodeMutation, RuntimeNodeView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MarketSettlementResult {
    Completed(String),
    Rejected(String),
    NeedsAttention(String),
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
    pub(super) fn notice(&self) -> &str {
        match self {
            Self::Completed(notice) | Self::Rejected(notice) | Self::NeedsAttention(notice) => {
                notice
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettlementDelivery {
    Applied,
    Unavailable,
    Rejected(&'static str),
}

trait SettlementTarget {
    fn deliver(&mut self, intent: SettlementIntent) -> SettlementDelivery;
}

struct SingleTarget<'a, N: RuntimeNodeMutation> {
    node: &'a mut N,
}

struct ClusterTarget<'a, N: RuntimeNodeMutation> {
    nodes: &'a mut [N],
}

impl<N: RuntimeNodeMutation> SettlementTarget for SingleTarget<'_, N> {
    fn deliver(&mut self, intent: SettlementIntent) -> SettlementDelivery {
        let (player_id, ship_id) = intent_identity(intent);
        self.node.with_runtime_node_mut(|node| {
            if !owns_docked_ship(node, player_id, ship_id) {
                return SettlementDelivery::Unavailable;
            }
            apply_intent(node, intent)
        })
    }
}

impl<N: RuntimeNodeMutation> SettlementTarget for ClusterTarget<'_, N> {
    fn deliver(&mut self, intent: SettlementIntent) -> SettlementDelivery {
        let (player_id, ship_id) = intent_identity(intent);
        let Some(index) = find_node_index(self.nodes, player_id, ship_id) else {
            return SettlementDelivery::Unavailable;
        };
        self.nodes[index].with_runtime_node_mut(|node| apply_intent(node, intent))
    }
}

impl<'a, N: RuntimeNodeMutation> ClusterTarget<'a, N> {
    fn new(nodes: &'a mut [N]) -> Self {
        Self { nodes }
    }
}

impl<'a, N: RuntimeNodeMutation> SingleTarget<'a, N> {
    fn new(node: &'a mut N) -> Self {
        Self { node }
    }
}

pub(super) struct MarketSettlement;

impl MarketSettlement {
    pub(super) fn place_single<N: RuntimeNodeMutation>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
        node: &mut N,
    ) -> MarketSettlementResult {
        let mut target = SingleTarget::new(node);
        Self::place(db, player_id, order, &mut target)
    }

    pub(super) fn place_cluster<N: RuntimeNodeMutation>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
        nodes: &mut [N],
    ) -> MarketSettlementResult {
        let mut target = ClusterTarget::new(nodes);
        Self::place(db, player_id, order, &mut target)
    }

    pub(super) fn cancel_single<N: RuntimeNodeMutation>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
        node: &mut N,
    ) -> MarketSettlementResult {
        let mut target = SingleTarget::new(node);
        Self::cancel(db, player_id, order_id, &mut target)
    }

    pub(super) fn cancel_cluster<N: RuntimeNodeMutation>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
        nodes: &mut [N],
    ) -> MarketSettlementResult {
        let mut target = ClusterTarget::new(nodes);
        Self::cancel(db, player_id, order_id, &mut target)
    }

    fn place<T: SettlementTarget>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order: ParsedOrder,
        target: &mut T,
    ) -> MarketSettlementResult {
        let command = MarketCommand::PlaceOrder {
            player_id,
            ship_id: order.ship_id,
            item_id: order.item_id,
            side: order.side,
            price: order.price,
            quantity: order.quantity,
        };
        match db.execute(command) {
            Ok(transition) => settle_transition(db, target, transition, "Order placed"),
            Err(error) => rejected(error),
        }
    }

    fn cancel<T: SettlementTarget>(
        db: &mut MarketDb,
        player_id: PlayerId,
        order_id: OrderId,
        target: &mut T,
    ) -> MarketSettlementResult {
        match db.execute(MarketCommand::CancelOrder {
            player_id,
            order_id,
        }) {
            Ok(transition) => settle_transition(db, target, transition, "Order cancelled"),
            Err(error) => rejected(error),
        }
    }
}

fn settle_transition<T: SettlementTarget>(
    db: &mut MarketDb,
    target: &mut T,
    _transition: MarketTransition,
    success_notice: &str,
) -> MarketSettlementResult {
    let mut saw_rejection = false;
    for _ in 0..32 {
        let pending = match db.pending_settlements() {
            Ok(pending) => pending,
            Err(error) => return MarketSettlementResult::NeedsAttention(error.to_string()),
        };
        if pending.is_empty() {
            return if saw_rejection {
                MarketSettlementResult::NeedsAttention(format!(
                    "{success_notice}; settlement compensation required"
                ))
            } else {
                MarketSettlementResult::Completed(success_notice.to_owned())
            };
        }

        let mut made_progress = false;
        for intent in pending {
            match target.deliver(intent) {
                SettlementDelivery::Applied => {
                    if let Err(error) = db.execute(MarketCommand::AcknowledgeSettlement {
                        settlement_id: intent.id,
                    }) {
                        return MarketSettlementResult::NeedsAttention(error.to_string());
                    }
                    made_progress = true;
                }
                SettlementDelivery::Unavailable => {}
                SettlementDelivery::Rejected(reason) => {
                    saw_rejection = true;
                    if let Err(error) = db.execute(MarketCommand::RejectSettlement {
                        settlement_id: intent.id,
                        reason: reason.to_owned(),
                    }) {
                        return MarketSettlementResult::NeedsAttention(error.to_string());
                    }
                    made_progress = true;
                }
            }
        }
        if !made_progress {
            return MarketSettlementResult::Completed(format!(
                "{success_notice}; settlement pending"
            ));
        }
    }
    MarketSettlementResult::NeedsAttention(format!(
        "{success_notice}; settlement retry limit reached"
    ))
}

fn rejected(error: MarketError) -> MarketSettlementResult {
    MarketSettlementResult::Rejected(error.to_string())
}

fn apply_intent(node: &mut SimulationNode, intent: SettlementIntent) -> SettlementDelivery {
    let settlement_id = match u64::try_from(intent.id.0) {
        Ok(id) if id > 0 => id,
        _ => return SettlementDelivery::Rejected("invalid settlement identity"),
    };
    let applied = match intent.effect {
        SettlementEffect::ReserveAsk {
            player_id,
            ship_id,
            item_id,
            quantity,
            ..
        } => node.remove_item_owned(RemoveItemCommand {
            player_id,
            ship_id,
            item_id,
            quantity,
            settlement_id,
        }),
        SettlementEffect::ReturnItem {
            player_id,
            ship_id,
            item_id,
            quantity,
        } => node.return_item_owned(ReturnItemCommand {
            player_id,
            ship_id,
            item_id,
            quantity,
            settlement_id,
        }),
        SettlementEffect::CreditItem {
            buyer,
            buyer_ship_id,
            item_id,
            quantity,
            ..
        } => node.credit_item_owned(CreditItemCommand {
            player_id: buyer,
            ship_id: buyer_ship_id,
            item_id,
            quantity,
            settlement_id,
        }),
    };
    if applied {
        SettlementDelivery::Applied
    } else {
        SettlementDelivery::Rejected("Sector rejected the inventory settlement")
    }
}

fn intent_identity(intent: SettlementIntent) -> (PlayerId, ShipId) {
    match intent.effect {
        SettlementEffect::ReserveAsk {
            player_id, ship_id, ..
        }
        | SettlementEffect::ReturnItem {
            player_id, ship_id, ..
        } => (player_id, ship_id),
        SettlementEffect::CreditItem {
            buyer,
            buyer_ship_id,
            ..
        } => (buyer, buyer_ship_id),
    }
}

fn find_node_index<N: RuntimeNodeView>(
    nodes: &[N],
    player_id: PlayerId,
    ship_id: ShipId,
) -> Option<usize> {
    nodes
        .iter()
        .position(|node| owns_docked_ship(node.runtime_node(), player_id, ship_id))
}

fn owns_docked_ship(node: &SimulationNode, player_id: PlayerId, ship_id: ShipId) -> bool {
    node.owns_ship(player_id, ship_id) && node.docked_station(ship_id).is_some()
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
        fn deliver(&mut self, intent: SettlementIntent) -> SettlementDelivery {
            let (player_id, ship_id) = intent_identity(intent);
            if !self.owned_ships.contains(&(player_id, ship_id)) {
                return SettlementDelivery::Rejected("destination ship is unavailable");
            }
            match intent.effect {
                SettlementEffect::ReserveAsk {
                    item_id, quantity, ..
                } => {
                    let Some((_, _, _, count)) =
                        self.cargo.iter_mut().find(|(owner, ship, item, _)| {
                            *owner == player_id && *ship == ship_id && *item == item_id
                        })
                    else {
                        return SettlementDelivery::Rejected("item is unavailable");
                    };
                    if *count < quantity {
                        return SettlementDelivery::Rejected("item quantity is unavailable");
                    }
                    *count -= quantity;
                }
                SettlementEffect::ReturnItem {
                    item_id, quantity, ..
                } => self.add(player_id, ship_id, item_id, quantity),
                SettlementEffect::CreditItem {
                    buyer,
                    buyer_ship_id,
                    item_id,
                    quantity,
                    ..
                } => {
                    self.credit_attempts += 1;
                    if self
                        .fail_credit_after
                        .is_some_and(|limit| self.credit_attempts > limit)
                    {
                        return SettlementDelivery::Rejected("credit rejected");
                    }
                    self.add(buyer, buyer_ship_id, item_id, quantity);
                }
            }
            SettlementDelivery::Applied
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

        assert_eq!(
            result,
            MarketSettlementResult::Completed("Order placed".to_owned())
        );
        assert_eq!(target.count(seller, seller_ship, ItemId::ScrapMetal), 0);
        assert_eq!(db.open_orders_for(seller).unwrap().len(), 1);
    }

    #[test]
    fn failed_credit_refunds_currency_and_returns_reserved_item() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let seller = PlayerId(1);
        let buyer = PlayerId(2);
        let seller_ship = ship(1);
        let buyer_ship = ship(2);
        let mut target = FakeTarget {
            owned_ships: vec![(seller, seller_ship), (buyer, buyer_ship)],
            cargo: vec![(seller, seller_ship, ItemId::ScrapMetal, 2)],
            fail_credit_after: Some(0),
            ..Default::default()
        };
        MarketSettlement::place(
            &mut db,
            seller,
            order(seller_ship, OrderSide::Ask, 2),
            &mut target,
        );
        db.credit_currency(buyer, 200).unwrap();

        let result = MarketSettlement::place(
            &mut db,
            buyer,
            order(buyer_ship, OrderSide::Bid, 2),
            &mut target,
        );

        assert!(matches!(result, MarketSettlementResult::NeedsAttention(_)));
        assert_eq!(db.currency_balance(buyer).unwrap(), 200);
        assert_eq!(target.count(seller, seller_ship, ItemId::ScrapMetal), 2);
    }

    #[test]
    fn cancel_reports_when_sector_cannot_return_the_ask() {
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
        target.owned_ships.clear();

        let result = MarketSettlement::cancel(&mut db, seller, order_id, &mut target);
        assert!(matches!(result, MarketSettlementResult::NeedsAttention(_)));
        assert!(db.open_orders_for(seller).unwrap().is_empty());
    }
}
