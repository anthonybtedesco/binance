use std::error::Error;
use log::info;
use serde::{Deserialize, Serialize};
use crate::ws_client::{WsClient, PriceData, SymbolInfo};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOrder {
    pub symbol: String,
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: f64,
    pub price: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderType {
    Limit,
    Market,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderStatus {
    pub order_id: u64,
    pub symbol: String,
    pub status: OrderState,
    pub side: OrderSide,
    pub price: f64,
    pub quantity: f64,
    pub filled: f64,
    pub last_update: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderState {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Rejected,
    Expired,
}

pub type OrderTracker = Arc<Mutex<HashMap<u64, OrderStatus>>>;

impl TradeOrder {
    pub async fn submit(&self, order_tracker: OrderTracker) -> Result<(), Box<dyn Error>> {
        let api_key = std::env::var("BINANCE_API_KEY")?;
        let api_secret = std::env::var("BINANCE_API_SECRET")?;

        let timestamp = chrono::Utc::now().timestamp_millis();
        let recv_window = 5000;

        let mut params = vec![
            ("symbol", self.symbol.clone()),
            ("side", format!("{:?}", self.side)),
            ("type", format!("{:?}", self.order_type)),
            ("timestamp", timestamp.to_string()),
            ("recvWindow", recv_window.to_string()),
        ];

        if let Some(price) = self.price {
            params.push(("price", price.to_string()));
        }
        params.push(("quantity", self.quantity.to_string()));

        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join("&");

        let signature = crate::functions::hmac_sha256(&query_string, &api_secret);
        let final_query = format!("{}&signature={}", query_string, signature);

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.binance.com/api/v3/order")
            .header("X-MBX-APIKEY", api_key)
            .body(final_query)
            .send()
            .await?;

        info!("Order submission response: {:?}", response);

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Order submission failed: {}", error_text).into());
        }

        let order_response: serde_json::Value = response.json().await?;

        let order_id = order_response["orderId"].as_u64().unwrap();
        let order_status = OrderStatus {
            order_id,
            symbol: self.symbol.clone(),
            status: OrderState::New,
            side: self.side.clone(),
            price: self.price.unwrap_or(0.0),
            quantity: self.quantity,
            filled: 0.0,
            last_update: chrono::Utc::now(),
        };

        let mut order_tracker = order_tracker.lock().await;
        order_tracker.insert(order_id, order_status);

        Ok(())
    }

    pub fn new_tracker() -> OrderTracker {
        Arc::new(Mutex::new(HashMap::new()))
    }
}