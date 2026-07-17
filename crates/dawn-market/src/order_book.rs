//! SQLite-backed limit order book + Currency escrow (ADR-0034 §5/§6).
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
//!
//! # Currency escrow (9D-3)
//!
//! A `Bid` reserves `price * quantity` from the buyer's Currency balance
//! the moment it's placed (`orders.escrowed_currency`), so a player can
//! never place more Bids than they can actually pay for even before any of
//! them fill -- without escrow, several simultaneous Bids could each pass
//! a balance check individually and then all fill together, overdrawing
//! the balance. An `Ask` escrows nothing: it sells an Item the seller
//! already holds (bridged into Market via 9D-4's `RemoveItemCommand`, not
//! this crate's concern), so there's no Currency of the seller's to hold.
//!
//! Trades always settle at the resting (maker) order's price. When a Bid
//! crosses a cheaper resting Ask, the buyer's escrow (reserved at their own
//! bid price) exceeds the true cost -- the difference is refunded
//! immediately. When an Ask fills a resting Bid, the resting Bid's escrow
//! already equals exactly `maker_price * quantity`, so it's paid out and
//! consumed with no refund needed.

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

/// `place_order` for a `Bid` whose `price * quantity` exceeds the caller's
/// current Currency balance. The order is not placed and nothing is
/// escrowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsufficientBalance;

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

