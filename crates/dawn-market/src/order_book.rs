//! Pure Market order, escrow, and settlement policy.
//!
//! This module deliberately has no SQL or Sector bridge command knowledge.
//! [`MarketState::apply`] is the only place that decides order visibility,
//! price-time matching, Currency escrow, and the durable settlement effects
//! that must be delivered to Sector.

use std::collections::BTreeMap;

use dawn_core::{ItemId, PlayerId, ShipId};

use crate::matching::{self, IncomingOrder, RestingOrder};

const FIRST_ORDER_ID: i64 = 1;
const FIRST_SETTLEMENT_ID: i64 = 1;

/// Which side of the book an order rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderSide {
    Bid,
    Ask,
}

impl OrderSide {
    pub(super) fn opposite(self) -> Self {
        match self {
            Self::Bid => Self::Ask,
            Self::Ask => Self::Bid,
        }
    }
}

/// Identifies one resting or historical order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderId(pub i64);

/// Identifies one durable Market-to-Sector delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SettlementId(pub i64);

/// Whether an order is visible to matching and Market snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    /// An Ask whose source cargo has not been acknowledged by Sector yet.
    PendingReservation,
    /// The order is available to the pure matching policy.
    Open,
}

/// The effect that a Market transaction asks the owning Sector to apply.
///
/// These are value objects, not `dawn-core` commands. The serve adapter is the
/// only layer that translates them into a Sector-local mutation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementEffect {
    /// Reserve an Ask's cargo before the order becomes visible.
    ReserveAsk {
        order_id: OrderId,
        player_id: PlayerId,
        ship_id: ShipId,
        item_id: ItemId,
        quantity: u64,
    },
    /// Return a cancelled or compensated Ask quantity.
    ReturnItem {
        player_id: PlayerId,
        ship_id: ShipId,
        item_id: ItemId,
        quantity: u64,
    },
    /// Deliver a filled Item; the seller receives the maker-price proceeds
    /// only when this intent is acknowledged.
    CreditItem {
        buyer: PlayerId,
        buyer_ship_id: ShipId,
        seller: PlayerId,
        seller_ship_id: ShipId,
        item_id: ItemId,
        quantity: u64,
        price: u64,
    },
}

/// Durable lifecycle of one settlement outbox row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementStatus {
    Pending,
    Applied,
    /// A failed credit has refunded Currency and is waiting for the seller
    /// Item compensation to be acknowledged.
    Compensating,
    /// The downstream effect was rejected and no further automatic mutation
    /// is safe. Operators can inspect the retained row and retry deliberately.
    Terminal,
    /// The original effect failed but its compensating ReturnItem succeeded.
    Compensated,
}

/// A pending delivery selected by the runtime outbox poller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettlementIntent {
    pub id: SettlementId,
    pub effect: SettlementEffect,
}

/// Durable settlement state retained for retry, audit, and compensation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementRecord {
    pub id: SettlementId,
    pub effect: SettlementEffect,
    pub status: SettlementStatus,
    pub compensation_for: Option<SettlementId>,
    pub last_error: Option<String>,
}

/// One currently visible order read from the Market state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketOrderView {
    pub order_id: OrderId,
    pub player_id: PlayerId,
    pub ship_id: ShipId,
    pub item_id: ItemId,
    pub side: OrderSide,
    pub price: u64,
    pub quantity_remaining: u64,
}

/// A public/domain fact produced by one accepted Market transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketEvent {
    OrderQueued {
        order_id: OrderId,
        side: OrderSide,
        quantity: u64,
    },
    OrderOpened {
        order_id: OrderId,
    },
    OrderCancelled {
        order_id: OrderId,
    },
    TradeExecuted {
        settlement_id: SettlementId,
        buyer: PlayerId,
        seller: PlayerId,
        item_id: ItemId,
        price: u64,
        quantity: u64,
    },
    SettlementQueued {
        settlement_id: SettlementId,
    },
    SettlementApplied {
        settlement_id: SettlementId,
    },
    SettlementCompensationQueued {
        settlement_id: SettlementId,
        compensation_id: SettlementId,
    },
    SettlementTerminal {
        settlement_id: SettlementId,
    },
}

