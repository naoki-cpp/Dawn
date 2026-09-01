//! SQLite adapter for the pure Market transition policy.
//!
//! All SQL lives in this module. A command loads a bounded working set,
//! applies one pure transition, and persists only its changed rows in one
//! SQLite transaction.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use dawn_core::{EntityId, ItemId, PlayerId, ShipId};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::order_book::{
    MarketCommand, MarketOrderView, MarketRejection, MarketState, MarketTransition, Order, OrderId,
    OrderSide, OrderStatus, SettlementEffect, SettlementId, SettlementIntent, SettlementRecord,
    SettlementStatus,
};

const SCHEMA_VERSION: i64 = 1;
const MAX_ORDER_VIEW: usize = 200;
const MAX_MATCH_CANDIDATES: usize = 10_000;
const MAX_SETTLEMENT_VIEW: usize = 1_000;
const NEXT_ORDER_ID_KEY: &str = "next_order_id";
const NEXT_SETTLEMENT_ID_KEY: &str = "next_settlement_id";

/// Storage/application error at the Market boundary.
#[derive(Debug, thiserror::Error)]
pub enum MarketError {
    #[error("Market policy rejected the command: {0}")]
    Rejected(#[from] MarketRejection),
    #[error("Market storage failed: {0}")]
    Storage(#[from] rusqlite::Error),
}

/// SQLite-backed Market application boundary.
pub struct MarketDb {
    conn: Connection,
    pending_cursor: Cell<i64>,
}

impl std::fmt::Debug for MarketDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarketDb").finish_non_exhaustive()
    }
}

impl MarketDb {
    /// Open a Market database, replacing pre-#279 schema versions because
    /// backward compatibility is intentionally not part of this pre-release.
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// Open an isolated Market database for tests and local demos.
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Apply one pure Market command and atomically persist its bounded
    /// working-set delta and any newly-created outbox entries.
    pub fn execute(&mut self, command: MarketCommand) -> Result<MarketTransition, MarketError> {
        let tx = self.conn.transaction()?;
        let mut state = load_working_state(&tx, &command)?;
        let before = snapshot_state(&state);
        let transition = state.apply(command)?;
        persist_state_delta(&tx, &before, &snapshot_state(&state))?;
        tx.commit()?;
        Ok(transition)
    }

    /// Current Currency balance, or zero when no row exists.
    pub fn currency_balance(&self, player_id: PlayerId) -> rusqlite::Result<u64> {
        self.conn
            .query_row(
                "SELECT balance FROM currency WHERE player_id = ?1",
                params![to_i64(player_id.raw())?],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map(|value| value.unwrap_or(0).max(0) as u64)
    }

    /// Convenience wrapper for controlled funding in tests/bootstrap code.
    pub fn credit_currency(
        &mut self,
        player_id: PlayerId,
        amount: u64,
    ) -> Result<MarketTransition, MarketError> {
        self.execute(MarketCommand::CreditCurrency { player_id, amount })
    }

    /// Return a bounded stable snapshot of the shared public order book.
    ///
    /// `player_id` is retained for the existing call shape; it does not filter
    /// the result to the caller's own orders.
    pub fn open_orders_for(&self, _player_id: PlayerId) -> rusqlite::Result<Vec<MarketOrderView>> {
        let mut stmt = self.conn.prepare(
            "SELECT order_id, player_id, ship_id, item_type, module_id, ship_type_id,
                    side, price, quantity_remaining
             FROM orders
             WHERE status = ?1
             ORDER BY order_id
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![status_as_str(OrderStatus::Open), MAX_ORDER_VIEW as i64],
            |row| {
                Ok(MarketOrderView {
                    order_id: OrderId(row.get(0)?),
                    player_id: PlayerId(from_i64(row.get(1)?)?),
                    ship_id: ShipId(EntityId::from_raw(from_i64(row.get(2)?)?)),
                    item_id: item_from_row(row, 3)?,
                    side: side_from_str(&row.get::<_, String>(6)?)?,
                    price: from_i64(row.get(7)?)?,
                    quantity_remaining: from_i64(row.get(8)?)?,
                })
            },
        )?;
        rows.collect()
    }

    /// Return a bounded pending settlement page in stable cyclic ID order.
    ///
    /// The cursor advances after every page so a permanently unroutable early
    /// row cannot starve later rows in the outbox. Each SQL query remains
    /// explicitly bounded and ordered; the wraparound only joins two such
    /// ordered ranges into one cyclic page.
    pub fn pending_settlements(&self) -> rusqlite::Result<Vec<SettlementIntent>> {
        let cursor = self.pending_cursor.get();
        let mut records = load_pending_settlements_after(&self.conn, cursor, MAX_SETTLEMENT_VIEW)?;
        if records.len() < MAX_SETTLEMENT_VIEW {
            let remaining = MAX_SETTLEMENT_VIEW - records.len();
            records.extend(load_pending_settlements_at_most(
                &self.conn, cursor, remaining,
            )?);
        }
        if let Some(record) = records.last() {
            self.pending_cursor.set(record.id.0);
        }
        Ok(records
            .into_iter()
            .map(|record| SettlementIntent {
                id: record.id,
                effect: record.effect,
            })
            .collect())
    }

    /// Inspect a settlement after a delivery attempt.
    pub fn settlement(
        &self,
        settlement_id: SettlementId,
    ) -> rusqlite::Result<Option<SettlementRecord>> {
        self.conn
            .query_row(
                "SELECT settlement_id, effect, status, compensation_for, player_id, ship_id,
                        item_type, module_id, ship_type_id, quantity, order_id,
                        seller_player_id, seller_ship_id, price, last_error
                 FROM settlements WHERE settlement_id = ?1",
                params![settlement_id.0],
                load_settlement_row,
            )
            .optional()
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            conn.execute_batch(
                "DROP TABLE IF EXISTS settlements;
                 DROP TABLE IF EXISTS orders;
                 DROP TABLE IF EXISTS currency;
                 DROP TABLE IF EXISTS market_meta;
                 PRAGMA user_version = 1;",
            )?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS orders (
                order_id               INTEGER PRIMARY KEY,
                player_id              INTEGER NOT NULL,
                ship_id                INTEGER NOT NULL,
                side                   TEXT NOT NULL CHECK (side IN ('Bid', 'Ask')),
                item_type              TEXT NOT NULL,
                module_id              INTEGER NOT NULL,
                ship_type_id           INTEGER NOT NULL,
                price                  INTEGER NOT NULL,
                quantity_remaining     INTEGER NOT NULL,
                escrowed_currency      INTEGER NOT NULL,
                status                 TEXT NOT NULL CHECK (status IN ('PendingReservation', 'Open')),
                reservation_id         INTEGER
            );
            CREATE INDEX IF NOT EXISTS orders_matching_asc_idx
                ON orders (item_type, module_id, ship_type_id, side, status, price, order_id);
            CREATE INDEX IF NOT EXISTS orders_matching_desc_idx
                ON orders (item_type, module_id, ship_type_id, side, status, price DESC, order_id ASC);
            CREATE INDEX IF NOT EXISTS orders_open_book_idx
                ON orders (status, order_id);
            DROP INDEX IF EXISTS orders_matching_idx;
            DROP INDEX IF EXISTS orders_matching_status_idx;
            CREATE TABLE IF NOT EXISTS currency (
                player_id INTEGER PRIMARY KEY,
                balance   INTEGER NOT NULL CHECK (balance >= 0)
            );
            CREATE TABLE IF NOT EXISTS settlements (
                settlement_id       INTEGER PRIMARY KEY,
                effect              TEXT NOT NULL CHECK (effect IN ('ReserveAsk', 'ReturnItem', 'CreditItem')),
                status              TEXT NOT NULL CHECK (status IN ('Pending', 'Applied', 'Compensating', 'Terminal', 'Compensated')),
                compensation_for    INTEGER,
                player_id           INTEGER NOT NULL,
                ship_id             INTEGER NOT NULL,
                item_type           TEXT NOT NULL,
                module_id           INTEGER NOT NULL,
                ship_type_id        INTEGER NOT NULL,
                quantity            INTEGER NOT NULL,
                order_id            INTEGER,
                seller_player_id    INTEGER,
                seller_ship_id      INTEGER,
                price               INTEGER,
                last_error          TEXT
            );
            CREATE INDEX IF NOT EXISTS settlements_pending_idx
                ON settlements (status, settlement_id);
            CREATE TABLE IF NOT EXISTS market_meta (
                key   TEXT PRIMARY KEY,
                value INTEGER NOT NULL
            );",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO market_meta (key, value) VALUES (?1, ?2)",
            params![NEXT_ORDER_ID_KEY, 1_i64],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO market_meta (key, value) VALUES (?1, ?2)",
            params![NEXT_SETTLEMENT_ID_KEY, 1_i64],
        )?;
        Ok(Self {
            conn,
            pending_cursor: Cell::new(0),
        })
    }
}

