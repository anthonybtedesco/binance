use eframe;
use anyhow::Result;
use dotenv::dotenv;
use binance::gui::BinanceApp;
use binance::ws_client::WsClient;
use std::sync::Arc;
use std::sync::Mutex;
use binance::trade::TradeOrder;

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    env_logger::init();
    dotenv::dotenv().ok();

    let options = eframe::NativeOptions {
        ..Default::default()
    };

    // Load past orders
    let order_tracker = TradeOrder::new_tracker();

    // Create WS client with the loaded orders
    let ws_client = Arc::new(Mutex::new(WsClient::new(order_tracker.clone())));

    // Spawn WebSocket handling task
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            // WebSocket keeps running in background
        }
    });

    // Run GUI with WebSocket client
    eframe::run_native(
        "Binance Trading Bot",
        options,
        Box::new(|_cc| Ok(Box::new(BinanceApp::new(ws_client)))),
    )
}