/// Commands accepted by the pure Market policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketCommand {
    PlaceOrder {
        player_id: PlayerId,
        ship_id: ShipId,
        item_id: ItemId,
        side: OrderSide,
        price: u64,
        quantity: u64,
    },
    CancelOrder {
        player_id: PlayerId,
        order_id: OrderId,
    },
    AcknowledgeSettlement {
        settlement_id: SettlementId,
    },
    RejectSettlement {
        settlement_id: SettlementId,
        reason: String,
    },
    /// Administrative/test funding remains a normal transition so the
    /// repository never mutates Currency behind the domain policy's back.
    CreditCurrency {
        player_id: PlayerId,
        amount: u64,
    },
}

/// Rejections from pure Market policy validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarketRejection {
    #[error("price and quantity must be positive")]
    InvalidQuantityOrPrice,
    #[error("price multiplied by quantity overflows")]
    ArithmeticOverflow,
    #[error("Currency balance is insufficient")]
    InsufficientBalance,
    #[error("order {0:?} was not found")]
    OrderNotFound(OrderId),
    #[error("order {order_id:?} is not owned by player {player_id:?}")]
    OrderNotOwned {
        order_id: OrderId,
        player_id: PlayerId,
    },
    #[error("order {0:?} is still waiting for Sector cargo reservation")]
    SettlementPending(OrderId),
    #[error("settlement {0:?} was not found")]
    SettlementNotFound(SettlementId),
    #[error("Currency balance would overflow")]
    CurrencyOverflow,
}

/// Result classification for an accepted transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketCommandResult {
    OrderPending {
        order_id: OrderId,
        reservation_id: SettlementId,
    },
    OrderPlaced {
        order_id: Option<OrderId>,
        fills: usize,
    },
    OrderCancelled {
        return_id: Option<SettlementId>,
    },
    SettlementApplied {
        settlement_id: SettlementId,
    },
    SettlementCompensating {
        settlement_id: SettlementId,
        compensation_id: SettlementId,
    },
    SettlementTerminal {
        settlement_id: SettlementId,
    },
    DuplicateSettlementAcknowledgement {
        settlement_id: SettlementId,
    },
    CurrencyCredited,
}

/// All pure outputs of one accepted Market command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketTransition {
    pub result: MarketCommandResult,
    pub events: Vec<MarketEvent>,
    pub settlements: Vec<SettlementIntent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Order {
    pub(crate) id: OrderId,
    pub(crate) player_id: PlayerId,
    pub(crate) ship_id: ShipId,
    pub(crate) item_id: ItemId,
    pub(crate) side: OrderSide,
    pub(crate) price: u64,
    pub(crate) quantity_remaining: u64,
    pub(crate) escrowed_currency: u64,
    pub(crate) status: OrderStatus,
    pub(crate) reservation_id: Option<SettlementId>,
}

/// Pure Market state. No SQL connection, clock, transport, or Sector handle
/// is stored here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketState {
    orders: BTreeMap<OrderId, Order>,
    balances: BTreeMap<PlayerId, u64>,
    settlements: BTreeMap<SettlementId, SettlementRecord>,
    next_order_id: i64,
    next_settlement_id: i64,
}

impl Default for MarketState {
    fn default() -> Self {
        Self {
            orders: BTreeMap::new(),
            balances: BTreeMap::new(),
            settlements: BTreeMap::new(),
            next_order_id: FIRST_ORDER_ID,
            next_settlement_id: FIRST_SETTLEMENT_ID,
        }
    }
}