struct WorkingSet {
    orders: BTreeMap<OrderId, Order>,
    balances: BTreeMap<PlayerId, u64>,
    settlements: BTreeMap<SettlementId, SettlementRecord>,
    next_order_id: i64,
    next_settlement_id: i64,
}

fn snapshot_state(state: &MarketState) -> WorkingSet {
    WorkingSet {
        orders: state.orders().map(|order| (order.id, *order)).collect(),
        balances: state
            .balances()
            .map(|(player, balance)| (*player, *balance))
            .collect(),
        settlements: state
            .settlements()
            .map(|record| (record.id, record.clone()))
            .collect(),
        next_order_id: state.next_ids().0,
        next_settlement_id: state.next_ids().1,
    }
}

fn load_working_state(
    tx: &Transaction<'_>,
    command: &MarketCommand,
) -> rusqlite::Result<MarketState> {
    let mut orders = BTreeMap::new();
    let mut balances = BTreeMap::new();
    let mut settlements = BTreeMap::new();

    match command {
        MarketCommand::PlaceOrder {
            player_id,
            item_id,
            side: OrderSide::Bid,
            price,
            quantity,
            ..
        } => {
            load_balance(tx, *player_id, &mut balances)?;
            load_crossing_orders(tx, *item_id, OrderSide::Bid, *price, *quantity, &mut orders)?;
        }
        MarketCommand::PlaceOrder { .. } => {}
        MarketCommand::CancelOrder { order_id, .. } => {
            load_order(tx, *order_id, &mut orders)?;
            if let Some(order) = orders.get(order_id) {
                if order.escrowed_currency > 0 {
                    load_balance(tx, order.player_id, &mut balances)?;
                }
            }
        }
        MarketCommand::AcknowledgeSettlement { settlement_id }
        | MarketCommand::RejectSettlement { settlement_id, .. } => {
            let Some(record) = load_settlement(tx, *settlement_id, &mut settlements)? else {
                return Ok(MarketState::from_parts(
                    orders,
                    balances,
                    settlements,
                    next_id(tx, NEXT_ORDER_ID_KEY)?,
                    next_id(tx, NEXT_SETTLEMENT_ID_KEY)?,
                ));
            };
            let is_pending = record.status == SettlementStatus::Pending;
            let is_acknowledgement = matches!(command, MarketCommand::AcknowledgeSettlement { .. });
            match record.effect {
                SettlementEffect::ReserveAsk { order_id, .. } => {
                    if is_pending {
                        load_order(tx, order_id, &mut orders)?;
                    }
                    if is_pending && is_acknowledgement {
                        if let Some(order) = orders.get(&order_id).copied() {
                            load_crossing_orders(
                                tx,
                                order.item_id,
                                OrderSide::Ask,
                                order.price,
                                order.quantity_remaining,
                                &mut orders,
                            )?;
                        }
                    }
                }
                SettlementEffect::ReturnItem { .. } => {
                    if is_pending {
                        if let Some(parent_id) = settlements
                            .get(settlement_id)
                            .and_then(|record| record.compensation_for)
                        {
                            load_settlement(tx, parent_id, &mut settlements)?;
                        }
                    }
                }
                SettlementEffect::CreditItem { seller, .. } if is_pending && is_acknowledgement => {
                    load_balance(tx, seller, &mut balances)?;
                }
                SettlementEffect::CreditItem { buyer, .. } if is_pending => {
                    load_balance(tx, buyer, &mut balances)?;
                }
                SettlementEffect::CreditItem { .. } => {}
            }
        }
        MarketCommand::CreditCurrency { player_id, .. } => {
            load_balance(tx, *player_id, &mut balances)?;
        }
    }

    Ok(MarketState::from_parts(
        orders,
        balances,
        settlements,
        next_id(tx, NEXT_ORDER_ID_KEY)?,
        next_id(tx, NEXT_SETTLEMENT_ID_KEY)?,
    ))
}

