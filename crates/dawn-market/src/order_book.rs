//! SQLite-backed limit order book (bid/ask matching, ADR-0034 §6).
//!
//! Column encoding for `ItemId` mirrors the flat `item_type`/`module_id`/
//! `ship_type_id` shape `dawn-sector`'s `station_inventory_db.rs` already
//! uses -- easier to eyeball with a sqlite3 CLI, and one fewer encoding to
//! keep in sync. This module can't reuse that code directly (`dawn-market`
//! must not depend on `dawn-sector`, ADR-0034 §4), so the encoding is
//! duplicated here.
//!
//! Same-price priority is time priority: SQLite's own `rowid` (auto-
//! incrementing insertion order) is time priority for free, so orders carry
//! no explicit timestamp/Tick -- this crate has no reason to know about
//! `dawn-core::Tick` at all, keeping it genuinely independent of the Sector
//! tick pipeline (ADR-0034 §4).
//!
//! Self-trading (a player's own bid and ask crossing) is allowed and
//! matches like any other pair -- no special-casing.

use dawn_core::{ItemId, ModuleId, PlayerId, ShipTypeId};
use rusqlite::{params, Connection, OptionalExtension};

/// Which side of the book an order rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderSide {
    Bid,
    Ask,
}

impl OrderSide {
    fn as_column_value(self) -> &'static str {
        match self {
            OrderSide::Bid => "Bid",
            OrderSide::Ask => "Ask",
        }
    }

    fn opposite(self) -> OrderSide {
        match self {
            OrderSide::Bid => OrderSide::Ask,
            OrderSide::Ask => OrderSide::Bid,
        }
    }
}

/// Identifies one resting or historical order (SQLite `rowid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderId(pub i64);

/// One match between a resting order and an incoming order. Trades execute
/// at the resting (maker) order's price, not the incoming (taker) order's
/// price -- standard price-time-priority book convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trade {
    pub buyer: PlayerId,
    pub seller: PlayerId,
    pub item_id: ItemId,
    pub price: u64,
    pub quantity: u64,
}

/// Result of `MarketDb::place_order`: any immediate fills, plus the order
/// left resting on the book (`None` if fully filled immediately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceOrderOutcome {
    pub trades: Vec<Trade>,
    pub resting_order_id: Option<OrderId>,
}

/// The order a `cancel_order` call removed from the book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelledOrder {
    pub item_id: ItemId,
    pub side: OrderSide,
    pub price: u64,
    pub quantity_remaining: u64,
}

fn item_id_to_columns(item_id: ItemId) -> (&'static str, u32, u32) {
    match item_id {
        ItemId::Module(module_id) => ("Module", module_id.0, 0),
        ItemId::PackagedShip(ship_type_id) => ("PackagedShip", 0, ship_type_id.0),
        ItemId::ScrapMetal => ("ScrapMetal", 0, 0),
    }
}

fn columns_to_item_id(item_type: &str, module_id: u32, ship_type_id: u32) -> Option<ItemId> {
    match item_type {
        "Module" => Some(ItemId::Module(ModuleId(module_id))),
        "PackagedShip" => Some(ItemId::PackagedShip(ShipTypeId(ship_type_id))),
        "ScrapMetal" => Some(ItemId::ScrapMetal),
        _ => None,
    }
}

fn columns_to_side(side: &str) -> OrderSide {
    match side {
        "Bid" => OrderSide::Bid,
        "Ask" => OrderSide::Ask,
        other => unreachable!("orders.side check constraint should forbid {other:?}"),
    }
}

/// One resting order row read back while matching.
struct RestingOrder {
    order_id: i64,
    player_id: u64,
    quantity_remaining: u64,
    price: u64,
}

/// The Market's limit order book, backed by SQLite (its own authority,
/// independent of Sector tick determinism, ADR-0034 §4).
pub struct MarketDb {
    conn: Connection,
}

