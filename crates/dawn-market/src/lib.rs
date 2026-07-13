//! Player-to-player Market (ADR-0034 §4/§5/§6).
//!
//! This crate owns the bid/ask order book ([`MarketDb`]) and will own the
//! `PlayerId`-keyed Currency ledger (9D-3) -- both backed by their own
//! SQLite database, a separate authority from the Sector's event-sourced
//! state, deliberately outside the Sector tick's determinism constraints
//! (INV-001/002/005). It bridges into Sector-owned state (a ship's
//! `InventoryComp`) only through three one-sided `dawn-core` commands
//! (`RemoveItemCommand` on List, `ReturnItemCommand` on Cancel,
//! `CreditItemCommand` on Settle, 9D-4) -- this crate constructs those
//! commands, but never applies them itself; the caller (currently
//! `dawn-simulation`, see roadmap.md §12 9D-1) is responsible for routing
//! each one to whichever `SimulationNode` currently owns the affected ship.
//!
//! ```
//! use dawn_core::{ItemId, ModuleId, PlayerId};
//! use dawn_market::{MarketDb, OrderSide};
//!
//! let mut market = MarketDb::open_in_memory().unwrap();
//! let item = ItemId::Module(ModuleId(1));
//!
//! // Seller lists 5 units at 100.
//! let listed = market.place_order(PlayerId(1), item, OrderSide::Ask, 100, 5).unwrap();
//! assert!(listed.trades.is_empty());
//!
//! // Buyer crosses it: fills at the resting (seller's) price.
//! let bought = market.place_order(PlayerId(2), item, OrderSide::Bid, 100, 3).unwrap();
//! assert_eq!(bought.trades.len(), 1);
//! assert_eq!(bought.trades[0].price, 100);
//! assert_eq!(bought.trades[0].quantity, 3);
//! ```
//!
//! # Scope of this crate today
//!
//! 9D-1 established this crate's place in the Dependency DAG (leaf,
//! `dawn-core` + serde + rusqlite only, same position as `dawn-wire`). 9D-2
//! (this slice) adds the order book. The Currency ledger and the three
//! bridging commands are 9D-3/9D-4 follow-up work (roadmap.md §12).

mod order_book;

pub use order_book::{CancelledOrder, MarketDb, OrderId, OrderSide, PlaceOrderOutcome, Trade};