fn load_order(
    conn: &Connection,
    order_id: OrderId,
    orders: &mut BTreeMap<OrderId, Order>,
) -> rusqlite::Result<()> {
    if let Some(order) = conn
        .query_row(
            "SELECT order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
                    price, quantity_remaining, escrowed_currency, status, reservation_id
             FROM orders WHERE order_id = ?1",
            params![order_id.0],
            load_order_row,
        )
        .optional()?
    {
        orders.insert(order_id, order);
    }
    Ok(())
}

fn load_balance(
    conn: &Connection,
    player_id: PlayerId,
    balances: &mut BTreeMap<PlayerId, u64>,
) -> rusqlite::Result<()> {
    if let Some(balance) = conn
        .query_row(
            "SELECT balance FROM currency WHERE player_id = ?1",
            params![to_i64(player_id.raw())?],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        balances.insert(player_id, from_i64(balance)?);
    }
    Ok(())
}

fn load_settlement(
    conn: &Connection,
    settlement_id: SettlementId,
    settlements: &mut BTreeMap<SettlementId, SettlementRecord>,
) -> rusqlite::Result<Option<SettlementRecord>> {
    let record = conn
        .query_row(
            "SELECT settlement_id, effect, status, compensation_for, player_id, ship_id,
                    item_type, module_id, ship_type_id, quantity, order_id,
                    seller_player_id, seller_ship_id, price, last_error
             FROM settlements WHERE settlement_id = ?1",
            params![settlement_id.0],
            load_settlement_row,
        )
        .optional()?;
    if let Some(record) = record.as_ref() {
        settlements.insert(record.id, record.clone());
    }
    Ok(record)
}

fn load_crossing_orders(
    conn: &Connection,
    item_id: ItemId,
    incoming_side: OrderSide,
    incoming_price: u64,
    incoming_quantity: u64,
    orders: &mut BTreeMap<OrderId, Order>,
) -> rusqlite::Result<()> {
    let (item_type, module_id, ship_type_id) = item_id.storage_columns().into_tuple();
    let limit = MAX_MATCH_CANDIDATES
        .checked_add(1)
        .ok_or_else(|| invalid_row("matching candidate limit overflow"))?;
    let (sql, opposite_side) = match incoming_side {
        OrderSide::Bid => (
            "SELECT order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
                    price, quantity_remaining, escrowed_currency, status, reservation_id
             FROM orders
             WHERE item_type = ?1 AND module_id = ?2 AND ship_type_id = ?3
               AND side = ?4 AND status = ?5 AND price <= ?6
             ORDER BY price ASC, order_id ASC
             LIMIT ?7",
            OrderSide::Ask,
        ),
        OrderSide::Ask => (
            "SELECT order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
                    price, quantity_remaining, escrowed_currency, status, reservation_id
             FROM orders
             WHERE item_type = ?1 AND module_id = ?2 AND ship_type_id = ?3
               AND side = ?4 AND status = ?5 AND price >= ?6
             ORDER BY price DESC, order_id ASC
             LIMIT ?7",
            OrderSide::Bid,
        ),
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![
            item_type,
            module_id,
            ship_type_id,
            side_as_str(opposite_side),
            status_as_str(OrderStatus::Open),
            to_i64(incoming_price)?,
            limit as i64,
        ],
        load_order_row,
    )?;
    let mut loaded = Vec::new();
    let mut available_quantity = 0_u64;
    for (candidate_index, row) in rows.enumerate() {
        if candidate_index == MAX_MATCH_CANDIDATES {
            return Err(limit_exceeded("matching candidate set"));
        }
        let order = row?;
        available_quantity = available_quantity
            .checked_add(order.quantity_remaining)
            .ok_or_else(|| invalid_row("matching candidate quantity overflow"))?;
        loaded.push(order);
        if available_quantity >= incoming_quantity {
            break;
        }
    }
    orders.extend(loaded.into_iter().map(|order| (order.id, order)));
    Ok(())
}

fn load_pending_settlements_after(
    conn: &Connection,
    cursor: i64,
    limit: usize,
) -> rusqlite::Result<Vec<SettlementRecord>> {
    let mut stmt = conn.prepare(
        "SELECT settlement_id, effect, status, compensation_for, player_id, ship_id,
                item_type, module_id, ship_type_id, quantity, order_id,
                seller_player_id, seller_ship_id, price, last_error
         FROM settlements
         WHERE status = ?1 AND settlement_id > ?2
         ORDER BY settlement_id
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            settlement_status_as_str(SettlementStatus::Pending),
            cursor,
            limit as i64,
        ],
        load_settlement_row,
    )?;
    rows.collect()
}

fn load_pending_settlements_at_most(
    conn: &Connection,
    cursor: i64,
    limit: usize,
) -> rusqlite::Result<Vec<SettlementRecord>> {
    let mut stmt = conn.prepare(
        "SELECT settlement_id, effect, status, compensation_for, player_id, ship_id,
                item_type, module_id, ship_type_id, quantity, order_id,
                seller_player_id, seller_ship_id, price, last_error
         FROM settlements
         WHERE status = ?1 AND settlement_id <= ?2
         ORDER BY settlement_id
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            settlement_status_as_str(SettlementStatus::Pending),
            cursor,
            limit as i64,
        ],
        load_settlement_row,
    )?;
    rows.collect()
}