impl MarketDb {
    /// Open (creating if absent) the on-disk database at `path`.
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// A private, non-persistent database -- for tests/demos.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS orders (
                order_id            INTEGER PRIMARY KEY AUTOINCREMENT,
                player_id           INTEGER NOT NULL,
                side                TEXT    NOT NULL CHECK (side IN ('Bid', 'Ask')),
                item_type           TEXT    NOT NULL,
                module_id           INTEGER NOT NULL DEFAULT 0,
                ship_type_id        INTEGER NOT NULL DEFAULT 0,
                price               INTEGER NOT NULL,
                quantity_remaining  INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS orders_matching_idx
                ON orders (item_type, module_id, ship_type_id, side, price, order_id)",
            [],
        )?;
        Ok(Self { conn })
    }

    /// Place a limit order. Matches immediately against any crossing resting
    /// orders on the opposite side (best price first, then insertion order
    /// within a price level), then rests any unfilled remainder on the book.
    pub fn place_order(
        &mut self,
        player_id: PlayerId,
        item_id: ItemId,
        side: OrderSide,
        price: u64,
        quantity: u64,
    ) -> rusqlite::Result<PlaceOrderOutcome> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let tx = self.conn.transaction()?;
        let mut trades = Vec::new();
        let mut remaining = quantity;

        {
            let price_order = match side {
                // A Bid crosses Asks priced at or below it; best (lowest)
                // Ask first.
                OrderSide::Bid => "ASC",
                // An Ask crosses Bids priced at or above it; best (highest)
                // Bid first.
                OrderSide::Ask => "DESC",
            };
            let cmp = match side {
                OrderSide::Bid => "<=",
                OrderSide::Ask => ">=",
            };
            let query = format!(
                "SELECT order_id, player_id, quantity_remaining, price FROM orders
                 WHERE item_type = ?1 AND module_id = ?2 AND ship_type_id = ?3
                   AND side = ?4 AND price {cmp} ?5
                 ORDER BY price {price_order}, order_id ASC"
            );
            let mut stmt = tx.prepare(&query)?;
            let mut rows = stmt.query(params![
                item_type,
                module_id,
                ship_type_id,
                side.opposite().as_column_value(),
                price,
            ])?;

            let mut fills: Vec<(RestingOrder, u64)> = Vec::new();
            while remaining > 0 {
                let Some(row) = rows.next()? else { break };
                let resting = RestingOrder {
                    order_id: row.get(0)?,
                    player_id: row.get(1)?,
                    quantity_remaining: row.get(2)?,
                    price: row.get(3)?,
                };
                let fill_qty = remaining.min(resting.quantity_remaining);
                remaining -= fill_qty;
                fills.push((resting, fill_qty));
            }
            drop(rows);
            drop(stmt);

            for (resting, fill_qty) in fills {
                let new_remaining = resting.quantity_remaining - fill_qty;
                if new_remaining == 0 {
                    tx.execute(
                        "DELETE FROM orders WHERE order_id = ?1",
                        params![resting.order_id],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE orders SET quantity_remaining = ?1 WHERE order_id = ?2",
                        params![new_remaining, resting.order_id],
                    )?;
                }
                let (buyer, seller) = match side {
                    OrderSide::Bid => (player_id, PlayerId(resting.player_id)),
                    OrderSide::Ask => (PlayerId(resting.player_id), player_id),
                };
                trades.push(Trade {
                    buyer,
                    seller,
                    item_id,
                    price: resting.price,
                    quantity: fill_qty,
                });
            }
        }

        let resting_order_id = if remaining > 0 {
            tx.execute(
                "INSERT INTO orders (player_id, side, item_type, module_id, ship_type_id, price, quantity_remaining)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    player_id.raw(),
                    side.as_column_value(),
                    item_type,
                    module_id,
                    ship_type_id,
                    price,
                    remaining,
                ],
            )?;
            Some(OrderId(tx.last_insert_rowid()))
        } else {
            None
        };

        tx.commit()?;
        Ok(PlaceOrderOutcome {
            trades,
            resting_order_id,
        })
    }

    /// Cancel a resting order. Returns `None` if the order doesn't exist or
    /// belongs to a different player (both treated as "not cancellable by
    /// you", same rejection shape as the Station operations in
    /// `dawn-sector` -- no distinct error needed at this layer).
    pub fn cancel_order(
        &mut self,
        player_id: PlayerId,
        order_id: OrderId,
    ) -> rusqlite::Result<Option<CancelledOrder>> {
        let row = self
            .conn
            .query_row(
                "SELECT player_id, side, item_type, module_id, ship_type_id, price, quantity_remaining
                 FROM orders WHERE order_id = ?1",
                params![order_id.0],
                |row| {
                    Ok((
                        row.get::<_, u64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, u64>(5)?,
                        row.get::<_, u64>(6)?,
                    ))
                },
            )
            .optional()?;

        let Some((owner, side, item_type, module_id, ship_type_id, price, quantity_remaining)) =
            row
        else {
            return Ok(None);
        };
        if owner != player_id.raw() {
            return Ok(None);
        }

        self.conn.execute(
            "DELETE FROM orders WHERE order_id = ?1",
            params![order_id.0],
        )?;

        let Some(item_id) = columns_to_item_id(&item_type, module_id, ship_type_id) else {
            return Ok(None);
        };
        Ok(Some(CancelledOrder {
            item_id,
            side: columns_to_side(&side),
            price,
            quantity_remaining,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scrap() -> ItemId {
        ItemId::ScrapMetal
    }

    #[test]
    fn an_order_with_nothing_to_cross_just_rests_on_the_book() {
        let mut market = MarketDb::open_in_memory().unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();
        assert!(outcome.trades.is_empty());
        assert!(outcome.resting_order_id.is_some());
    }

    #[test]
    fn a_crossing_order_fills_at_the_resting_orders_price() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();

        // Buyer bids higher than the ask -- fills at the resting Ask price
        // (100), not the bid price (120).
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 120, 3)
            .unwrap();

        assert_eq!(outcome.trades.len(), 1);
        let trade = outcome.trades[0];
        assert_eq!(trade.buyer, PlayerId(2));
        assert_eq!(trade.seller, PlayerId(1));
        assert_eq!(trade.price, 100);
        assert_eq!(trade.quantity, 3);
        assert!(outcome.resting_order_id.is_none());
    }

    #[test]
    fn partial_fill_leaves_the_remainder_resting() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();

        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 8)
            .unwrap();

        assert_eq!(outcome.trades.len(), 1);
        assert_eq!(outcome.trades[0].quantity, 5);
        // 8 - 5 = 3 units of the incoming Bid rest on the book.
        assert!(outcome.resting_order_id.is_some());
    }

    #[test]
    fn best_price_is_matched_before_a_worse_price_at_the_same_level() {
        let mut market = MarketDb::open_in_memory().unwrap();
        // Two Asks: 110 first, then a cheaper 100.
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 110, 5)
            .unwrap();
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();

        let outcome = market
            .place_order(PlayerId(3), scrap(), OrderSide::Bid, 110, 5)
            .unwrap();

        // The cheaper Ask (100) is the better price for a buyer and must
        // fill first, even though it was listed second.
        assert_eq!(outcome.trades.len(), 1);
        assert_eq!(outcome.trades[0].seller, PlayerId(2));
        assert_eq!(outcome.trades[0].price, 100);
    }

    #[test]
    fn time_priority_breaks_ties_at_the_same_price() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();

        let outcome = market
            .place_order(PlayerId(3), scrap(), OrderSide::Bid, 100, 5)
            .unwrap();

        // Both Asks are priced equally -- the first one listed (player 1)
        // must fill first.
        assert_eq!(outcome.trades.len(), 1);
        assert_eq!(outcome.trades[0].seller, PlayerId(1));
    }

    #[test]
    fn a_non_crossing_order_does_not_match() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();

        // Bid below the Ask -- no cross.
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 90, 5)
            .unwrap();

        assert!(outcome.trades.is_empty());
        assert!(outcome.resting_order_id.is_some());
    }

    #[test]
    fn self_trading_is_allowed_and_matches_like_any_other_pair() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();

        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Bid, 100, 5)
            .unwrap();

        assert_eq!(outcome.trades.len(), 1);
        assert_eq!(outcome.trades[0].buyer, PlayerId(1));
        assert_eq!(outcome.trades[0].seller, PlayerId(1));
    }

    #[test]
    fn different_items_never_cross() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(
                PlayerId(1),
                ItemId::PackagedShip(ShipTypeId(7)),
                OrderSide::Ask,
                100,
                5,
            )
            .unwrap();

        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 5)
            .unwrap();

        assert!(outcome.trades.is_empty());
    }

    #[test]
    fn cancel_order_removes_it_and_returns_its_details() {
        let mut market = MarketDb::open_in_memory().unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();
        let order_id = outcome.resting_order_id.unwrap();

        let cancelled = market.cancel_order(PlayerId(1), order_id).unwrap();
        assert_eq!(
            cancelled,
            Some(CancelledOrder {
                item_id: scrap(),
                side: OrderSide::Ask,
                price: 100,
                quantity_remaining: 5,
            })
        );

        // Cancelled order no longer rests on the book -- a crossing Bid
        // finds nothing to match.
        let after = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 5)
            .unwrap();
        assert!(after.trades.is_empty());
    }

    #[test]
    fn cancel_order_rejects_a_player_who_does_not_own_it() {
        let mut market = MarketDb::open_in_memory().unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap();
        let order_id = outcome.resting_order_id.unwrap();

        let result = market.cancel_order(PlayerId(2), order_id).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn cancel_order_returns_none_for_an_unknown_order_id() {
        let mut market = MarketDb::open_in_memory().unwrap();
        let result = market.cancel_order(PlayerId(1), OrderId(999)).unwrap();
        assert_eq!(result, None);
    }
}
