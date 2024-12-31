use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use log::{error, info};
use crate::trade::{OrderSide, OrderType, TradeOrder};
use tokio::sync::Mutex as TokioMutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrailingOrder {
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: f64,
    pub delta_percentage: f64,  // The trailing percentage
    pub initial_price: f64,     // Price when tracking started
    pub last_price: f64,        // Last known price
    pub highest_price: f64,     // Highest price seen (for sell stops)
    pub lowest_price: f64,      // Lowest price seen (for buy stops)
    pub trigger_price: f64,     // Price at which to execute
    pub usdt_value: f64,        // Estimated USDT value of trade
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub type TrailingOrderMap = Arc<TokioMutex<HashMap<String, TrailingOrder>>>;

impl TrailingOrder {
    pub fn new(symbol: String, side: OrderSide, quantity: f64, delta_percentage: f64, current_price: f64) -> Self {
        let trigger_price = match &side {
            OrderSide::SELL => current_price * (1.0 - delta_percentage / 100.0),
            OrderSide::BUY => current_price * (1.0 + delta_percentage / 100.0),
        };
        
        Self {
            symbol,
            side,
            quantity,
            delta_percentage,
            initial_price: current_price,
            last_price: current_price,
            highest_price: current_price,
            lowest_price: current_price,
            trigger_price,
            usdt_value: quantity * current_price,
            created_at: chrono::Utc::now(),
        }
    }

    pub fn update_price(&mut self, new_price: f64) -> bool {
        self.last_price = new_price;
        let should_trigger = match self.side {
            OrderSide::SELL => {
                if new_price > self.highest_price {
                    self.highest_price = new_price;
                    self.trigger_price = new_price * (1.0 - self.delta_percentage / 100.0);
                }
                new_price <= self.trigger_price
            },
            OrderSide::BUY => {
                if new_price < self.lowest_price {
                    self.lowest_price = new_price;
                    self.trigger_price = new_price * (1.0 + self.delta_percentage / 100.0);
                }
                new_price >= self.trigger_price
            },
        };
        self.usdt_value = self.quantity * new_price;
        should_trigger
    }

    pub fn to_trade_order(&self) -> TradeOrder {
        TradeOrder {
            symbol: self.symbol.clone(),
            side: self.side.clone(),
            order_type: OrderType::LIMIT,
            quantity: self.quantity,
            price: Some(self.last_price),
        }
    }

    pub fn save_trails(trails: &HashMap<String, TrailingOrder>) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(trails)?;
        fs::write("trails.json", json)?;
        Ok(())
    }

    pub fn load_trails() -> Result<HashMap<String, TrailingOrder>, Box<dyn std::error::Error>> {
        if Path::new("trails.json").exists() {
            let json = fs::read_to_string("trails.json")?;
            let trails: HashMap<String, TrailingOrder> = serde_json::from_str(&json)?;
            Ok(trails)
        } else {
            Ok(HashMap::new())
        }
    }

    pub fn new_tracker() -> TrailingOrderMap {
        let trails = Self::load_trails().unwrap_or_else(|e| {
            error!("Failed to load trailing orders: {}", e);
            HashMap::new()
        });
        info!("Loaded {} trailing orders", trails.len());
        Arc::new(TokioMutex::new(trails))
    }
} 