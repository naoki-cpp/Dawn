//! SQLite adapter for the pure Market transition policy.
//!
//! All SQL lives in this module. A command loads a complete `MarketState`,
//! applies one pure transition, and persists the resulting orders, balances,
//! and settlement outbox in one SQLite transaction.

use std::collections::BTreeMap;

use dawn_core::{EntityId, ItemId, PlayerId, ShipId};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::order_book::{
    MarketCommand, MarketOrderView, MarketRejection, MarketState, MarketTransition, OrderId,
    OrderSide, OrderStatus, SettlementEffect, SettlementId, SettlementIntent, SettlementRecord,
    SettlementStatus,
};

const SCHEMA_VERSION: i64 = 1;
const MAX_ORDER_VIEW: usize = 200;
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

    /// Apply one pure Market command and atomically persist its resulting
    /// state and any newly-created outbox entries.
    pub fn execute(&mut self, command: MarketCommand) -> Result<MarketTransition, MarketError> {
        let tx = self.conn.transaction()?;
        let state = load_state(&tx)?;
        let mut state = state;
        let transition = state.apply(command)?;
        persist_state(&tx, &state)?;
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

    /// Return a bounded stable snapshot of open orders.
    pub fn open_orders_for(&self, _player_id: PlayerId) -> rusqlite::Result<Vec<MarketOrderView>> {
        let state = load_state(&self.conn)?;
        Ok(state
            .open_orders()
            .into_iter()
            .take(MAX_ORDER_VIEW)
            .collect())
    }

    /// Return every pending settlement in ID order for runtime delivery.
    pub fn pending_settlements(&self) -> rusqlite::Result<Vec<SettlementIntent>> {
        Ok(load_state(&self.conn)?.pending_settlements())
    }

    /// Inspect a settlement after a delivery attempt.
    pub fn settlement(
        &self,
        settlement_id: SettlementId,
    ) -> rusqlite::Result<Option<SettlementRecord>> {
        Ok(load_state(&self.conn)?.settlement(settlement_id).cloned())
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
            CREATE INDEX IF NOT EXISTS orders_matching_idx
                ON orders (item_type, module_id, ship_type_id, side, price, order_id);
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
        Ok(Self { conn })
    }
}

fn load_state(conn: &Connection) -> rusqlite::Result<MarketState> {
    let mut orders = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
                price, quantity_remaining, escrowed_currency, status, reservation_id
         FROM orders ORDER BY order_id",
    )?;
    let rows = stmt.query_map([], |row| {
        let item = item_from_row(row, 4)?;
        let id = OrderId(row.get(0)?);
        Ok((
            id,
            crate::order_book::Order {
                id,
                player_id: PlayerId(from_i64(row.get(1)?)?),
                ship_id: ShipId(EntityId::from_raw(from_i64(row.get(2)?)?)),
                item_id: item,
                side: side_from_str(&row.get::<_, String>(3)?)?,
                price: from_i64(row.get(7)?)?,
                quantity_remaining: from_i64(row.get(8)?)?,
                escrowed_currency: from_i64(row.get(9)?)?,
                status: status_from_str(&row.get::<_, String>(10)?)?,
                reservation_id: row.get::<_, Option<i64>>(11)?.map(SettlementId),
            },
        ))
    })?;
    for row in rows {
        let (id, order) = row?;
        orders.insert(id, order);
    }

    let mut balances = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT player_id, balance FROM currency ORDER BY player_id")?;
    let rows = stmt.query_map([], |row| {
        Ok((PlayerId(from_i64(row.get(0)?)?), from_i64(row.get(1)?)?))
    })?;
    for row in rows {
        let (player, balance) = row?;
        balances.insert(player, balance);
    }

    let mut settlements = BTreeMap::new();
    let mut stmt = conn.prepare(
        "SELECT settlement_id, effect, status, compensation_for, player_id, ship_id,
                item_type, module_id, ship_type_id, quantity, order_id,
                seller_player_id, seller_ship_id, price, last_error
         FROM settlements ORDER BY settlement_id",
    )?;
    let rows = stmt.query_map([], load_settlement_row)?;
    for row in rows {
        let record = row?;
        settlements.insert(record.id, record);
    }

    let next_order_id = next_id(conn, NEXT_ORDER_ID_KEY)?;
    let next_settlement_id = next_id(conn, NEXT_SETTLEMENT_ID_KEY)?;
    Ok(MarketState::from_parts(
        orders,
        balances,
        settlements,
        next_order_id,
        next_settlement_id,
    ))
}

fn persist_state(tx: &Transaction<'_>, state: &MarketState) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM orders", [])?;
    for order in state.orders() {
        let (item_type, module_id, ship_type_id) = order.item_id.storage_columns().into_tuple();
        tx.execute(
            "INSERT INTO orders
             (order_id, player_id, ship_id, side, item_type, module_id, ship_type_id,
              price, quantity_remaining, escrowed_currency, status, reservation_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
    }

    tx.execute("DELETE FROM currency", [])?;
    for (player, balance) in state.balances() {
        tx.execute(
            "INSERT INTO currency (player_id, balance) VALUES (?1, ?2)",
            params![to_i64(player.raw())?, to_i64(*balance)?],
        )?;
    }

    for record in state.settlements() {
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
    }
    let (next_order_id, next_settlement_id) = state.next_ids();
    tx.execute(
        "INSERT INTO market_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![NEXT_ORDER_ID_KEY, next_order_id],
    )?;
    tx.execute(
        "INSERT INTO market_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![NEXT_SETTLEMENT_ID_KEY, next_settlement_id],
    )?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MarketCommandResult;
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
}
