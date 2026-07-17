//! Player-to-player Market (ADR-0034 §4/§5/§6).
//!
//! This crate owns the bid/ask order book and the `PlayerId`-keyed Currency
//! ledger ([`MarketDb`]) -- both backed by their own SQLite database, a
//! separate authority from the Sector's event-sourced state, deliberately
//! outside the Sector tick's determinism constraints (INV-001/002/005). It
//! bridges into Sector-owned state (a ship's `InventoryComp`) only through
//! three one-sided `dawn-core` commands (`RemoveItemCommand` on List,
//! `ReturnItemCommand` on Cancel, `CreditItemCommand` on Settle, 9D-4) --
//! this crate constructs those commands, but never applies them itself; the
//! caller (currently `dawn-simulation`, see roadmap.md §12 9D-1) is
//! responsible for routing each one to whichever `SimulationNode` currently
//! owns the affected ship.
//!
//! ```
//! use dawn_core::{ItemId, ModuleId, PlayerId, ShipId};
//! use dawn_market::{MarketDb, OrderSide};
//!
//! let mut market = MarketDb::open_in_memory().unwrap();
//! let item = ItemId::Module(ModuleId(1));
//!
//! // Seller lists 5 units at 100 -- no Currency required for an Ask.
//! let listed = market.place_order(PlayerId(1), ShipId::new(dawn_core::NodeId(0), 1), item, OrderSide::Ask, 100, 5).unwrap().unwrap();
//! assert!(listed.trades.is_empty());
//!
//! // Buyer needs Currency to place a Bid -- it's escrowed on placement.
//! market.credit_currency(PlayerId(2), 1000).unwrap();
//! let bought = market.place_order(PlayerId(2), ShipId::new(dawn_core::NodeId(0), 2), item, OrderSide::Bid, 100, 3).unwrap().unwrap();
//! assert_eq!(bought.trades.len(), 1);
//! assert_eq!(bought.trades[0].price, 100);
//! assert_eq!(bought.trades[0].quantity, 3);
//!
//! // Trade settles immediately: seller paid, buyer's escrow consumed.
//! assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 300);
//! assert_eq!(market.currency_balance(PlayerId(2)).unwrap(), 700);
//! ```
//!
//! # Scope of this crate today
//!
//! 9D-1 established this crate's place in the Dependency DAG (leaf,
//! `dawn-core` + serde + rusqlite only, same position as `dawn-wire`). 9D-2
//! added the order book. 9D-3 (this slice) adds Currency escrow/settlement.
//! 9D-4 adds the three bridging command outputs: listing returns a
//! `RemoveItemCommand`, Ask cancellation returns a `ReturnItemCommand`, and
//! settlement returns one or more `CreditItemCommand`s. This crate still
//! constructs those commands but never applies them itself.

mod order_book;

pub use order_book::{
    CancelledOrder, InsufficientBalance, MarketDb, OrderId, OrderSide, PlaceOrderOutcome, Trade,
};