/// Current Currency balance for `player_id`, `0` if they have no row yet.
fn currency_balance_raw(conn: &Connection, player_id: u64) -> rusqlite::Result<u64> {
    let balance = conn
        .query_row(
            "SELECT balance FROM currency WHERE player_id = ?1",
            params![player_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(balance.unwrap_or(0))
}

fn credit_currency_raw(conn: &Connection, player_id: u64, amount: u64) -> rusqlite::Result<()> {
    if amount == 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO currency (player_id, balance) VALUES (?1, ?2)
         ON CONFLICT(player_id) DO UPDATE SET balance = balance + excluded.balance",
        params![player_id, amount],
    )?;
    Ok(())
}

/// Attempts to move `amount` out of `player_id`'s balance. Returns `false`
/// (leaving the balance untouched) if it's insufficient -- the `CHECK
/// (balance >= 0)` constraint is the last line of defense, but the `WHERE
/// balance >= ?2` guard means that constraint should never actually fire.
fn try_debit_currency_raw(
    conn: &Connection,
    player_id: u64,
    amount: u64,
) -> rusqlite::Result<bool> {
    if amount == 0 {
        return Ok(true);
    }
    conn.execute(
        "INSERT INTO currency (player_id, balance) VALUES (?1, 0) ON CONFLICT(player_id) DO NOTHING",
        params![player_id],
    )?;
    let changed = conn.execute(
        "UPDATE currency SET balance = balance - ?2 WHERE player_id = ?1 AND balance >= ?2",
        params![player_id, amount],
    )?;
    Ok(changed == 1)
}

/// One resting order row read back while matching.
struct RestingOrder {
    order_id: i64,
    player_id: u64,
    quantity_remaining: u64,
    price: u64,
    escrowed_currency: u64,
}

/// The Market's limit order book + Currency ledger, backed by SQLite (its
/// own authority, independent of Sector tick determinism, ADR-0034 §4/§5).
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
                quantity_remaining  INTEGER NOT NULL,
                escrowed_currency   INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS orders_matching_idx
                ON orders (item_type, module_id, ship_type_id, side, price, order_id)",
            [],
        )?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS currency (
                player_id  INTEGER PRIMARY KEY,
                balance    INTEGER NOT NULL DEFAULT 0 CHECK (balance >= 0)
            )",
            [],
        )?;
        Ok(Self { conn })
    }

    /// Current Currency balance, `0` if `player_id` has never held any.
    pub fn currency_balance(&self, player_id: PlayerId) -> rusqlite::Result<u64> {
        currency_balance_raw(&self.conn, player_id.raw())
    }

    /// Grant `amount` Currency to `player_id` (e.g. an initial stipend, or
    /// a future non-Market credit source). Not used by trading itself --
    /// `place_order`/`cancel_order` move escrowed Currency internally.
    pub fn credit_currency(&mut self, player_id: PlayerId, amount: u64) -> rusqlite::Result<()> {
        credit_currency_raw(&self.conn, player_id.raw(), amount)
    }

    /// Place a limit order. Matches immediately against any crossing resting
    /// orders on the opposite side (best price first, then insertion order
    /// within a price level), then rests any unfilled remainder on the book.
    ///
    /// A `Bid` first escrows `price * quantity` from the caller's Currency
    /// balance; if that fails, the order is never placed and `Err`
    /// (`InsufficientBalance`) is returned -- everything else in this
    /// signature's `Result` layering is a genuine `rusqlite::Error`.
    pub fn place_order(
        &mut self,
        player_id: PlayerId,
        item_id: ItemId,
        side: OrderSide,
        price: u64,
        quantity: u64,
    ) -> rusqlite::Result<Result<PlaceOrderOutcome, InsufficientBalance>> {
        let (item_type, module_id, ship_type_id) = item_id_to_columns(item_id);
        let tx = self.conn.transaction()?;

        if side == OrderSide::Bid {
            let cost = price * quantity;
            if !try_debit_currency_raw(&tx, player_id.raw(), cost)? {
                // Dropping `tx` here rolls back -- nothing else has
                // happened yet, so there's nothing else to undo.
                return Ok(Err(InsufficientBalance));
            }
        }

        let mut trades = Vec::new();
        let mut remaining = quantity;
        let mut refund_to_buyer: u64 = 0;

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
                "SELECT order_id, player_id, quantity_remaining, price, escrowed_currency
                 FROM orders
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
                    escrowed_currency: row.get(4)?,
                };
                let fill_qty = remaining.min(resting.quantity_remaining);
                remaining -= fill_qty;
                fills.push((resting, fill_qty));
            }
            drop(rows);
            drop(stmt);

            for (resting, fill_qty) in fills {
                let new_remaining = resting.quantity_remaining - fill_qty;
                let new_escrowed = resting
                    .escrowed_currency
                    .saturating_sub(resting.price * fill_qty);
                if new_remaining == 0 {
                    tx.execute(
                        "DELETE FROM orders WHERE order_id = ?1",
                        params![resting.order_id],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE orders SET quantity_remaining = ?1, escrowed_currency = ?2
                         WHERE order_id = ?3",
                        params![new_remaining, new_escrowed, resting.order_id],
                    )?;
                }

                let (buyer, seller) = match side {
                    OrderSide::Bid => (player_id, PlayerId(resting.player_id)),
                    OrderSide::Ask => (PlayerId(resting.player_id), player_id),
                };
                let proceeds = resting.price * fill_qty;
                credit_currency_raw(&tx, seller.raw(), proceeds)?;
                if side == OrderSide::Bid {
                    // The buyer escrowed at their own (possibly higher) bid
                    // price; the resting Ask's price is always <= that, so
                    // this is always >= 0.
                    refund_to_buyer += (price - resting.price) * fill_qty;
                }
                // An incoming Ask fills at the resting Bid's own escrowed
                // price exactly -- no refund side to compute.

                trades.push(Trade {
                    buyer,
                    seller,
                    item_id,
                    price: resting.price,
                    quantity: fill_qty,
                });
            }
        }

        if refund_to_buyer > 0 {
            credit_currency_raw(&tx, player_id.raw(), refund_to_buyer)?;
        }

        let resting_order_id = if remaining > 0 {
            let escrowed_currency = if side == OrderSide::Bid {
                price * remaining
            } else {
                0
            };
            tx.execute(
                "INSERT INTO orders
                    (player_id, side, item_type, module_id, ship_type_id, price, quantity_remaining, escrowed_currency)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    player_id.raw(),
                    side.as_column_value(),
                    item_type,
                    module_id,
                    ship_type_id,
                    price,
                    remaining,
                    escrowed_currency,
                ],
            )?;
            Some(OrderId(tx.last_insert_rowid()))
        } else {
            None
        };

        tx.commit()?;
        Ok(Ok(PlaceOrderOutcome {
            trades,
            resting_order_id,
        }))
    }

    /// Cancel a resting order, refunding any Currency it had escrowed
    /// (`Bid` only -- an `Ask` never escrowed any). Returns `None` if the
    /// order doesn't exist or belongs to a different player (both treated
    /// as "not cancellable by you", same rejection shape as the Station
    /// operations in `dawn-sector` -- no distinct error needed at this
    /// layer).
    pub fn cancel_order(
        &mut self,
        player_id: PlayerId,
        order_id: OrderId,
    ) -> rusqlite::Result<Option<CancelledOrder>> {
        let tx = self.conn.transaction()?;
        let row = tx
            .query_row(
                "SELECT player_id, side, item_type, module_id, ship_type_id, price,
                        quantity_remaining, escrowed_currency
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
                        row.get::<_, u64>(7)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            owner,
            side,
            item_type,
            module_id,
            ship_type_id,
            price,
            quantity_remaining,
            escrowed_currency,
        )) = row
        else {
            return Ok(None);
        };
        if owner != player_id.raw() {
            return Ok(None);
        }

        tx.execute(
            "DELETE FROM orders WHERE order_id = ?1",
            params![order_id.0],
        )?;
        if escrowed_currency > 0 {
            credit_currency_raw(&tx, owner, escrowed_currency)?;
        }
        tx.commit()?;

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
            .unwrap()
            .unwrap();
        assert!(outcome.trades.is_empty());
        assert!(outcome.resting_order_id.is_some());
    }

    #[test]
    fn a_crossing_order_fills_at_the_resting_orders_price() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        // Buyer bids higher than the ask -- fills at the resting Ask price
        // (100), not the bid price (120).
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 120, 3)
            .unwrap()
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
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 8)
            .unwrap()
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
            .unwrap()
            .unwrap();
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(3), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(3), scrap(), OrderSide::Bid, 110, 5)
            .unwrap()
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
            .unwrap()
            .unwrap();
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(3), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(3), scrap(), OrderSide::Bid, 100, 5)
            .unwrap()
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
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        // Bid below the Ask -- no cross.
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 90, 5)
            .unwrap()
            .unwrap();

        assert!(outcome.trades.is_empty());
        assert!(outcome.resting_order_id.is_some());
    }

    #[test]
    fn self_trading_is_allowed_and_matches_like_any_other_pair() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(1), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Bid, 100, 5)
            .unwrap()
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
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 5)
            .unwrap()
            .unwrap();

        assert!(outcome.trades.is_empty());
    }

    #[test]
    fn cancel_order_removes_it_and_returns_its_details() {
        let mut market = MarketDb::open_in_memory().unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
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
        market.credit_currency(PlayerId(2), 1000).unwrap();
        let after = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 5)
            .unwrap()
            .unwrap();
        assert!(after.trades.is_empty());
    }

    #[test]
    fn cancel_order_rejects_a_player_who_does_not_own_it() {
        let mut market = MarketDb::open_in_memory().unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
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

    // -- Currency escrow (9D-3) ---------------------------------------------

    #[test]
    fn a_new_player_has_a_zero_balance() {
        let market = MarketDb::open_in_memory().unwrap();
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 0);
    }

    #[test]
    fn credit_currency_increases_the_balance() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market.credit_currency(PlayerId(1), 500).unwrap();
        market.credit_currency(PlayerId(1), 250).unwrap();
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 750);
    }

    #[test]
    fn placing_a_bid_escrows_price_times_quantity() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market.credit_currency(PlayerId(1), 1000).unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Bid, 100, 4)
            .unwrap()
            .unwrap();
        // 1000 - (100 * 4) = 600.
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 600);
    }

    #[test]
    fn a_bid_exceeding_the_balance_is_rejected_and_escrows_nothing() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market.credit_currency(PlayerId(1), 100).unwrap();
        let result = market
            .place_order(PlayerId(1), scrap(), OrderSide::Bid, 100, 4)
            .unwrap();
        assert_eq!(result, Err(InsufficientBalance));
        // Balance untouched -- nothing was placed or escrowed.
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 100);
    }

    #[test]
    fn an_ask_does_not_require_or_touch_the_sellers_balance() {
        let mut market = MarketDb::open_in_memory().unwrap();
        // Seller has zero Currency -- an Ask must still be placeable.
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();
        assert!(outcome.resting_order_id.is_some());
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 0);
    }

    #[test]
    fn cancelling_a_bid_refunds_its_escrow() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market.credit_currency(PlayerId(1), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(1), scrap(), OrderSide::Bid, 100, 4)
            .unwrap()
            .unwrap();
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 600);

        market
            .cancel_order(PlayerId(1), outcome.resting_order_id.unwrap())
            .unwrap();
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 1000);
    }

    #[test]
    fn a_fill_credits_the_seller_at_the_makers_price() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 5)
            .unwrap()
            .unwrap();

        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 500);
        assert_eq!(market.currency_balance(PlayerId(2)).unwrap(), 500);
    }

    #[test]
    fn a_bid_crossing_a_cheaper_ask_is_refunded_the_price_improvement() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 5)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        // Bid at 120 crosses the 100 Ask -- escrows 600 (120*5) up front,
        // fills at 100, so 100 (5*(120-100)) should come back.
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 120, 5)
            .unwrap()
            .unwrap();

        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 500);
        // 1000 - 600 (escrow) + 100 (refund) = 500.
        assert_eq!(market.currency_balance(PlayerId(2)).unwrap(), 500);
    }

    #[test]
    fn an_ask_filling_a_resting_bid_consumes_its_escrow_exactly() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market.credit_currency(PlayerId(1), 1000).unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Bid, 100, 5)
            .unwrap()
            .unwrap();
        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 500);

        // Seller's Ask at 90 still fills at the resting Bid's price (100).
        market
            .place_order(PlayerId(2), scrap(), OrderSide::Ask, 90, 5)
            .unwrap()
            .unwrap();

        assert_eq!(market.currency_balance(PlayerId(1)).unwrap(), 500);
        assert_eq!(market.currency_balance(PlayerId(2)).unwrap(), 500);
    }

    #[test]
    fn a_partially_filled_bids_leftover_escrow_matches_its_remaining_quantity() {
        let mut market = MarketDb::open_in_memory().unwrap();
        market
            .place_order(PlayerId(1), scrap(), OrderSide::Ask, 100, 3)
            .unwrap()
            .unwrap();

        market.credit_currency(PlayerId(2), 1000).unwrap();
        let outcome = market
            .place_order(PlayerId(2), scrap(), OrderSide::Bid, 100, 8)
            .unwrap()
            .unwrap();
        // 3 filled at 100 (=300), 5 remain resting with 500 still escrowed.
        assert_eq!(market.currency_balance(PlayerId(2)).unwrap(), 200);

        market
            .cancel_order(PlayerId(2), outcome.resting_order_id.unwrap())
            .unwrap();
        // The remaining 500 escrow comes back on cancel.
        assert_eq!(market.currency_balance(PlayerId(2)).unwrap(), 700);
    }
}