impl MarketState {
    /// Apply one command without performing any IO.
    pub fn apply(&mut self, command: MarketCommand) -> Result<MarketTransition, MarketRejection> {
        let before = self.clone();
        match self.apply_inner(command) {
            Ok(transition) => Ok(transition),
            Err(error) => {
                *self = before;
                Err(error)
            }
        }
    }

    fn apply_inner(&mut self, command: MarketCommand) -> Result<MarketTransition, MarketRejection> {
        match command {
            MarketCommand::PlaceOrder {
                player_id,
                ship_id,
                item_id,
                side,
                price,
                quantity,
            } => self.place_order(player_id, ship_id, item_id, side, price, quantity),
            MarketCommand::CancelOrder {
                player_id,
                order_id,
            } => self.cancel_order(player_id, order_id),
            MarketCommand::AcknowledgeSettlement { settlement_id } => {
                self.acknowledge_settlement(settlement_id)
            }
            MarketCommand::RejectSettlement {
                settlement_id,
                reason,
            } => self.reject_settlement(settlement_id, reason),
            MarketCommand::CreditCurrency { player_id, amount } => {
                let balance = self.balances.entry(player_id).or_default();
                *balance = balance
                    .checked_add(amount)
                    .ok_or(MarketRejection::CurrencyOverflow)?;
                Ok(MarketTransition {
                    result: MarketCommandResult::CurrencyCredited,
                    events: Vec::new(),
                    settlements: Vec::new(),
                })
            }
        }
    }

    /// Current Currency balance, or zero when no balance exists.
    pub fn currency_balance(&self, player_id: PlayerId) -> u64 {
        self.balances.get(&player_id).copied().unwrap_or(0)
    }

    /// Return visible orders in stable order-id order.
    pub fn open_orders(&self) -> Vec<MarketOrderView> {
        self.orders
            .values()
            .filter(|order| order.status == OrderStatus::Open)
            .map(|order| MarketOrderView {
                order_id: order.id,
                player_id: order.player_id,
                ship_id: order.ship_id,
                item_id: order.item_id,
                side: order.side,
                price: order.price,
                quantity_remaining: order.quantity_remaining,
            })
            .collect()
    }

    /// Return pending outbox work. Non-pending history is retained but never
    /// selected for automatic redelivery.
    pub fn pending_settlements(&self) -> Vec<SettlementIntent> {
        self.settlements
            .values()
            .filter(|record| record.status == SettlementStatus::Pending)
            .map(|record| SettlementIntent {
                id: record.id,
                effect: record.effect,
            })
            .collect()
    }

    /// Inspect one durable settlement record.
    pub fn settlement(&self, id: SettlementId) -> Option<&SettlementRecord> {
        self.settlements.get(&id)
    }

    pub(crate) fn from_parts(
        orders: BTreeMap<OrderId, Order>,
        balances: BTreeMap<PlayerId, u64>,
        settlements: BTreeMap<SettlementId, SettlementRecord>,
        next_order_id: i64,
        next_settlement_id: i64,
    ) -> Self {
        Self {
            orders,
            balances,
            settlements,
            next_order_id: next_order_id.max(FIRST_ORDER_ID),
            next_settlement_id: next_settlement_id.max(FIRST_SETTLEMENT_ID),
        }
    }

