use eframe;
use anyhow::Result;
use dotenv::dotenv;
use binance::gui::BinanceApp;
use binance::ws_client::WsClient;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    env_logger::init();
    dotenv().ok();

    let ws_client = Arc::new(WsClient::new());
    let ws_client_gui = ws_client.clone();

    // Spawn WebSocket handling task
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            // WebSocket keeps running in background
        }
    });

    // Run GUI with WebSocket client
    eframe::run_native(
        "Binance Trading Interface",
        eframe::NativeOptions::default(),
        Box::new(move |_cc| Ok(Box::new(BinanceApp::new(ws_client_gui)))),
    )
}