fn persist_state_delta(
    tx: &Transaction<'_>,
    before: &WorkingSet,
    after: &WorkingSet,
) -> rusqlite::Result<()> {
    let order_ids: BTreeSet<_> = before
        .orders
        .keys()
        .chain(after.orders.keys())
        .copied()
        .collect();
    for order_id in order_ids {
        match (before.orders.get(&order_id), after.orders.get(&order_id)) {
            (Some(before), Some(after)) if before == after => {}
            (_, Some(order)) => upsert_order(tx, order)?,
            (Some(_), None) => {
                tx.execute(
                    "DELETE FROM orders WHERE order_id = ?1",
                    params![order_id.0],
                )?;
            }
            (None, None) => unreachable!(),
        }
    }

    let balance_players: BTreeSet<_> = before
        .balances
        .keys()
        .chain(after.balances.keys())
        .copied()
        .collect();
    for player_id in balance_players {
        if before.balances.get(&player_id) == after.balances.get(&player_id) {
            continue;
        }
        let balance = after
            .balances
            .get(&player_id)
            .copied()
            .ok_or_else(|| invalid_row("Currency balance cannot be deleted"))?;
        tx.execute(
            "INSERT INTO currency (player_id, balance) VALUES (?1, ?2)
             ON CONFLICT(player_id) DO UPDATE SET balance = excluded.balance",
            params![to_i64(player_id.raw())?, to_i64(balance)?],
        )?;
    }

    let settlement_ids: BTreeSet<_> = before
        .settlements
        .keys()
        .chain(after.settlements.keys())
        .copied()
        .collect();
    for settlement_id in settlement_ids {
        match (
            before.settlements.get(&settlement_id),
            after.settlements.get(&settlement_id),
        ) {
            (Some(before), Some(after)) if before == after => {}
            (Some(_), Some(record)) => {
                tx.execute(
                    "UPDATE settlements
                     SET status = ?1, compensation_for = ?2, last_error = ?3
                     WHERE settlement_id = ?4",
                    params![
                        settlement_status_as_str(record.status),
                        record.compensation_for.map(|id| id.0),
                        record.last_error.as_deref(),
                        record.id.0,
                    ],
                )?;
            }
            (None, Some(record)) => insert_settlement(tx, record)?,
            (Some(_), None) => {
                tx.execute(
                    "DELETE FROM settlements WHERE settlement_id = ?1",
                    params![settlement_id.0],
                )?;
            }
            (None, None) => unreachable!(),
        }
    }

    if before.next_order_id != after.next_order_id {
        tx.execute(
            "UPDATE market_meta SET value = ?1 WHERE key = ?2",
            params![after.next_order_id, NEXT_ORDER_ID_KEY],
        )?;
    }
    if before.next_settlement_id != after.next_settlement_id {
        tx.execute(
            "UPDATE market_meta SET value = ?1 WHERE key = ?2",
            params![after.next_settlement_id, NEXT_SETTLEMENT_ID_KEY],
        )?;
    }
    Ok(())
}

fn upsert_order(tx: &Transaction<'_>, order: &Order) -> rusqlite::Result<()> {
    let (item_type, module_id, ship_type_id) = order.item_id.storage_columns().into_tuple();
    tx.execute(
        "INSERT INTO orders
         (order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
          price, quantity_remaining, escrowed_currency, status, reservation_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(order_id) DO UPDATE SET
            player_id = excluded.player_id,
            ship_id = excluded.ship_id,
            side = excluded.side,
            item_type = excluded.item_type,
            module_id = excluded.module_id,
            ship_type_id = excluded.ship_type_id,
            price = excluded.price,
            quantity_remaining = excluded.quantity_remaining,
            escrowed_currency = excluded.escrowed_currency,
            status = excluded.status,
            reservation_id = excluded.reservation_id",
        params![
            order.id.0,
            to_i64(order.player_id.raw())?,
            to_i64(order.ship_id.raw())?,
            side_as_str(order.side),
            item_type,
            module_id,
            ship_type_id,
            to_i64(order.price)?,
            to_i64(order.quantity_remaining)?,
            to_i64(order.escrowed_currency)?,
            status_as_str(order.status),
            order.reservation_id.map(|id| id.0),
        ],
    )?;
    Ok(())
}

fn insert_settlement(tx: &Transaction<'_>, record: &SettlementRecord) -> rusqlite::Result<()> {
    let SettlementColumns {
        effect,
        player_id,
        ship_id,
        item_id,
        quantity,
        order_id,
        seller_player_id,
        seller_ship_id,
        price,
    } = settlement_columns(record.effect);
    let (item_type, module_id, ship_type_id) = item_id.storage_columns().into_tuple();
    tx.execute(
        "INSERT INTO settlements
             (settlement_id, effect, status, compensation_for, player_id, ship_id,
              item_type, module_id, ship_type_id, quantity, order_id,
              seller_player_id, seller_ship_id, price, last_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(settlement_id) DO UPDATE SET
                status = excluded.status,
                compensation_for = excluded.compensation_for,
                last_error = excluded.last_error",
        params![
            record.id.0,
            effect,
            settlement_status_as_str(record.status),
            record.compensation_for.map(|id| id.0),
            to_i64(player_id.raw())?,
            to_i64(ship_id.raw())?,
            item_type,
            module_id,
            ship_type_id,
            to_i64(quantity)?,
            order_id.map(|id| id.0),
            seller_player_id.map(|id| to_i64(id.raw())).transpose()?,
            seller_ship_id.map(|id| to_i64(id.raw())).transpose()?,
            price.map(to_i64).transpose()?,
            record.last_error.as_deref(),
        ],
    )?;
    Ok(())
}

fn load_order_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Order> {
    Ok(Order {
        id: OrderId(row.get(0)?),
        player_id: PlayerId(from_i64(row.get(1)?)?),
        ship_id: ShipId(EntityId::from_raw(from_i64(row.get(2)?)?)),
        side: side_from_str(&row.get::<_, String>(3)?)?,
        item_id: item_from_row(row, 4)?,
        price: from_i64(row.get(7)?)?,
        quantity_remaining: from_i64(row.get(8)?)?,
        escrowed_currency: from_i64(row.get(9)?)?,
        status: status_from_str(&row.get::<_, String>(10)?)?,
        reservation_id: row.get::<_, Option<i64>>(11)?.map(SettlementId),
    })
}

fn next_id(conn: &Connection, key: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT value FROM market_meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
}

fn item_from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<ItemId> {
    let item_type: String = row.get(offset)?;
    let module_id: u32 = row.get(offset + 1)?;
    let ship_type_id: u32 = row.get(offset + 2)?;
    ItemId::from_storage_columns(&item_type, module_id, ship_type_id).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            offset,
            rusqlite::types::Type::Text,
            error.to_string().into(),
        )
    })
}

fn load_settlement_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SettlementRecord> {
    let id = SettlementId(row.get(0)?);
    let effect_name: String = row.get(1)?;
    let item_id = item_from_row(row, 6)?;
    let player_id = PlayerId(from_i64(row.get(4)?)?);
    let ship_id = ShipId(EntityId::from_raw(from_i64(row.get(5)?)?));
    let quantity = from_i64(row.get(9)?)?;
    let effect = match effect_name.as_str() {
        "ReserveAsk" => SettlementEffect::ReserveAsk {
            order_id: OrderId(row.get(10)?),
            player_id,
            ship_id,
            item_id,
            quantity,
        },
        "ReturnItem" => SettlementEffect::ReturnItem {
            player_id,
            ship_id,
            item_id,
            quantity,
        },
        "CreditItem" => SettlementEffect::CreditItem {
            buyer: player_id,
            buyer_ship_id: ship_id,
            seller: PlayerId(from_i64(row.get(11)?)?),
            seller_ship_id: ShipId(EntityId::from_raw(from_i64(row.get(12)?)?)),
            item_id,
            quantity,
            price: from_i64(row.get(13)?)?,
        },
        _ => return Err(invalid_row("unknown settlement effect")),
    };
    Ok(SettlementRecord {
        id,
        effect,
        status: settlement_status_from_str(&row.get::<_, String>(2)?)?,
        compensation_for: row.get::<_, Option<i64>>(3)?.map(SettlementId),
        last_error: row.get(14)?,
    })
}

