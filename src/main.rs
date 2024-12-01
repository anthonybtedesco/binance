use anyhow::Result;
use binance::ws_client::WSClient;
use log::{debug, info};
use serde_json::Value;
use std::sync::Arc;
use tokio::spawn;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

const STREAMS: [&str; 3] = ["btcusdt@ticker", "solusdt@ticker", "ethusdt@ticker"];

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    info!("Running ...");

    debug!("Creating and connecting websocket ...");

    let ws = WSClient::init(dotenv::var("BINANCE_WS_ENDPOINT").unwrap())
        .await
        .expect("Valid WebSocket");

    let ws = Arc::new(Mutex::new(ws));

    for s in STREAMS {
        let mut ws = ws.lock().await;
        ws.add_stream(s).await?;
    }

    let (tx, mut rx) = mpsc::channel::<String>(32);

    let ws_clone = Arc::clone(&ws);
    spawn(handle_incoming_messages(ws_clone, tx));

    let ws_clone = Arc::clone(&ws);
    spawn(handle_ping_pong(ws_clone));

    loop {
        if let Some(message) = rx.recv().await {
            debug!("Received message from WebSocket: {}", message);

            let json_message: Value = serde_json::from_str(&message).expect("Must be valid JSON");

            debug!("Json Message: {}", json_message);
        }
    }
}

async fn handle_ping_pong(ws: Arc<Mutex<WSClient>>) {
    info!("Handling ping-pong...");
    loop {
        let mut ws = ws.lock().await;
        if let Err(e) = ws.handle_ping_pong().await {
            log::error!("Error handling ping-pong: {}", e);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

async fn handle_incoming_messages(ws: Arc<Mutex<WSClient>>, tx: mpsc::Sender<String>) {
    info!("Receiving messages ...");
    let mut ws = ws.lock().await;
    let _ = ws.init_user_stream().await;

    loop {
        match ws.receive_message().await {
            Ok(message) => {
                if let Message::Text(text) = message {
                    tx.send(text)
                        .await
                        .expect("Failed to send message to channel");
                }
            }
            Err(e) => {
                log::error!("Error receiving WebSocket message: {}", e);
                break;
            }
        }
    }
}

