use anyhow::Result;
use binance::models::PriceData;
use binance::user_data::Asset;
use binance::ws_client::WSClient;
use log::{debug, info};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

async fn handle_ping_pong(ws: Arc<Mutex<WSClient>>) {
    debug!("Handling ping-pong...");
    loop {
        let mut ws = ws.lock().await;
        if let Err(e) = ws.handle_ping_pong().await {
            log::error!("Ping-pong error: {}", e);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

async fn handle_incoming_messages(ws: Arc<Mutex<WSClient>>, tx: mpsc::Sender<String>) {
    debug!("Receiving messages...");
    let mut ws = ws.lock().await;
    loop {
        match ws.receive_message().await {
            Ok(Message::Text(text)) => {
                if let Err(e) = tx.send(text).await {
                    log::error!("Channel send error: {}", e);
                    break;
                }
            }
            Err(e) => {
                log::error!("Message receive error: {}", e);
                break;
            }
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    dotenv::dotenv()?;
    info!("Running ...");

    debug!("Creating and connecting websocket ...");

    let ws = WSClient::init(dotenv::var("BINANCE_WS_ENDPOINT")?)
        .await
        .expect("Valid WebSocket");

    let ws = Arc::new(Mutex::new(ws));
    let price_data = Arc::new(Mutex::new(HashMap::new()));

    let assets = Asset::get_all_assets().await.unwrap();
    for asset in assets {
        if asset.free.parse::<f64>()? > 0.0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            info!("Asset: {:?}", asset);
            let mut ws = ws.lock().await;
            ws.add_stream(format!("{}usdt@ticker", asset.asset.to_lowercase()))
                .await?
        }
    }

    let (tx, mut rx) = mpsc::channel::<String>(32);

    // Spawn tasks
    let ws_clone = Arc::clone(&ws);
    spawn(handle_incoming_messages(ws_clone, tx));

    let ws_clone = Arc::clone(&ws);
    spawn(handle_ping_pong(ws_clone));

    let price_data_clone = Arc::clone(&price_data);
    tokio::spawn(async move {
        loop {
            let price_data_snapshot = {
                let price_data_lock = price_data_clone.lock().await;
                serde_json::to_string(&*price_data_lock).unwrap_or_else(|_| "{}".to_string())
            };
            info!("Price Data Snapshot: {}", price_data_snapshot);
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });

    while let Some(message) = rx.recv().await {
        debug!("Received message: {}", message);

        if let Ok(json_message) = serde_json::from_str::<Value>(&message) {
            if let Some(stream) = json_message["stream"].as_str() {
                let pair = stream.split('@').next().unwrap_or_default();
                if let Some(price) = json_message["data"]["c"].as_str() {
                    if let Ok(parsed_price) = price.parse::<f64>() {
                        let mut price_data_lock = price_data.lock().await;
                        price_data_lock.insert(
                            pair.to_string(),
                            PriceData {
                                current_price: parsed_price,
                            },
                        );
                        debug!("Updated {} price to {}", pair, parsed_price);
                    }
                }
            } else {
                info!("Recieved: {json_message}");
            }
        }
    }

    Ok(())
}
