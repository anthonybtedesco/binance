use eframe;
use anyhow::Result;
use dotenv::dotenv;
use binance::gui::BinanceApp;
use binance::ws_client::WsClient;
use std::sync::Arc;
use std::sync::Mutex;
use binance::trade::TradeOrder;
use binance::trailing::TrailingOrder;
use binance::trailing_monitor;

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

    // Get price data and trailing orders from WS client
    let price_data = ws_client.lock().unwrap().get_prices_arc();
    let trailing_orders = ws_client.lock().unwrap().get_trailing_orders();

    // Spawn WebSocket handling task
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            // WebSocket keeps running in background
        }
    });

    // Start price monitor for trailing orders
    let monitor_handle = trailing_monitor::start_price_monitor(
        trailing_orders.clone(),
        price_data.clone(),
        order_tracker.clone(),
    );

    // Optional: Cancel the monitor when the app exits
    tokio::spawn(async move {
        if let Err(e) = monitor_handle.await {
            log::error!("Trail monitor error: {}", e);
        }
    });

    // Run GUI with WebSocket client
    eframe::run_native(
        "Binance Trading Bot",
        options,
        Box::new(|_cc| Ok(Box::new(BinanceApp::new(ws_client)))),
    )
}
