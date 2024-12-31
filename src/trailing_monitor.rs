use crate::trailing::{TrailingOrder, TrailingOrderMap};
use crate::trade::OrderTracker;
use crate::ws_client::PriceData;
use std::collections::HashMap;
use tokio::sync::Mutex as TokioMutex;
use std::sync::Arc;
use log::{error, info};

pub struct TrailingMonitor {
    trailing_orders: TrailingOrderMap,
    prices: Arc<TokioMutex<HashMap<String, PriceData>>>,
    order_tracker: OrderTracker,
}

impl TrailingMonitor {
    pub fn new(prices: Arc<TokioMutex<HashMap<String, PriceData>>>, order_tracker: OrderTracker) -> Self {
        let trailing_orders = TrailingOrder::new_tracker();
        Self { trailing_orders, prices, order_tracker }
    }

    pub fn start(self) {
        tokio::spawn(async move {
            self.monitor_loop().await;
        });
    }

    async fn monitor_loop(&self) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(100));
        let mut last_log = std::time::Instant::now();

        loop {
            interval.tick().await;
            self.process_price_updates(&mut last_log).await;
        }
    }

    async fn process_price_updates(&self, last_log: &mut std::time::Instant) {
        let prices = self.prices.lock().await;
        let mut trails = self.trailing_orders.lock().await;

        // Log status every 30 seconds
        if last_log.elapsed() >= std::time::Duration::from_secs(30) {
            self.log_status(&trails);
            *last_log = std::time::Instant::now();
        }

        // Update trails and collect triggers
        for (id, trail) in trails.iter_mut() {
            if let Some(price_data) = prices.get(&trail.symbol) {
                let old_price = trail.last_price;
                trail.update_price(price_data.price);
                
                // Log price updates
                info!("Trail {}: {} price update {} -> {} (trigger: {:.8}, delta: {:.2}%)", 
                    id,
                    trail.symbol,
                    old_price,
                    trail.last_price,
                    trail.trigger_price,
                    trail.delta_percentage
                );

                // Log if getting close to trigger
                if (trail.last_price - trail.trigger_price).abs() / trail.trigger_price < 0.01 {
                    info!("⚠️ Trail {} is within 1% of trigger price!", id);
                }
            }
        }

        // Save updates
        if let Err(e) = TrailingOrder::save_trails(&trails) {
            error!("Failed to save trail updates: {}", e);
        }
    }

    fn log_status(&self, trails: &HashMap<String, TrailingOrder>) {
        info!("📊 Active trailing orders status:");
        for (id, trail) in trails.iter() {
            info!(
                "🔄 Trail {}: {:?} {} @ {:.8} (trigger: {:.8}, delta: {:.2}%, value: ${:.2})",
                id, trail.side, trail.symbol, trail.last_price, trail.trigger_price,
                trail.delta_percentage, trail.usdt_value
            );
        }
    }
} 