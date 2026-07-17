use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A client request handled by the Market authority, outside the Sector
/// command stream (ADR-0034).
#[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum MarketCommandWire {
    /// Request the caller's Currency balance and currently open orders.
    RefreshMarketCommand {},
    /// Place a limit Bid or Ask for one item stack.
    PlaceMarketOrderCommand {
        ship_id: u64,
        item_type: String,
        module_id: u32,
        ship_type_id: u32,
        side: String,
        price: u64,
        quantity: u64,
    },
    /// Cancel one of the caller's own open orders.
    CancelMarketOrderCommand { order_id: u64 },
}

/// One open order shown by the Market UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarketOrderWire {
    pub order_id: u64,
    pub item_type: String,
    pub module_id: u32,
    pub ship_type_id: u32,
    pub side: String,
    pub price: u64,
    pub quantity: u64,
    pub is_own: bool,
}

/// The server-owned Market state rendered by the client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarketSnapshotWire {
    pub balance: u64,
    pub orders: Vec<MarketOrderWire>,
    pub notice: String,
}

/// Render the Market request wire schema as JSON Schema.
pub fn market_command_wire_json_schema() -> schemars::schema::RootSchema {
    schemars::schema_for!(MarketCommandWire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_order_round_trips_through_json() {
        let command = MarketCommandWire::PlaceMarketOrderCommand {
            ship_id: 42,
            item_type: "ScrapMetal".to_owned(),
            module_id: 0,
            ship_type_id: 0,
            side: "Ask".to_owned(),
            price: 100,
            quantity: 3,
        };

        let json = serde_json::to_string(&command).expect("serialize");
        let decoded: MarketCommandWire = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, command);
    }

    #[test]
    fn snapshot_preserves_order_ownership_for_the_client() {
        let snapshot = MarketSnapshotWire {
            balance: 500,
            orders: vec![MarketOrderWire {
                order_id: 7,
                item_type: "Module".to_owned(),
                module_id: 12,
                ship_type_id: 0,
                side: "Bid".to_owned(),
                price: 25,
                quantity: 2,
                is_own: true,
            }],
            notice: "Order placed".to_owned(),
        };

        let bytes = postcard::to_stdvec(&snapshot).expect("encode");
        let decoded: MarketSnapshotWire = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, snapshot);
    }
}