struct SettlementColumns {
    effect: &'static str,
    player_id: PlayerId,
    ship_id: ShipId,
    item_id: ItemId,
    quantity: u64,
    order_id: Option<OrderId>,
    seller_player_id: Option<PlayerId>,
    seller_ship_id: Option<ShipId>,
    price: Option<u64>,
}

fn settlement_columns(effect: SettlementEffect) -> SettlementColumns {
    match effect {
        SettlementEffect::ReserveAsk {
            order_id,
            player_id,
            ship_id,
            item_id,
            quantity,
        } => SettlementColumns {
            effect: "ReserveAsk",
            player_id,
            ship_id,
            item_id,
            quantity,
            order_id: Some(order_id),
            seller_player_id: None,
            seller_ship_id: None,
            price: None,
        },
        SettlementEffect::ReturnItem {
            player_id,
            ship_id,
            item_id,
            quantity,
        } => SettlementColumns {
            effect: "ReturnItem",
            player_id,
            ship_id,
            item_id,
            quantity,
            order_id: None,
            seller_player_id: None,
            seller_ship_id: None,
            price: None,
        },
        SettlementEffect::CreditItem {
            buyer,
            buyer_ship_id,
            seller,
            seller_ship_id,
            item_id,
            quantity,
            price,
        } => SettlementColumns {
            effect: "CreditItem",
            player_id: buyer,
            ship_id: buyer_ship_id,
            item_id,
            quantity,
            order_id: None,
            seller_player_id: Some(seller),
            seller_ship_id: Some(seller_ship_id),
            price: Some(price),
        },
    }
}

fn side_as_str(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Bid => "Bid",
        OrderSide::Ask => "Ask",
    }
}

fn side_from_str(side: &str) -> rusqlite::Result<OrderSide> {
    match side {
        "Bid" => Ok(OrderSide::Bid),
        "Ask" => Ok(OrderSide::Ask),
        _ => Err(invalid_row("unknown order side")),
    }
}

fn status_as_str(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::PendingReservation => "PendingReservation",
        OrderStatus::Open => "Open",
    }
}

fn status_from_str(status: &str) -> rusqlite::Result<OrderStatus> {
    match status {
        "PendingReservation" => Ok(OrderStatus::PendingReservation),
        "Open" => Ok(OrderStatus::Open),
        _ => Err(invalid_row("unknown order status")),
    }
}

fn settlement_status_as_str(status: SettlementStatus) -> &'static str {
    match status {
        SettlementStatus::Pending => "Pending",
        SettlementStatus::Applied => "Applied",
        SettlementStatus::Compensating => "Compensating",
        SettlementStatus::Terminal => "Terminal",
        SettlementStatus::Compensated => "Compensated",
    }
}

fn settlement_status_from_str(status: &str) -> rusqlite::Result<SettlementStatus> {
    match status {
        "Pending" => Ok(SettlementStatus::Pending),
        "Applied" => Ok(SettlementStatus::Applied),
        "Compensating" => Ok(SettlementStatus::Compensating),
        "Terminal" => Ok(SettlementStatus::Terminal),
        "Compensated" => Ok(SettlementStatus::Compensated),
        _ => Err(invalid_row("unknown settlement status")),
    }
}

fn to_i64(value: u64) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|_| invalid_row("value exceeds SQLite integer range"))
}

fn from_i64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| invalid_row("negative SQLite integer"))
}

fn invalid_row(message: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Integer,
        message.to_owned().into(),
    )
}

