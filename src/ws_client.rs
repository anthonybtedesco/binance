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
use std::sync::Mutex;
use crate::{functions::hmac_sha256, trade::{OrderSide, OrderState, OrderStatus, OrderTracker, TradeOrder}};

pub struct WsClient {
    status: Arc<std::sync::Mutex<String>>,
    _sender: mpsc::Sender<String>,
    balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
    prices: Arc<TokioMutex<HashMap<String, PriceData>>>,
    exchange_info: Arc<Mutex<ExchangeInfo>>,
    order_tracker: OrderTracker,
}

#[derive(Clone, Debug)]
pub struct AssetBalance {
    pub free: f64,
    pub locked: f64,
    pub total_in_usdt: f64,
    pub total_in_btc: f64,
    pub total_in_usdc: f64,
}

impl AssetBalance {
    pub fn update_valuations(&mut self, prices: &HashMap<String, PriceData>, asset: &str) {
        let total = self.free + self.locked;
        
        // Direct price for USDT/USDC/BTC
        if asset == "USDT" {
            self.total_in_usdt = total;
        } else if asset == "USDC" {
            self.total_in_usdc = total;
        } else if asset == "BTC" {
            self.total_in_btc = total;
        } else {
            // Try to find price through pairs
            debug!("Asset Price: {:?}", prices.get(&format!("{}USDT", asset)));
            if let Some(price) = prices.get(&format!("{}USDT", asset)) {
                self.total_in_usdt = total * price.price;
                println!("{}: Total in USDT: {}", asset, self.total_in_usdt);
            }
            if let Some(price) = prices.get(&format!("{}BTC", asset)) {
                self.total_in_btc = total * price.price;
                println!("{}: Total in BTC: {}", asset, self.total_in_btc);
            }
            if let Some(price) = prices.get(&format!("{}USDC", asset)) {
                self.total_in_usdc = total * price.price;
                println!("{}: Total in USDC: {}", asset, self.total_in_usdc);
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PriceData {
    pub price: f64,
    pub high_24h: f64,
    pub low_24h: f64,
    pub volume: f64,
    pub quote_volume: f64,
    pub last_update: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct ExchangeInfo {
    pub symbols: HashMap<String, SymbolInfo>,
}

#[derive(Clone, Debug)]
pub struct SymbolInfo {
    pub base_asset: String,
    pub quote_asset: String,
    pub status: String,
    pub min_price: f64,
    pub max_price: f64,
    pub tick_size: f64,
    pub min_qty: f64,
    pub max_qty: f64,
    pub step_size: f64,
    pub quantity_precision: usize,
    pub price_precision: usize,
}

impl WsClient {
    pub fn new(order_tracker: OrderTracker) -> Self {
        info!("Initializing WebSocket client");
        let (tx, rx) = mpsc::channel::<String>(100);
        let status = Arc::new(std::sync::Mutex::new("Disconnected".to_string()));
        let balances = Arc::new(TokioMutex::new(HashMap::new()));
        let prices = Arc::new(TokioMutex::new(HashMap::new()));
        let exchange_info = Arc::new(Mutex::new(ExchangeInfo { symbols: HashMap::new() }));
        
        Self::spawn_background_task(rx, status.clone(), balances.clone(), prices.clone(), exchange_info.clone(), order_tracker.clone());

        Self {
            status,
            _sender: tx,
            balances,
            prices,
            exchange_info,
            order_tracker,
        }
    }

    fn spawn_background_task(
        mut rx: mpsc::Receiver<String>,
        status: Arc<std::sync::Mutex<String>>,
        balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
        prices: Arc<TokioMutex<HashMap<String, PriceData>>>,
        exchange_info: Arc<Mutex<ExchangeInfo>>,
        order_tracker: OrderTracker,
    ) {
        tokio::spawn(async move {
            debug!("Starting WebSocket background task");
            if let Err(e) = Self::run_websocket_loop(rx, status.clone(), balances, prices, exchange_info, order_tracker).await {
                error!("WebSocket task error: {}", e);
                *status.lock().unwrap() = format!("Error: {}", e);
            }
        });
    }

    async fn run_websocket_loop(
        mut rx: mpsc::Receiver<String>,
        status: Arc<std::sync::Mutex<String>>,
        balances: Arc<TokioMutex<HashMap<String, AssetBalance>>>,
        prices: Arc<TokioMutex<HashMap<String, PriceData>>>,
        exchange_info: Arc<Mutex<ExchangeInfo>>,
        order_tracker: OrderTracker,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        
        // Load initial data
        Self::load_initial_balances(&client, &balances, &prices).await?;
        Self::load_exchange_info(&client, &exchange_info).await?;
        Self::subscribe_to_market_streams(&exchange_info, &prices).await?;
        
        // Setup WebSocket connection
        let ws_stream = Self::setup_websocket_connection(&client, &status).await?;
        let (write, read) = ws_stream.split();
        
        // Handle WebSocket communication
        Self::handle_websocket_communication(rx, read, write, status, balances, prices, exchange_info, order_tracker).await?;
        
        Ok(())
    }

    async fn load_initial_balances(
        client: &reqwest::Client,
        balances: &Arc<TokioMutex<HashMap<String, AssetBalance>>>,
        prices: &Arc<TokioMutex<HashMap<String, PriceData>>>,
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
            Self::process_balance_update(balances_arr, balances, prices).await;
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
        prices: Arc<TokioMutex<HashMap<String, PriceData>>>,
        exchange_info: Arc<Mutex<ExchangeInfo>>,
        order_tracker: OrderTracker,
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
                    Self::handle_message(&txt, &status, &balances, &prices, &exchange_info, &order_tracker).await?;
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
        prices: &Arc<TokioMutex<HashMap<String, PriceData>>>,
        exchange_info: &Arc<Mutex<ExchangeInfo>>,
        order_tracker: &OrderTracker,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let json: serde_json::Value = serde_json::from_str(txt)?;
        
        match json["e"].as_str() {
            Some("executionReport") => {
                info!("Received execution report: {}", txt);
                if let Some(order_id) = json["i"].as_u64() {
                    let mut orders = order_tracker.lock().unwrap();
                    let mut order = orders.get(&order_id)
                        .cloned()
                        .unwrap_or_else(|| OrderStatus {
                            order_id,
                            symbol: json["s"].as_str().unwrap_or("").to_string(),
                            status: OrderState::New,
                            side: if json["S"].as_str() == Some("BUY") { OrderSide::BUY } else { OrderSide::SELL },
                            price: json["p"].as_str().and_then(|p| p.parse().ok()).unwrap_or(0.0),
                            quantity: json["q"].as_str().and_then(|q| q.parse().ok()).unwrap_or(0.0),
                            filled: 0.0,
                            last_executed_price: None,
                            quote_qty: None,
                            commission: None,
                            last_update: chrono::Utc::now(),
                        });

                    // Update execution status (X is the current status)
                    order.status = match json["X"].as_str() {
                        Some("NEW") => OrderState::New,
                        Some("FILLED") => OrderState::Filled,
                        Some("PARTIALLY_FILLED") => OrderState::PartiallyFilled,
                        Some("CANCELED") => OrderState::Canceled,
                        Some("REJECTED") => OrderState::Rejected,
                        Some("EXPIRED") => OrderState::Expired,
                        Some(status) => {
                            info!("Unknown order status: {}", status);
                            order.status.clone()
                        }
                        None => order.status.clone(),
                    };

                    // Update cumulative filled quantity (z)
                    if let Some(filled_qty) = json["z"].as_str() {
                        if let Ok(qty) = filled_qty.parse::<f64>() {
                            order.filled = qty;
                        }
                    }

                    // Update last executed price (L)
                    if let Some(exec_price) = json["L"].as_str() {
                        if let Ok(price) = exec_price.parse::<f64>() {
                            order.last_executed_price = Some(price);
                        }
                    }

                    // Update cumulative quote asset transacted quantity (Z)
                    if let Some(quote_qty) = json["Z"].as_str() {
                        if let Ok(qty) = quote_qty.parse::<f64>() {
                            order.quote_qty = Some(qty);
                        }
                    }

                    // Update commission asset (N) and amount (n)
                    if let (Some(commission_asset), Some(commission_amt)) = (json["N"].as_str(), json["n"].as_str()) {
                        if let Ok(amt) = commission_amt.parse::<f64>() {
                            order.commission = Some((commission_asset.to_string(), amt));
                        }
                    }

                    // Update timestamp (T)
                    if let Some(timestamp) = json["T"].as_i64() {
                        order.last_update = chrono::DateTime::from_timestamp(
                            timestamp / 1000,
                            ((timestamp % 1000) * 1_000_000) as u32,
                        ).unwrap_or(chrono::Utc::now());
                    }

                    info!("Updated order status: {:?}", order);

                    // Insert the updated order
                    orders.insert(order_id, order);
                }
            }
            Some("outboundAccountPosition") => {
                info!("Received balance update");
                if let Some(balances_arr) = json["B"].as_array() {
                    Self::process_balance_update(balances_arr, balances, prices).await;
                }
            }
            _ => {}
        }
        
        *status.lock().unwrap() = "Receiving messages".to_string();

        // Save orders after update
        if let Ok(orders) = order_tracker.lock() {
            if let Err(e) = TradeOrder::save_orders(&orders) {
                error!("Failed to save orders: {}", e);
            }
        }

        Ok(())
    }

    pub async fn process_balance_update(
        balances_arr: &[serde_json::Value],
        balances: &Arc<TokioMutex<HashMap<String, AssetBalance>>>,
        prices: &Arc<TokioMutex<HashMap<String, PriceData>>>,
    ) {
        let mut balance_map = balances.lock().await;
        let prices_map = prices.lock().await;
        
        for balance in balances_arr {
            if let (Some(asset), Some(free), Some(locked)) = (
                balance["asset"].as_str(),
                balance["free"].as_str(),
                balance["locked"].as_str(),
            ) {
                if let (Ok(free), Ok(locked)) = (free.parse::<f64>(), locked.parse::<f64>()) {
                    if free > 0.0 || locked > 0.0 {
                        let total = free + locked;
                        let mut total_in_usdt = 0.0;
                        let mut total_in_btc = 0.0;
                        let mut total_in_usdc = 0.0;

                        // Direct values for base currencies
                        match asset {
                            "USDT" => total_in_usdt = total,
                            "BTC" => total_in_btc = total,
                            "USDC" => total_in_usdc = total,
                            _ => {
                                // Calculate using current prices
                                if let Some(price) = prices_map.get(&format!("{}USDT", asset)) {
                                    total_in_usdt = total * price.price;
                                }
                                if let Some(price) = prices_map.get(&format!("{}BTC", asset)) {
                                    total_in_btc = total * price.price;
                                }
                                else {
                                    if let Some(usdt_price) = prices_map.get(&format!("{}USDT", asset)) {
                                        if let Some(btc_usdt_price) = prices_map.get("BTCUSDT") {
                                            total_in_btc = total * usdt_price.price / btc_usdt_price.price;
                                        }
                                    }
                                }
                                if let Some(price) = prices_map.get(&format!("{}USDC", asset)) {
                                    total_in_usdc = total * price.price;
                                }
                            }
                        }

                        balance_map.insert(
                            asset.to_string(),
                            AssetBalance { 
                                free, 
                                locked,
                                total_in_usdt,
                                total_in_btc,
                                total_in_usdc,
                            },
                        );
                    }
                }
            }
        }
    }

    pub fn get_status(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    pub async fn get_balances(&self) -> Result<HashMap<String, AssetBalance>, Box<dyn std::error::Error>> {
        Ok(self.balances.lock().await.clone())
    }

    pub fn get_symbols(&self) -> Vec<String> {
        self.exchange_info
            .lock()
            .unwrap()
            .symbols
            .keys()
            .cloned()
            .collect()
    }

    pub async fn get_prices(&self) -> Result<HashMap<String, PriceData>, Box<dyn std::error::Error>> {
        Ok(self.prices.lock().await.clone())
    }

    async fn load_exchange_info(
        client: &reqwest::Client,
        exchange_info: &Arc<Mutex<ExchangeInfo>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Loading exchange information");
        let api_endpoint = env::var("BINANCE_API_ENDPOINT")?;
        
        let resp = client
            .get(format!("{}/api/v3/exchangeInfo", api_endpoint))
            .send()
            .await?;

        let info: serde_json::Value = resp.json().await?;
        
        if let Some(symbols) = info["symbols"].as_array() {
            let mut exchange_data = exchange_info.lock().unwrap();
            for symbol in symbols {
                if let (Some(symbol_str), Some(status), Some(base), Some(quote)) = (
                    symbol["symbol"].as_str(),
                    symbol["status"].as_str(),
                    symbol["baseAsset"].as_str(),
                    symbol["quoteAsset"].as_str(),
                ) {
                    if status == "TRADING" {
                        let filters = symbol["filters"].as_array().unwrap();
                        let mut symbol_info = SymbolInfo {
                            base_asset: base.to_string(),
                            quote_asset: quote.to_string(),
                            status: status.to_string(),
                            min_price: 0.0,
                            max_price: 0.0,
                            tick_size: 0.0,
                            min_qty: 0.0,
                            max_qty: 0.0,
                            step_size: 0.0,
                            quantity_precision: 0,
                            price_precision: 0,
                        };

                        for filter in filters {
                            match filter["filterType"].as_str() {
                                Some("PRICE_FILTER") => {
                                    symbol_info.min_price = filter["minPrice"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                                    symbol_info.max_price = filter["maxPrice"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                                    symbol_info.tick_size = filter["tickSize"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                                },
                                Some("LOT_SIZE") => {
                                    symbol_info.min_qty = filter["minQty"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                                    symbol_info.max_qty = filter["maxQty"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                                    symbol_info.step_size = filter["stepSize"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
                                },
                                _ => {}
                            }
                        }

                        exchange_data.symbols.insert(symbol_str.to_string(), symbol_info);
                    }
                }
            }
            info!("Loaded {} trading pairs", exchange_data.symbols.len());
        }

        Ok(())
    }

    async fn subscribe_to_market_streams(
        exchange_info: &Arc<Mutex<ExchangeInfo>>,
        prices: &Arc<TokioMutex<HashMap<String, PriceData>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let symbols = exchange_info.lock().unwrap().symbols.keys().cloned().collect::<Vec<_>>();
        debug!("Total symbols to subscribe: {}", symbols.len());
        let ws_endpoint = env::var("BINANCE_WS_ENDPOINT")?;
        
        for chunk in symbols.chunks(100) {
            let streams: Vec<String> = chunk.iter()
                .map(|s| format!("{}@ticker", s.to_lowercase()))
                .collect();
            
            let combined_stream_url = format!("{}/stream?streams={}", ws_endpoint, streams.join("/"));
            debug!("Connecting to stream URL: {}", combined_stream_url);
            
            let (ws_stream, _) = connect_async(combined_stream_url).await?;
            let (_write, mut read) = ws_stream.split();

            let prices_clone = prices.clone();
            tokio::spawn(async move {
                debug!("Started price update task for chunk");
                while let Some(msg) = read.next().await {
                    if let Ok(Message::Text(txt)) = msg {
                        debug!("Received market data: {}", txt);
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&txt) {
                            debug!("Parsed JSON: {:?}", json);
                            if let Some(data) = json["data"].as_object() {
                                debug!("Processing market data for symbol: {:?}", data.get("s"));
                                if let (Some(symbol), Some(price), Some(high), Some(low), Some(volume), Some(quote_volume)) = (
                                    data["s"].as_str(),
                                    data["c"].as_str(),
                                    data["h"].as_str(),
                                    data["l"].as_str(),
                                    data["v"].as_str(),
                                    data["q"].as_str(),
                                ) {
                                    if let (Ok(price), Ok(high), Ok(low), Ok(volume), Ok(quote_volume)) = (
                                        price.parse::<f64>(),
                                        high.parse::<f64>(),
                                        low.parse::<f64>(),
                                        volume.parse::<f64>(),
                                        quote_volume.parse::<f64>(),
                                    ) {
                                        let mut prices = prices_clone.lock().await;
                                        debug!("Updating price for {}: {}", symbol, price);
                                        prices.insert(symbol.to_string(), PriceData {
                                            price,
                                            high_24h: high,
                                            low_24h: low,
                                            volume,
                                            quote_volume,
                                            last_update: chrono::Utc::now(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                debug!("Price update task ended for chunk");
            });
        }

        Ok(())
    }

    pub async fn refresh_balances(&self) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        Self::load_initial_balances(&client, &self.balances, &self.prices).await
    }

    pub fn get_symbol_info(&self, symbol: &str) -> Option<SymbolInfo> {
        self.exchange_info
            .lock()
            .unwrap()
            .symbols
            .get(symbol)
            .cloned()
    }

    pub async fn get_exchange_info(&self) -> Result<ExchangeInfo, Box<dyn std::error::Error>> {
        Ok(self.exchange_info.lock().unwrap().clone())
    }

    pub fn set_order_tracker(&mut self, order_tracker: OrderTracker) {
        // Store order_tracker in WsClient struct
        self.order_tracker = order_tracker;
    }
}
