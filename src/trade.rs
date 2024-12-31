use std::error::Error;
use log::info;
use serde::{Deserialize, Serialize};
use std::fs;
use crate::ws_client::{WsClient, PriceData, SymbolInfo};
use std::collections::HashMap;
use std::sync::Arc;
use std::path::Path;

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
    BUY,
    SELL,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderType {
    LIMIT,
    MARKET,
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
    pub last_executed_price: Option<f64>,
    pub quote_qty: Option<f64>,
    pub commission: Option<(String, f64)>,  // (asset, amount)
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

pub type OrderTracker = Arc<std::sync::Mutex<HashMap<u64, OrderStatus>>>;

impl TradeOrder {
    pub fn optimize_order(&mut self, symbol_info: &SymbolInfo) -> Result<(), Box<dyn Error>> {
        // Helper function to round to step size
        fn round_to_step(value: f64, step_size: f64) -> f64 {
            (value / step_size).floor() * step_size
        }

        // Round quantity to step size
        self.quantity = round_to_step(self.quantity, symbol_info.step_size);
        
        // Ensure quantity is within bounds
        self.quantity = self.quantity.max(symbol_info.min_qty).min(symbol_info.max_qty);
        
        // For limit orders, round price to tick size
        if let Some(price) = self.price.as_mut() {
            *price = round_to_step(*price, symbol_info.tick_size);
        }

        info!("Optimized order: qty={}, price={:?}", self.quantity, self.price);
        Ok(())
    }

    pub fn save_orders(orders: &HashMap<u64, OrderStatus>) -> Result<(), Box<dyn Error>> {
        info!("Saving orders to file");
        let json = serde_json::to_string_pretty(orders)?;
        fs::write("orders.json", json)?;
        Ok(())
    }

    pub async fn submit(&mut self, order_tracker: OrderTracker) -> Result<(), Box<dyn Error>> {
        info!("Submitting order");
        let api_key = std::env::var("API_KEY")?;
        let api_secret = std::env::var("SECRET_KEY")?;
        let api_endpoint = std::env::var("BINANCE_API_ENDPOINT")?;

        // Get current market data for optimization
        let client = reqwest::Client::new();
        let ticker_url = format!("{}/api/v3/ticker/price?symbol={}", api_endpoint, self.symbol);
        let ticker: serde_json::Value = client.get(&ticker_url).send().await?.json().await?;
        let current_price = ticker["price"].as_str()
            .ok_or("No price found")?
            .parse::<f64>()?;

        // Get symbol info for precision
        let exchange_info_url = format!("{}/api/v3/exchangeInfo?symbol={}", api_endpoint, self.symbol);
        let info: serde_json::Value = client.get(&exchange_info_url).send().await?.json().await?;
        
        if let Some(symbol) = info["symbols"].as_array()
            .and_then(|symbols| symbols.first()) {
            
            let mut symbol_info = SymbolInfo {
                quantity_precision: symbol["baseAssetPrecision"].as_i64().unwrap_or(8) as usize,
                price_precision: symbol["quotePrecision"].as_i64().unwrap_or(8) as usize,
                base_asset: symbol["baseAsset"].as_str().unwrap_or("").to_string(),
                quote_asset: symbol["quoteAsset"].as_str().unwrap_or("").to_string(),
                status: symbol["status"].as_str().unwrap_or("").to_string(),
                min_price: 0.0,
                max_price: f64::MAX,
                tick_size: 0.0,
                min_qty: 0.0,
                max_qty: f64::MAX,
                step_size: 0.0,
            };

            // Extract filter values
            if let Some(filters) = symbol["filters"].as_array() {
                for filter in filters {
                    match filter["filterType"].as_str() {
                        Some("PRICE_FILTER") => {
                            symbol_info.min_price = filter["minPrice"].as_str()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0.0);
                            symbol_info.max_price = filter["maxPrice"].as_str()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(f64::MAX);
                            symbol_info.tick_size = filter["tickSize"].as_str()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0.0);
                        },
                        Some("LOT_SIZE") => {
                            symbol_info.min_qty = filter["minQty"].as_str()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0.0);
                            symbol_info.max_qty = filter["maxQty"].as_str()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(f64::MAX);
                            symbol_info.step_size = filter["stepSize"].as_str()
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0.0);
                        },
                        _ => {}
                    }
                }
            }

            // Optimize the order with proper filter values
            self.optimize_order(&symbol_info)?;
        }

        info!("Submitting optimized order for {}", self.symbol);
        
        let timestamp = chrono::Utc::now().timestamp_millis();
        let recv_window = 5000;

        // Build base parameters
        let mut params = vec![
            ("symbol", self.symbol.clone()),
            ("side", format!("{:?}", self.side)),
            ("type", format!("{:?}", self.order_type)),
            ("quantity", self.quantity.to_string()),
            ("timestamp", timestamp.to_string()),
            ("recvWindow", recv_window.to_string()),
        ];

        // Add timeInForce for LIMIT orders
        if let OrderType::LIMIT = self.order_type {
            params.push(("timeInForce", "GTC".to_string()));
            // Add price for LIMIT orders
            if let Some(price) = self.price {
                params.push(("price", price.to_string()));
            }
        }

        

        // Sort parameters alphabetically for consistent signing
        params.sort_by(|a, b| a.0.cmp(b.0));
        
        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<String>>()
            .join("&");

        // Calculate signature using the query string
        let signature = crate::functions::hmac_sha256(&api_secret, &query_string);

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/api/v3/order", api_endpoint))
            .header("X-MBX-APIKEY", api_key)
            .query(&params)
            .query(&[("signature", signature)])
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
            last_executed_price: None,
            quote_qty: None,
            commission: None,
            last_update: chrono::Utc::now(),
        };

        let mut order_tracker = order_tracker.lock().unwrap();
        order_tracker.insert(order_id, order_status);

        Ok(())
    }

    pub fn new_tracker() -> OrderTracker {
        let orders = Self::load_orders().unwrap_or_else(|e| {
            log::error!("Failed to load orders: {}", e);
            HashMap::new()
        });
        info!("Loaded {} past orders", orders.len());
        Arc::new(std::sync::Mutex::new(orders))
    }

    pub fn load_orders() -> Result<HashMap<u64, OrderStatus>, Box<dyn Error>> {
        if Path::new("orders.json").exists() {
            let json = fs::read_to_string("orders.json")?;
            let orders: HashMap<u64, OrderStatus> = serde_json::from_str(&json)?;
            Ok(orders)
        } else {
            Ok(HashMap::new())
        }
    }
}