fn limit_exceeded(scope: &str) -> rusqlite::Error {
    invalid_row(&format!("{scope} exceeds the configured resource limit"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MarketCommandResult, MarketEvent};
    use dawn_core::{ItemId, NodeId};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn ship(counter: u64) -> ShipId {
        ShipId::new(NodeId(0), counter)
    }

    fn temp_db_path() -> String {
        let id = TEMP_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!("dawn-market-{}-{id}.sqlite", std::process::id()))
            .to_string_lossy()
            .into_owned()
    }

    fn install_write_audit(db: &MarketDb) {
        db.conn
            .execute_batch(
                "CREATE TABLE write_audit (
                    sequence_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    table_name TEXT NOT NULL,
                    operation TEXT NOT NULL,
                    row_id INTEGER NOT NULL
                );
                CREATE TRIGGER audit_orders_insert AFTER INSERT ON orders BEGIN
                    INSERT INTO write_audit (table_name, operation, row_id)
                    VALUES ('orders', 'insert', NEW.order_id);
                END;
                CREATE TRIGGER audit_orders_update AFTER UPDATE ON orders BEGIN
                    INSERT INTO write_audit (table_name, operation, row_id)
                    VALUES ('orders', 'update', NEW.order_id);
                END;
                CREATE TRIGGER audit_orders_delete AFTER DELETE ON orders BEGIN
                    INSERT INTO write_audit (table_name, operation, row_id)
                    VALUES ('orders', 'delete', OLD.order_id);
                END;
                CREATE TRIGGER audit_currency_insert AFTER INSERT ON currency BEGIN
                    INSERT INTO write_audit (table_name, operation, row_id)
                    VALUES ('currency', 'insert', NEW.player_id);
                END;
                CREATE TRIGGER audit_currency_update AFTER UPDATE ON currency BEGIN
                    INSERT INTO write_audit (table_name, operation, row_id)
                    VALUES ('currency', 'update', NEW.player_id);
                END;
                CREATE TRIGGER audit_currency_delete AFTER DELETE ON currency BEGIN
                    INSERT INTO write_audit (table_name, operation, row_id)
                    VALUES ('currency', 'delete', OLD.player_id);
                END;",
            )
            .unwrap();
    }

    fn audited_writes(db: &MarketDb) -> Vec<(String, String, i64)> {
        let mut stmt = db
            .conn
            .prepare(
                "SELECT table_name, operation, row_id
                 FROM write_audit ORDER BY sequence_id",
            )
            .unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn open_ask(db: &mut MarketDb, player_id: u64, item_id: ItemId, price: u64) -> OrderId {
        let queued = db
            .execute(MarketCommand::PlaceOrder {
                player_id: PlayerId(player_id),
                ship_id: ship(player_id),
                item_id,
                side: OrderSide::Ask,
                price,
                quantity: 2,
            })
            .unwrap();
        db.execute(MarketCommand::AcknowledgeSettlement {
            settlement_id: queued.settlements[0].id,
        })
        .unwrap();
        db.open_orders_for(PlayerId(player_id))
            .unwrap()
            .into_iter()
            .find(|order| order.player_id == PlayerId(player_id) && order.item_id == item_id)
            .unwrap()
            .order_id
    }

    fn seed_open_asks(db: &mut MarketDb, count: usize) {
        let tx = db.conn.transaction().unwrap();
        let (item_type, module_id, ship_type_id) =
            ItemId::ScrapMetal.storage_columns().into_tuple();
        for order_id in 1..=count {
            tx.execute(
                "INSERT INTO orders
                 (order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
                  price, quantity_remaining, escrowed_currency, status, reservation_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
                params![
                    order_id as i64,
                    order_id as i64,
                    order_id as i64,
                    side_as_str(OrderSide::Ask),
                    item_type,
                    module_id,
                    ship_type_id,
                    100_i64,
                    1_i64,
                    0_i64,
                    status_as_str(OrderStatus::Open),
                ],
            )
            .unwrap();
        }
        tx.execute(
            "UPDATE market_meta SET value = ?1 WHERE key = ?2",
            params![(count as i64) + 1, NEXT_ORDER_ID_KEY],
        )
        .unwrap();
        tx.commit().unwrap();
    }

    fn remember_settlement_ids(
        transition: &MarketTransition,
        settlement_ids: &mut BTreeSet<SettlementId>,
    ) {
        settlement_ids.extend(transition.settlements.iter().map(|intent| intent.id));
        for event in &transition.events {
            match event {
                MarketEvent::SettlementQueued { settlement_id }
                | MarketEvent::SettlementApplied { settlement_id }
                | MarketEvent::SettlementTerminal { settlement_id }
                | MarketEvent::TradeExecuted { settlement_id, .. } => {
                    settlement_ids.insert(*settlement_id);
                }
                MarketEvent::SettlementCompensationQueued {
                    settlement_id,
                    compensation_id,
                } => {
                    settlement_ids.insert(*settlement_id);
                    settlement_ids.insert(*compensation_id);
                }
                MarketEvent::OrderQueued { .. }
                | MarketEvent::OrderOpened { .. }
                | MarketEvent::OrderCancelled { .. } => {}
            }
        }
        match &transition.result {
            MarketCommandResult::OrderPending { reservation_id, .. }
            | MarketCommandResult::SettlementApplied {
                settlement_id: reservation_id,
            }
            | MarketCommandResult::SettlementTerminal {
                settlement_id: reservation_id,
            }
            | MarketCommandResult::DuplicateSettlementAcknowledgement {
                settlement_id: reservation_id,
            } => {
                settlement_ids.insert(*reservation_id);
            }
            MarketCommandResult::OrderCancelled { return_id } => {
                if let Some(settlement_id) = return_id {
                    settlement_ids.insert(*settlement_id);
                }
            }
            MarketCommandResult::SettlementCompensating {
                settlement_id,
                compensation_id,
            } => {
                settlement_ids.insert(*settlement_id);
                settlement_ids.insert(*compensation_id);
            }
            MarketCommandResult::OrderPlaced { .. } | MarketCommandResult::CurrencyCredited => {}
        }
    }

    fn assert_observable_parity(
        pure: &MarketState,
        db: &MarketDb,
        players: &[PlayerId],
        settlement_ids: &BTreeSet<SettlementId>,
    ) {
        for player_id in players {
            assert_eq!(
                pure.currency_balance(*player_id),
                db.currency_balance(*player_id).unwrap(),
                "balance diverged for {player_id:?}",
            );
        }

        assert_eq!(
            pure.open_orders(),
            db.open_orders_for(PlayerId(999)).unwrap()
        );

        let mut pure_pending = pure.pending_settlements();
        let mut db_pending = db.pending_settlements().unwrap();
        pure_pending.sort_by_key(|intent| intent.id);
        db_pending.sort_by_key(|intent| intent.id);
        assert_eq!(pure_pending, db_pending);

        for settlement_id in settlement_ids {
            assert_eq!(
                pure.settlement(*settlement_id).cloned(),
                db.settlement(*settlement_id).unwrap(),
                "settlement {settlement_id:?} diverged",
            );
        }
    }

    fn assert_transition_parity(
        pure: &mut MarketState,
        db: &mut MarketDb,
        command: MarketCommand,
        players: &[PlayerId],
        settlement_ids: &mut BTreeSet<SettlementId>,
    ) -> MarketTransition {
        let pure_transition = pure.apply(command.clone()).unwrap();
        let db_transition = db.execute(command).unwrap();
        assert_eq!(pure_transition, db_transition);
        remember_settlement_ids(&pure_transition, settlement_ids);
        assert_observable_parity(pure, db, players, settlement_ids);
        pure_transition
    }

    #[test]
    fn pending_settlement_survives_database_reopen() {
        let path = temp_db_path();
        {
            let mut db = MarketDb::open(&path).unwrap();
            let transition = db
                .execute(MarketCommand::PlaceOrder {
                    player_id: PlayerId(1),
                    ship_id: ship(1),
                    item_id: ItemId::ScrapMetal,
                    side: OrderSide::Ask,
                    price: 100,
                    quantity: 2,
                })
                .unwrap();
            assert_eq!(transition.settlements[0].id, SettlementId(1));
        }

        let reopened = MarketDb::open(&path).unwrap();
        assert_eq!(reopened.pending_settlements().unwrap().len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn order_and_settlement_ids_are_not_reused_after_restart_and_cancel() {
        let path = temp_db_path();
        {
            let mut db = MarketDb::open(&path).unwrap();
            let placed = db
                .execute(MarketCommand::PlaceOrder {
                    player_id: PlayerId(1),
                    ship_id: ship(1),
                    item_id: ItemId::ScrapMetal,
                    side: OrderSide::Ask,
                    price: 100,
                    quantity: 2,
                })
                .unwrap();
            db.execute(MarketCommand::AcknowledgeSettlement {
                settlement_id: placed.settlements[0].id,
            })
            .unwrap();
            let order_id = db.open_orders_for(PlayerId(1)).unwrap()[0].order_id;
            db.execute(MarketCommand::CancelOrder {
                player_id: PlayerId(1),
                order_id,
            })
            .unwrap();
        }

        let mut reopened = MarketDb::open(&path).unwrap();
        let placed = reopened
            .execute(MarketCommand::PlaceOrder {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Ask,
                price: 100,
                quantity: 1,
            })
            .unwrap();
        assert_eq!(
            placed.result,
            MarketCommandResult::OrderPending {
                order_id: OrderId(2),
                reservation_id: SettlementId(3),
            }
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn open_orders_returns_the_bounded_global_book_for_every_caller() {
        let mut db = MarketDb::open_in_memory().unwrap();
        open_ask(&mut db, 1, ItemId::ScrapMetal, 100);
        open_ask(&mut db, 2, ItemId::ScrapMetal, 110);

        let book = db.open_orders_for(PlayerId(999)).unwrap();
        assert_eq!(book.len(), 2);
        assert_eq!(book[0].player_id, PlayerId(1));
        assert_eq!(book[1].player_id, PlayerId(2));
    }

    #[test]
    fn cancelling_a_bid_restores_escrow_without_losing_the_remaining_balance() {
        let mut db = MarketDb::open_in_memory().unwrap();
        db.credit_currency(PlayerId(1), 300).unwrap();
        db.credit_currency(PlayerId(2), 50).unwrap();

        let placed = db
            .execute(MarketCommand::PlaceOrder {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 100,
                quantity: 2,
            })
            .unwrap();
        let order_id = match placed.result {
            MarketCommandResult::OrderPlaced {
                order_id: Some(order_id),
                fills: 0,
            } => order_id,
            result => panic!("expected an unfilled bid, got {result:?}"),
        };
        assert_eq!(db.currency_balance(PlayerId(1)).unwrap(), 100);

        let rejected = db.execute(MarketCommand::CancelOrder {
            player_id: PlayerId(2),
            order_id,
        });
        assert!(matches!(
            rejected,
            Err(MarketError::Rejected(MarketRejection::OrderNotOwned {
                order_id: rejected_order_id,
                player_id: PlayerId(2),
            })) if rejected_order_id == order_id
        ));
        assert_eq!(db.currency_balance(PlayerId(1)).unwrap(), 100);
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), 50);

        db.execute(MarketCommand::CancelOrder {
            player_id: PlayerId(1),
            order_id,
        })
        .unwrap();
        assert_eq!(db.currency_balance(PlayerId(1)).unwrap(), 300);
    }

    #[test]
    fn repository_matches_the_pure_policy_across_every_command_family() {
        let mut pure = MarketState::default();
        let mut db = MarketDb::open_in_memory().unwrap();
        let players = [PlayerId(1), PlayerId(2)];
        let mut settlement_ids = BTreeSet::new();

        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::CreditCurrency {
                player_id: PlayerId(2),
                amount: 500,
            },
            &players,
            &mut settlement_ids,
        );

        let ask = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::PlaceOrder {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Ask,
                price: 100,
                quantity: 2,
            },
            &players,
            &mut settlement_ids,
        );
        let reservation_id = match ask.result {
            MarketCommandResult::OrderPending { reservation_id, .. } => reservation_id,
            result => panic!("expected a reservation, got {result:?}"),
        };
        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: reservation_id,
            },
            &players,
            &mut settlement_ids,
        );

        let bid = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::PlaceOrder {
                player_id: PlayerId(2),
                ship_id: ship(2),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 120,
                quantity: 3,
            },
            &players,
            &mut settlement_ids,
        );
        let bid_order_id = match bid.result {
            MarketCommandResult::OrderPlaced {
                order_id: Some(order_id),
                fills: 1,
            } => order_id,
            result => panic!("expected one partial fill, got {result:?}"),
        };
        let credit_item_id = bid
            .settlements
            .first()
            .map(|intent| intent.id)
            .expect("matching must queue a CreditItem settlement");
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), 180);

        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: credit_item_id,
            },
            &players,
            &mut settlement_ids,
        );
        assert_eq!(db.currency_balance(PlayerId(1)).unwrap(), 200);

        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::CancelOrder {
                player_id: PlayerId(2),
                order_id: bid_order_id,
            },
            &players,
            &mut settlement_ids,
        );
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), 300);

        let return_ask = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::PlaceOrder {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Ask,
                price: 100,
                quantity: 1,
            },
            &players,
            &mut settlement_ids,
        );
        let return_reservation_id = match return_ask.result {
            MarketCommandResult::OrderPending { reservation_id, .. } => reservation_id,
            result => panic!("expected a return-ask reservation, got {result:?}"),
        };
        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: return_reservation_id,
            },
            &players,
            &mut settlement_ids,
        );
        let return_order_id = db
            .open_orders_for(PlayerId(999))
            .unwrap()
            .into_iter()
            .find(|order| order.player_id == PlayerId(1))
            .expect("the second Ask should be open")
            .order_id;
        let cancelled = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::CancelOrder {
                player_id: PlayerId(1),
                order_id: return_order_id,
            },
            &players,
            &mut settlement_ids,
        );
        let return_item_id = match cancelled.result {
            MarketCommandResult::OrderCancelled {
                return_id: Some(return_id),
            } => return_id,
            result => panic!("expected a ReturnItem settlement, got {result:?}"),
        };
        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: return_item_id,
            },
            &players,
            &mut settlement_ids,
        );

        let rejected_ask = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::PlaceOrder {
                player_id: PlayerId(1),
                ship_id: ship(1),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Ask,
                price: 100,
                quantity: 1,
            },
            &players,
            &mut settlement_ids,
        );
        let rejected_reservation_id = match rejected_ask.result {
            MarketCommandResult::OrderPending { reservation_id, .. } => reservation_id,
            result => panic!("expected a rejected-ask reservation, got {result:?}"),
        };
        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: rejected_reservation_id,
            },
            &players,
            &mut settlement_ids,
        );

        let rejected_trade = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::PlaceOrder {
                player_id: PlayerId(2),
                ship_id: ship(2),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 100,
                quantity: 1,
            },
            &players,
            &mut settlement_ids,
        );
        let rejected_credit_id = rejected_trade
            .settlements
            .first()
            .map(|intent| intent.id)
            .expect("second matching trade must queue a settlement");
        let compensation = assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::RejectSettlement {
                settlement_id: rejected_credit_id,
                reason: "destination cargo unavailable".to_owned(),
            },
            &players,
            &mut settlement_ids,
        );
        let compensation_id = match compensation.result {
            MarketCommandResult::SettlementCompensating {
                compensation_id, ..
            } => compensation_id,
            result => panic!("expected compensation, got {result:?}"),
        };
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), 300);

        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: rejected_credit_id,
            },
            &players,
            &mut settlement_ids,
        );
        assert_transition_parity(
            &mut pure,
            &mut db,
            MarketCommand::AcknowledgeSettlement {
                settlement_id: compensation_id,
            },
            &players,
            &mut settlement_ids,
        );
    }

    #[test]
    fn large_book_matching_stops_when_the_first_candidate_fulfills_the_bid() {
        let mut db = MarketDb::open_in_memory().unwrap();
        seed_open_asks(&mut db, MAX_MATCH_CANDIDATES + 1);
        db.credit_currency(PlayerId(2), 100).unwrap();

        let transition = db
            .execute(MarketCommand::PlaceOrder {
                player_id: PlayerId(2),
                ship_id: ship(2),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Bid,
                price: 100,
                quantity: 1,
            })
            .unwrap();

        assert_eq!(
            transition.result,
            MarketCommandResult::OrderPlaced {
                order_id: None,
                fills: 1,
            }
        );
        assert_eq!(transition.settlements.len(), 1);
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), 0);
        assert!(!db
            .open_orders_for(PlayerId(999))
            .unwrap()
            .iter()
            .any(|order| order.order_id == OrderId(1)));
    }

    #[test]
    fn matching_rejects_a_book_that_exceeds_the_candidate_limit_before_fulfillment() {
        let mut db = MarketDb::open_in_memory().unwrap();
        seed_open_asks(&mut db, MAX_MATCH_CANDIDATES + 1);
        let required_currency = 100 * (MAX_MATCH_CANDIDATES as u64 + 1);
        db.credit_currency(PlayerId(2), required_currency).unwrap();

        let result = db.execute(MarketCommand::PlaceOrder {
            player_id: PlayerId(2),
            ship_id: ship(2),
            item_id: ItemId::ScrapMetal,
            side: OrderSide::Bid,
            price: 100,
            quantity: MAX_MATCH_CANDIDATES as u64 + 1,
        });

        assert!(matches!(result, Err(MarketError::Storage(_))));
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), required_currency);
        assert_eq!(
            db.open_orders_for(PlayerId(999)).unwrap()[0].order_id,
            OrderId(1)
        );
    }

    #[test]
    fn a_command_does_not_rewrite_unrelated_orders_or_balances() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let matched_order = open_ask(&mut db, 1, ItemId::ScrapMetal, 100);
        let unrelated_item_order = open_ask(
            &mut db,
            3,
            ItemId::PackagedShip(dawn_core::ShipTypeId(7)),
            100,
        );
        let unrelated_price_order = open_ask(&mut db, 4, ItemId::ScrapMetal, 200);
        db.credit_currency(PlayerId(2), 100).unwrap();
        db.credit_currency(PlayerId(99), 500).unwrap();

        install_write_audit(&db);
        db.conn.execute("DELETE FROM write_audit", []).unwrap();

        db.execute(MarketCommand::PlaceOrder {
            player_id: PlayerId(2),
            ship_id: ship(2),
            item_id: ItemId::ScrapMetal,
            side: OrderSide::Bid,
            price: 100,
            quantity: 1,
        })
        .unwrap();

        let writes = audited_writes(&db);
        assert!(writes.iter().any(|(table, operation, row_id)| {
            table == "orders" && operation == "update" && *row_id == matched_order.0
        }));
        assert!(!writes.iter().any(|(table, _, row_id)| table == "orders"
            && (*row_id == unrelated_item_order.0 || *row_id == unrelated_price_order.0)));
        assert!(!writes
            .iter()
            .any(|(table, _, row_id)| table == "currency" && *row_id == 99));
        assert_eq!(db.currency_balance(PlayerId(99)).unwrap(), 500);
        assert_eq!(
            db.open_orders_for(PlayerId(2))
                .unwrap()
                .into_iter()
                .map(|order| order.order_id)
                .collect::<Vec<_>>(),
            vec![matched_order, unrelated_item_order, unrelated_price_order]
        );
    }

    #[test]
    fn pending_settlement_pages_make_progress_past_the_first_bound() {
        let mut db = MarketDb::open_in_memory().unwrap();
        for player_id in 1..=1_001 {
            db.execute(MarketCommand::PlaceOrder {
                player_id: PlayerId(player_id),
                ship_id: ship(player_id),
                item_id: ItemId::ScrapMetal,
                side: OrderSide::Ask,
                price: 100,
                quantity: 1,
            })
            .unwrap();
        }

        let first_page = db.pending_settlements().unwrap();
        let second_page = db.pending_settlements().unwrap();
        assert_eq!(first_page.len(), MAX_SETTLEMENT_VIEW);
        assert_eq!(second_page.len(), MAX_SETTLEMENT_VIEW);
        assert_eq!(second_page[0].id, SettlementId(1_001));
        assert!(second_page
            .iter()
            .any(|intent| intent.id == SettlementId(1_001)));
    }

    #[test]
    fn failed_delta_persistence_rolls_back_every_market_write() {
        let mut db = MarketDb::open_in_memory().unwrap();
        let order_id = open_ask(&mut db, 1, ItemId::ScrapMetal, 100);
        db.credit_currency(PlayerId(2), 100).unwrap();
        db.conn
            .execute_batch(
                "CREATE TRIGGER fail_currency_update BEFORE UPDATE ON currency BEGIN
                    SELECT RAISE(ABORT, 'injected currency write failure');
                END;",
            )
            .unwrap();

        let result = db.execute(MarketCommand::PlaceOrder {
            player_id: PlayerId(2),
            ship_id: ship(2),
            item_id: ItemId::ScrapMetal,
            side: OrderSide::Bid,
            price: 100,
            quantity: 1,
        });

        assert!(matches!(result, Err(MarketError::Storage(_))));
        let order = db
            .open_orders_for(PlayerId(999))
            .unwrap()
            .into_iter()
            .find(|order| order.order_id == order_id)
            .unwrap();
        assert_eq!(order.quantity_remaining, 2);
        assert_eq!(db.currency_balance(PlayerId(2)).unwrap(), 100);
        assert!(db.pending_settlements().unwrap().is_empty());
    }
}
