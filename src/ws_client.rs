use futures::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, WebSocketStream, MaybeTlsStream};
use std::env;
use std::sync::Arc;
use tokio::sync::mpsc;
use reqwest;
use std::collections::HashMap;
use tokio::sync::Mutex as TokioMutex;
use log::{debug, info, error};
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn hmac_sha256(secret: &str, message: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub struct WsClient {
    status: Arc<std::sync::Mutex<String>>,
    _sender: mpsc::Sender<String>,
    balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
}

#[derive(Clone, Debug)]
pub struct AssetBalance {
    pub free: f64,
    pub locked: f64,
}

impl WsClient {
    pub fn new() -> Self {
        info!("Initializing WebSocket client");
        let (tx, rx) = mpsc::channel::<String>(100);
        let status = Arc::new(std::sync::Mutex::new("Disconnected".to_string()));
        let balances = Arc::new(TokioMutex::new(HashMap::new()));
        
        Self::spawn_background_task(rx, status.clone(), balances.clone());

        Self {
            status,
            _sender: tx,
            balances,
        }
    }

    fn spawn_background_task(
        mut rx: mpsc::Receiver<String>,
        status: Arc<std::sync::Mutex<String>>,
        balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    ) {
        tokio::spawn(async move {
            debug!("Starting WebSocket background task");
            if let Err(e) = Self::run_websocket_loop(rx, status.clone(), balances).await {
                error!("WebSocket task error: {}", e);
                *status.lock().unwrap() = format!("Error: {}", e);
            }
        });
    }

    async fn run_websocket_loop(
        mut rx: mpsc::Receiver<String>,
        status: Arc<std::sync::Mutex<String>>,
        balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        
        // Load initial balances
        Self::load_initial_balances(&client, &balances).await?;
        
        // Setup WebSocket connection
        let ws_stream = Self::setup_websocket_connection(&client, &status).await?;
        let (write, read) = ws_stream.split();
        
        // Handle WebSocket communication
        Self::handle_websocket_communication(rx, read, write, status, balances).await?;
        
        Ok(())
    }

    async fn load_initial_balances(
        client: &reqwest::Client,
        balances: &Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Fetching initial account balances");
        let api_key = env::var("API_KEY")?;
        let api_endpoint = env::var("BINANCE_API_ENDPOINT")?;
        let api_secret = env::var("SECRET_KEY")?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .to_string();

        let signature_payload = format!("timestamp={}", timestamp);
        let signature = hmac_sha256(&api_secret, &signature_payload);

        let resp = client
            .get(format!("{}/api/v3/account", api_endpoint))
            .header("X-MBX-APIKEY", &api_key)
            .query(&[("timestamp", timestamp), ("signature", signature)])
            .send()
            .await?;

        let account: serde_json::Value = resp.json().await?;

        debug!("Account data: {}", account);
        
        if let Some(balances_arr) = account["balances"].as_array() {
            Self::process_balance_update(balances_arr, balances).await;
        }

        Ok(())
    }

    async fn setup_websocket_connection(
        client: &reqwest::Client,
        status: &Arc<std::sync::Mutex<String>>,
    ) -> Result<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, Box<dyn std::error::Error>> {
        info!("Getting listen key for WebSocket stream");
        let api_key = env::var("API_KEY")?;
        let ws_endpoint = env::var("BINANCE_WS_ENDPOINT")?;
        
        let listen_key = Self::get_listen_key(client).await?;
        let ws_url = format!("{}/ws/{}", ws_endpoint, listen_key);
        
        info!("Connecting to WebSocket at {}", ws_url);
        let (ws_stream, _) = connect_async(ws_url).await?;
        info!("WebSocket connection established");
        
        *status.lock().unwrap() = "Connected".to_string();
        Ok(ws_stream)
    }

    async fn get_listen_key(client: &reqwest::Client) -> Result<String, Box<dyn std::error::Error>> {
        let api_key = env::var("API_KEY")?;
        let api_endpoint = env::var("BINANCE_API_ENDPOINT")?;
        
        let resp = client
            .post(format!("{}/api/v3/userDataStream", api_endpoint))
            .header("X-MBX-APIKEY", &api_key)
            .send()
            .await?;

        let json: serde_json::Value = resp.json().await?;
        Ok(json["listenKey"].as_str().ok_or("No listen key")?.to_string())
    }

    async fn handle_websocket_communication(
        mut rx: mpsc::Receiver<String>,
        mut read: SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>,
        mut write: SplitSink<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, Message>,
        status: Arc<std::sync::Mutex<String>>,
        balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Spawn message sender task
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(e) = write.send(Message::Text(msg)).await {
                    error!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
        });

        // Handle incoming messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(txt)) => {
                    Self::handle_message(&txt, &status, &balances).await?;
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        info!("WebSocket connection closed");
        *status.lock().unwrap() = "Disconnected".to_string();
        Ok(())
    }

    async fn handle_message(
        txt: &str,
        status: &Arc<std::sync::Mutex<String>>,
        balances: &Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        debug!("Received WebSocket message: {}", txt);
        let json: serde_json::Value = serde_json::from_str(txt)?;
        
        if json["e"] == "outboundAccountPosition" {
            info!("Received balance update");
            if let Some(balances_arr) = json["B"].as_array() {
                Self::process_balance_update(balances_arr, balances).await;
            }
        }
        
        *status.lock().unwrap() = "Receiving messages".to_string();
        Ok(())
    }

    async fn process_balance_update(
        balances_arr: &[serde_json::Value],
        balances: &Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    ) {
        let mut balance_map = balances.lock().await;
        debug!("Processing {} balances", balances_arr.len());
        
        for balance in balances_arr {
            if let (Some(asset), Some(free), Some(locked)) = (
                balance["asset"].as_str(),
                balance["free"].as_str(),
                balance["locked"].as_str(),
            ) {
                if let (Ok(free), Ok(locked)) = (free.parse::<f64>(), locked.parse::<f64>()) {
                    if free > 0.0 || locked > 0.0 {
                        debug!("Updated balance for {}: Free={}, Locked={}", asset, free, locked);
                        balance_map.insert(
                            asset.to_string(),
                            AssetBalance { free, locked },
                        );
                    }
                }
            }
        }
        
        info!("Loaded {} non-zero balances", balance_map.len());
    }

    pub fn get_status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    pub async fn get_balances(&self) -> Result<HashMap<String, AssetBalance>, Box<dyn std::error::Error>> {
        Ok(self.balances.lock().await.clone())
    }
}