    pub(crate) fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }

    pub(crate) fn balances(&self) -> impl Iterator<Item = (&PlayerId, &u64)> {
        self.balances.iter()
    }

    pub(crate) fn settlements(&self) -> impl Iterator<Item = &SettlementRecord> {
        self.settlements.values()
    }

    pub(crate) fn next_ids(&self) -> (i64, i64) {
        (self.next_order_id, self.next_settlement_id)
    }

    fn place_order(
        &mut self,
        player_id: PlayerId,
        ship_id: ShipId,
        item_id: ItemId,
        side: OrderSide,
        price: u64,
        quantity: u64,
    ) -> Result<MarketTransition, MarketRejection> {
        let cost = price
            .checked_mul(quantity)
            .ok_or(MarketRejection::ArithmeticOverflow)?;
        if price == 0 || quantity == 0 {
            return Err(MarketRejection::InvalidQuantityOrPrice);
        }

        let order_id = self.allocate_order_id()?;
        if side == OrderSide::Ask {
            let reservation_id = self.queue_settlement(SettlementEffect::ReserveAsk {
                order_id,
                player_id,
                ship_id,
                item_id,
                quantity,
            })?;
            self.orders.insert(
                order_id,
                Order {
                    id: order_id,
                    player_id,
                    ship_id,
                    item_id,
                    side,
                    price,
                    quantity_remaining: quantity,
                    escrowed_currency: 0,
                    status: OrderStatus::PendingReservation,
                    reservation_id: Some(reservation_id),
                },
            );
            return Ok(MarketTransition {
                result: MarketCommandResult::OrderPending {
                    order_id,
                    reservation_id,
                },
                events: vec![
                    MarketEvent::OrderQueued {
                        order_id,
                        side,
                        quantity,
                    },
                    MarketEvent::SettlementQueued {
                        settlement_id: reservation_id,
                    },
                ],
                settlements: self.pending_settlements_for(&[reservation_id]),
            });
        }

        let balance = self.balances.entry(player_id).or_default();
        if *balance < cost {
            return Err(MarketRejection::InsufficientBalance);
        }
        *balance -= cost;
        self.orders.insert(
            order_id,
            Order {
                id: order_id,
                player_id,
                ship_id,
                item_id,
                side,
                price,
                quantity_remaining: quantity,
                escrowed_currency: cost,
                status: OrderStatus::Open,
                reservation_id: None,
            },
        );
        let (fills, settlements) = self.match_order(order_id)?;
        let visible_order = self.orders.get(&order_id).map(|order| order.id);
        let mut events = Vec::new();
        for settlement in &settlements {
            events.push(MarketEvent::SettlementQueued {
                settlement_id: settlement.id,
            });
        }
        events.extend(fills);
        Ok(MarketTransition {
            result: MarketCommandResult::OrderPlaced {
                order_id: visible_order,
                fills: events
                    .iter()
                    .filter(|event| matches!(event, MarketEvent::TradeExecuted { .. }))
                    .count(),
            },
            events,
            settlements,
        })
    }

    fn cancel_order(
        &mut self,
        player_id: PlayerId,
        order_id: OrderId,
    ) -> Result<MarketTransition, MarketRejection> {
        let order = *self
            .orders
            .get(&order_id)
            .ok_or(MarketRejection::OrderNotFound(order_id))?;
        if order.player_id != player_id {
            return Err(MarketRejection::OrderNotOwned {
                order_id,
                player_id,
            });
        }
        if order.status == OrderStatus::PendingReservation {
            return Err(MarketRejection::SettlementPending(order_id));
        }
        self.orders.remove(&order_id);
        if order.escrowed_currency > 0 {
            self.credit_currency(player_id, order.escrowed_currency)?;
        }
        let return_id = if order.side == OrderSide::Ask {
            Some(self.queue_settlement(SettlementEffect::ReturnItem {
                player_id,
                ship_id: order.ship_id,
                item_id: order.item_id,
                quantity: order.quantity_remaining,
            })?)
        } else {
            None
        };
        let mut events = vec![MarketEvent::OrderCancelled { order_id }];
        if let Some(id) = return_id {
            events.push(MarketEvent::SettlementQueued { settlement_id: id });
        }
        Ok(MarketTransition {
            result: MarketCommandResult::OrderCancelled { return_id },
            events,
            settlements: return_id
                .map(|id| self.pending_settlements_for(&[id]))
                .unwrap_or_default(),
        })
    }

    fn acknowledge_settlement(
        &mut self,
        settlement_id: SettlementId,
    ) -> Result<MarketTransition, MarketRejection> {
        let record = self
            .settlements
            .get(&settlement_id)
            .cloned()
            .ok_or(MarketRejection::SettlementNotFound(settlement_id))?;
        if record.status != SettlementStatus::Pending {
            return Ok(MarketTransition {
                result: MarketCommandResult::DuplicateSettlementAcknowledgement { settlement_id },
                events: Vec::new(),
                settlements: Vec::new(),
            });
        }

        self.settlement_mut(settlement_id).status = SettlementStatus::Applied;
        let mut events = vec![MarketEvent::SettlementApplied { settlement_id }];
        let mut settlements = Vec::new();
        let mut result = MarketCommandResult::SettlementApplied { settlement_id };
        if let SettlementEffect::ReserveAsk { order_id, .. } = record.effect {
            let order = self
                .orders
                .get_mut(&order_id)
                .ok_or(MarketRejection::OrderNotFound(order_id))?;
            order.status = OrderStatus::Open;
            order.reservation_id = None;
            events.push(MarketEvent::OrderOpened { order_id });
            let (fills, new_settlements) = self.match_order(order_id)?;
            settlements = new_settlements;
            events.extend(fills);
            result = MarketCommandResult::SettlementApplied { settlement_id };
        } else if let SettlementEffect::CreditItem {
            seller,
            quantity,
            price,
            ..
        } = record.effect
        {
            self.credit_currency(
                seller,
                price
                    .checked_mul(quantity)
                    .ok_or(MarketRejection::ArithmeticOverflow)?,
            )?;
        } else if let SettlementEffect::ReturnItem { .. } = record.effect {
            if let Some(parent_id) = self
                .settlements
                .get(&settlement_id)
                .and_then(|record| record.compensation_for)
            {
                self.settlement_mut(parent_id).status = SettlementStatus::Compensated;
            }
        }
        Ok(MarketTransition {
            result,
            events,
            settlements,
        })
    }

    fn reject_settlement(
        &mut self,
        settlement_id: SettlementId,
        reason: String,
    ) -> Result<MarketTransition, MarketRejection> {
        let record = self
            .settlements
            .get(&settlement_id)
            .cloned()
            .ok_or(MarketRejection::SettlementNotFound(settlement_id))?;
        if record.status != SettlementStatus::Pending {
            return Ok(MarketTransition {
                result: MarketCommandResult::DuplicateSettlementAcknowledgement { settlement_id },
                events: Vec::new(),
                settlements: Vec::new(),
            });
        }

        let mut events = Vec::new();
        let mut settlements = Vec::new();
        let result = match record.effect {
            SettlementEffect::ReserveAsk { order_id, .. } => {
                self.orders.remove(&order_id);
                self.mark_terminal(settlement_id, reason);
                events.push(MarketEvent::SettlementTerminal { settlement_id });
                MarketCommandResult::SettlementTerminal { settlement_id }
            }
            SettlementEffect::ReturnItem { .. } => {
                if let Some(parent_id) = record.compensation_for {
                    self.settlement_mut(parent_id).status = SettlementStatus::Terminal;
                }
                self.mark_terminal(settlement_id, reason);
                events.push(MarketEvent::SettlementTerminal { settlement_id });
                MarketCommandResult::SettlementTerminal { settlement_id }
            }
            SettlementEffect::CreditItem {
                buyer,
                seller,
                seller_ship_id,
                item_id,
                quantity,
                price,
                ..
            } => {
                let refund = price
                    .checked_mul(quantity)
                    .ok_or(MarketRejection::ArithmeticOverflow)?;
                self.credit_currency(buyer, refund)?;
                let compensation_id = self.queue_compensation(
                    SettlementEffect::ReturnItem {
                        player_id: seller,
                        ship_id: seller_ship_id,
                        item_id,
                        quantity,
                    },
                    settlement_id,
                )?;
                self.settlement_mut(settlement_id).status = SettlementStatus::Compensating;
                self.settlement_mut(settlement_id).last_error = Some(reason);
                events.push(MarketEvent::SettlementCompensationQueued {
                    settlement_id,
                    compensation_id,
                });
                settlements = self.pending_settlements_for(&[compensation_id]);
                MarketCommandResult::SettlementCompensating {
                    settlement_id,
                    compensation_id,
                }
            }
        };
        Ok(MarketTransition {
            result,
            events,
            settlements,
        })
    }

    fn match_order(
        &mut self,
        order_id: OrderId,
    ) -> Result<(Vec<MarketEvent>, Vec<SettlementIntent>), MarketRejection> {
        let incoming = *self
            .orders
            .get(&order_id)
            .ok_or(MarketRejection::OrderNotFound(order_id))?;
        let candidates = self
            .orders
            .values()
            .filter(|order| order.id != order_id && order.status == OrderStatus::Open)
            .map(|order| RestingOrder {
                order_id: order.id.0,
                player_id: order.player_id,
                ship_id: order.ship_id,
                item_id: order.item_id,
                side: order.side,
                quantity_remaining: order.quantity_remaining,
                price: order.price,
                escrowed_currency: order.escrowed_currency,
            });
        let plan = matching::plan_matches(
            IncomingOrder {
                player_id: incoming.player_id,
                ship_id: incoming.ship_id,
                item_id: incoming.item_id,
                side: incoming.side,
                price: incoming.price,
                quantity: incoming.quantity_remaining,
            },
            candidates,
        )?;
        let mut events = Vec::new();
        let mut settlements = Vec::new();
        for fill in plan.fills {
            let resting_id = OrderId(fill.resting_order_id);
            if fill.resting_quantity_remaining == 0 {
                self.orders.remove(&resting_id);
            } else if let Some(resting) = self.orders.get_mut(&resting_id) {
                resting.quantity_remaining = fill.resting_quantity_remaining;
                resting.escrowed_currency = fill.resting_escrowed_currency;
            }

            let seller_ship_id = if incoming.side == OrderSide::Ask {
                incoming.ship_id
            } else {
                self.orders
                    .get(&resting_id)
                    .map(|order| order.ship_id)
                    .unwrap_or_else(|| {
                        // A fully filled resting order is removed above. Its
                        // identity is carried by the match plan instead.
                        fill.seller_ship_id
                    })
            };
            let settlement_id = self.queue_settlement(SettlementEffect::CreditItem {
                buyer: fill.buyer,
                buyer_ship_id: fill.buyer_ship_id,
                seller: fill.seller,
                seller_ship_id,
                item_id: incoming.item_id,
                quantity: fill.quantity,
                price: fill.price,
            })?;
            events.push(MarketEvent::TradeExecuted {
                settlement_id,
                buyer: fill.buyer,
                seller: fill.seller,
                item_id: incoming.item_id,
                price: fill.price,
                quantity: fill.quantity,
            });
            settlements.push(self.pending_settlements_for(&[settlement_id])[0]);
            if incoming.side == OrderSide::Bid {
                let refund = (incoming.price - fill.price)
                    .checked_mul(fill.quantity)
                    .ok_or(MarketRejection::ArithmeticOverflow)?;
                self.credit_currency(incoming.player_id, refund)?;
            }
        }

        if plan.remaining_quantity == 0 {
            self.orders.remove(&order_id);
        } else if let Some(order) = self.orders.get_mut(&order_id) {
            order.quantity_remaining = plan.remaining_quantity;
            order.escrowed_currency = if order.side == OrderSide::Bid {
                order
                    .price
                    .checked_mul(plan.remaining_quantity)
                    .ok_or(MarketRejection::ArithmeticOverflow)?
            } else {
                0
            };
        }
        Ok((events, settlements))
    }

    fn allocate_order_id(&mut self) -> Result<OrderId, MarketRejection> {
        let id = OrderId(self.next_order_id);
        self.next_order_id = self
            .next_order_id
            .checked_add(1)
            .ok_or(MarketRejection::ArithmeticOverflow)?;
        Ok(id)
    }

    fn queue_settlement(
        &mut self,
        effect: SettlementEffect,
    ) -> Result<SettlementId, MarketRejection> {
        self.queue_settlement_with_parent(effect, None)
    }

    fn queue_compensation(
        &mut self,
        effect: SettlementEffect,
        parent_id: SettlementId,
    ) -> Result<SettlementId, MarketRejection> {
        self.queue_settlement_with_parent(effect, Some(parent_id))
    }

    fn queue_settlement_with_parent(
        &mut self,
        effect: SettlementEffect,
        compensation_for: Option<SettlementId>,
    ) -> Result<SettlementId, MarketRejection> {
        let id = SettlementId(self.next_settlement_id);
        self.next_settlement_id = self
            .next_settlement_id
            .checked_add(1)
            .ok_or(MarketRejection::ArithmeticOverflow)?;
        self.settlements.insert(
            id,
            SettlementRecord {
                id,
                effect,
                status: SettlementStatus::Pending,
                compensation_for,
                last_error: None,
            },
        );
        Ok(id)
    }

    fn pending_settlements_for(&self, ids: &[SettlementId]) -> Vec<SettlementIntent> {
        ids.iter()
            .filter_map(|id| self.settlements.get(id))
            .filter(|record| record.status == SettlementStatus::Pending)
            .map(|record| SettlementIntent {
                id: record.id,
                effect: record.effect,
            })
            .collect()
    }

    fn settlement_mut(&mut self, id: SettlementId) -> &mut SettlementRecord {
        self.settlements
            .get_mut(&id)
            .expect("settlement was validated before mutation")
    }

    fn mark_terminal(&mut self, id: SettlementId, reason: String) {
        let record = self.settlement_mut(id);
        record.status = SettlementStatus::Terminal;
        record.last_error = Some(reason);
    }

    fn credit_currency(&mut self, player_id: PlayerId, amount: u64) -> Result<(), MarketRejection> {
        let balance = self.balances.entry(player_id).or_default();
        *balance = balance
            .checked_add(amount)
            .ok_or(MarketRejection::CurrencyOverflow)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::NodeId;

    fn ship(id: u64) -> ShipId {
        ShipId::new(NodeId(0), id)
    }

    fn ask(player: u64, quantity: u64) -> MarketCommand {
        MarketCommand::PlaceOrder {
            player_id: PlayerId(player),
            ship_id: ship(player),
            item_id: ItemId::ScrapMetal,
            side: OrderSide::Ask,
            price: 100,
            quantity,
        }
    }

    fn bid(player: u64, price: u64, quantity: u64) -> MarketCommand {
        MarketCommand::PlaceOrder {
            player_id: PlayerId(player),
            ship_id: ship(player),
            item_id: ItemId::ScrapMetal,
            side: OrderSide::Bid,
            price,
            quantity,
        }
    }

    #[test]
    fn ask_is_not_visible_until_sector_reservation_is_acknowledged() {
        let mut state = MarketState::default();
        let queued = state.apply(ask(1, 2)).unwrap();
        assert!(state.open_orders().is_empty());
        let reservation = queued.settlements[0].id;

        state
            .apply(MarketCommand::AcknowledgeSettlement {
                settlement_id: reservation,
            })
            .unwrap();
        assert_eq!(state.open_orders().len(), 1);
    }

    #[test]
    fn duplicate_settlement_ack_is_a_no_op() {
        let mut state = MarketState::default();
        let reservation = state.apply(ask(1, 1)).unwrap().settlements[0].id;
        let command = MarketCommand::AcknowledgeSettlement {
            settlement_id: reservation,
        };
        state.apply(command.clone()).unwrap();
        let duplicate = state.apply(command).unwrap();
        assert!(matches!(
            duplicate.result,
            MarketCommandResult::DuplicateSettlementAcknowledgement { .. }
        ));
        assert_eq!(state.open_orders().len(), 1);
    }

    #[test]
    fn failed_credit_refunds_currency_and_queues_item_compensation() {
        let mut state = MarketState::default();
        state
            .apply(MarketCommand::CreditCurrency {
                player_id: PlayerId(2),
                amount: 100,
            })
            .unwrap();
        let reservation = state.apply(ask(1, 1)).unwrap().settlements[0].id;
        state
            .apply(MarketCommand::AcknowledgeSettlement {
                settlement_id: reservation,
            })
            .unwrap();
        let transition = state.apply(bid(2, 100, 1)).unwrap();
        let credit = transition.settlements[0].id;
        assert_eq!(state.currency_balance(PlayerId(2)), 0);

        let rejected = state
            .apply(MarketCommand::RejectSettlement {
                settlement_id: credit,
                reason: "destination ship unavailable".to_owned(),
            })
            .unwrap();
        assert!(matches!(
            rejected.result,
            MarketCommandResult::SettlementCompensating { .. }
        ));
        assert_eq!(state.currency_balance(PlayerId(2)), 100);
        assert_eq!(state.currency_balance(PlayerId(1)), 0);
        assert_eq!(state.pending_settlements().len(), 1);
        assert!(matches!(
            state.pending_settlements()[0].effect,
            SettlementEffect::ReturnItem { .. }
        ));
    }

    #[test]
    fn seller_is_paid_only_when_credit_settlement_is_acknowledged() {
        let mut state = MarketState::default();
        state
            .apply(MarketCommand::CreditCurrency {
                player_id: PlayerId(2),
                amount: 100,
            })
            .unwrap();
        let reservation = state.apply(ask(1, 1)).unwrap().settlements[0].id;
        state
            .apply(MarketCommand::AcknowledgeSettlement {
                settlement_id: reservation,
            })
            .unwrap();
        let credit = state.apply(bid(2, 100, 1)).unwrap().settlements[0].id;

        assert_eq!(state.currency_balance(PlayerId(1)), 0);
        state
            .apply(MarketCommand::AcknowledgeSettlement {
                settlement_id: credit,
            })
            .unwrap();
        assert_eq!(state.currency_balance(PlayerId(1)), 100);
    }

    #[test]
    fn failed_settlement_acknowledgement_does_not_partially_mutate_state() {
        let mut state = MarketState::default();
        state
            .apply(MarketCommand::CreditCurrency {
                player_id: PlayerId(1),
                amount: u64::MAX,
            })
            .unwrap();
        state
            .apply(MarketCommand::CreditCurrency {
                player_id: PlayerId(2),
                amount: 100,
            })
            .unwrap();
        let reservation = state.apply(ask(1, 1)).unwrap().settlements[0].id;
        state
            .apply(MarketCommand::AcknowledgeSettlement {
                settlement_id: reservation,
            })
            .unwrap();
        let credit = state.apply(bid(2, 100, 1)).unwrap().settlements[0].id;

        assert!(matches!(
            state.apply(MarketCommand::AcknowledgeSettlement {
                settlement_id: credit,
            }),
            Err(MarketRejection::CurrencyOverflow)
        ));
        assert_eq!(state.currency_balance(PlayerId(1)), u64::MAX);
        assert_eq!(state.pending_settlements()[0].id, credit);
    }

    #[test]
    fn pure_state_never_constructs_sector_bridge_commands() {
        let mut state = MarketState::default();
        let transition = state.apply(ask(1, 1)).unwrap();
        assert!(matches!(
            transition.settlements[0].effect,
            SettlementEffect::ReserveAsk { .. }
        ));
    }
}
