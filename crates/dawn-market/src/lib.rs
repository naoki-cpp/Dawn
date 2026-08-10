//! Player-to-player Market domain and persistence boundary.
//!
//! This leaf crate owns the pure bid/ask policy, Currency escrow, and durable
//! settlement outbox. It deliberately does not depend on Sector commands or a
//! runtime handle. The caller delivers [`SettlementIntent`] values to the
//! Sector that owns the affected ship and acknowledges them after application.
//!
//! ```
//! use dawn_core::{ItemId, ModuleId, PlayerId, ShipId};
//! use dawn_market::{MarketCommand, MarketDb, OrderSide};
//!
//! let mut market = MarketDb::open_in_memory().unwrap();
//! let item = ItemId::Module(ModuleId(1));
//!
//! // An Ask first creates a durable cargo-reservation intent. It is not
//! // visible to matching until Sector acknowledges that intent.
//! let listed = market.execute(MarketCommand::PlaceOrder {
//!     player_id: PlayerId(1),
//!     ship_id: ShipId::new(dawn_core::NodeId(0), 1),
//!     item_id: item,
//!     side: OrderSide::Ask,
//!     price: 100,
//!     quantity: 5,
//! }).unwrap();
//! assert_eq!(listed.settlements.len(), 1);
//!
//! market.execute(MarketCommand::AcknowledgeSettlement {
//!     settlement_id: listed.settlements[0].id,
//! }).unwrap();
//! ```
//!
//! The earlier 9D slices established the crate boundary, order book, Currency
//! ledger, and Station-only runtime surface. #279 completes that boundary by
//! keeping SQL in [`MarketDb`] and committing orders, balances, and settlement
//! intents in one transaction. Sector command translation belongs to the
//! `dawn-simulation` serve adapter.

mod matching;
mod order_book;
mod repository;

pub use order_book::{
    MarketCommand, MarketCommandResult, MarketEvent, MarketOrderView, MarketRejection, MarketState,
    MarketTransition, OrderId, OrderSide, OrderStatus, SettlementEffect, SettlementId,
    SettlementIntent, SettlementRecord, SettlementStatus,
};
pub use repository::{MarketDb, MarketError};
