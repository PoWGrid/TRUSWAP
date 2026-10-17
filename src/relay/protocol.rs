use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayOrderSide {
    #[serde(rename = "SELL_TRU")]
    SellTru,
    #[serde(rename = "BUY_TRU")]
    BuyTru,
}

impl RelayOrderSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SellTru => "SELL_TRU",
            Self::BuyTru => "BUY_TRU",
        }
    }
}

/// Incoming client message payload over WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "payload")]
pub enum ClientMessage {
    #[serde(rename = "SubmitOrder")]
    SubmitOrder(SignedOrderPayload),

    #[serde(rename = "CancelOrder")]
    CancelOrder(SignedCancelPayload),

    #[serde(rename = "TakeOrder")]
    TakeOrder(TakeOrderPayload),

    #[serde(rename = "Ping")]
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedOrderPayload {
    pub order_id: String,
    pub side: RelayOrderSide,
    pub amount_tru: f64,
    pub amount_usdc: f64,
    pub hash160_hex: String,
    pub timelock: u32,
    pub maker_tru_address: String,
    pub maker_tru_pubkey: Option<String>,
    pub maker_base_address: String,
    #[serde(default)]
    pub signature_base: String,
    #[serde(default)]
    pub signature_tru: Option<String>,
    #[serde(default)]
    pub created_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedCancelPayload {
    pub order_id: String,
    #[serde(default)]
    pub maker_base_address: String,
    #[serde(default)]
    pub signature_base: String,
    #[serde(default)]
    pub maker_tru_address: Option<String>,
    #[serde(default)]
    pub signature_tru: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeOrderPayload {
    pub order_id: String,
    pub taker_base_address: String,
    pub taker_tru_address: String,
    #[serde(default)]
    pub taker_tru_pubkey: Option<String>,
    #[serde(default)]
    pub signature_base: Option<String>,
    #[serde(default)]
    pub signature_tru: Option<String>,
}

/// Outgoing broadcast event pushed from Relay to connected clients
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum RelayBroadcastEvent {
    #[serde(rename = "OrderbookSnapshot")]
    OrderbookSnapshot {
        orders: Vec<RelayOrder>,
        server_time: u64,
        mode: String,
    },

    #[serde(rename = "OrderAdded")]
    OrderAdded(RelayOrder),

    #[serde(rename = "OrderRemoved")]
    OrderRemoved {
        order_id: String,
        reason: String,
    },

    #[serde(rename = "OrderMatched")]
    OrderMatched {
        order_id: String,
        taker_base: String,
        taker_tru: String,
        #[serde(default)]
        taker_tru_pubkey: Option<String>,
        #[serde(default)]
        signature_base: Option<String>,
    },

    #[serde(rename = "Pong")]
    Pong,

    #[serde(rename = "Error")]
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayOrder {
    pub order_id: String,
    pub side: RelayOrderSide,
    pub amount_tru: f64,
    pub amount_usdc: f64,
    pub price: f64,
    pub hash160_hex: String,
    pub timelock: u32,
    pub maker_tru_address: String,
    pub maker_tru_pubkey: Option<String>,
    pub maker_base_address: String,
    pub created_at: u64,
    pub is_matched: bool,
}
