use anyhow::Result;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info};
use reqwest::Client;
use serde_json::json;
use std::error::Error;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub struct WSClient {
    pub read: SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    pub write: SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    pub streams: Vec<String>,
}

impl WSClient {
    pub async fn init<S>(url: S) -> Result<Self, Box<dyn Error>>
    where
        S: ToString,
    {
        let url_str = url.to_string();
        info!("Connecting to WebSocket at {}", url_str);

        let (ws_stream, _) = connect_async(format!("{url_str}/stream")).await?;

        let (write, read) = ws_stream.split();

        info!("WebSocket connection established.");

        Ok(WSClient {
            read,
            write,
            streams: vec![],
        })
    }

    // Send a message over the WebSocket
    pub async fn send_message(&mut self, message: Message) -> Result<(), WsError> {
        debug!("Sending message: {:?}", message);
        self.write.send(message).await
    }

    // Receive a message from the WebSocket
    pub async fn receive_message(&mut self) -> Result<Message, WsError> {
        debug!("Waiting for a new message...");
        self.read.next().await.ok_or(WsError::ConnectionClosed)?
    }

    // Add a new stream to the list of streams
    pub async fn add_stream<S>(&mut self, stream: S) -> Result<(), WsError>
    where
        S: ToString,
    {
        let stream_str = stream.to_string();

        if !self.streams.contains(&stream_str) {
            self.streams.push(stream_str.clone());
            info!("Added new stream: {}", stream_str);

            // Send SUBSCRIBE message to WebSocket
            let subscribe_msg = json!({
                "method": "SUBSCRIBE",
                "params": [stream_str],
                "id": self.streams.len() as u64, // A unique ID for each subscription request
            });
            let message = Message::Text(subscribe_msg.to_string());
            self.send_message(message).await?;
        }

        Ok(())
    }

    // Remove a stream from the list of streams
    pub async fn remove_stream<S>(&mut self, stream: S) -> Result<(), WsError>
    where
        S: ToString,
    {
        let stream_str = stream.to_string();

        if let Some(pos) = self.streams.iter().position(|x| *x == stream_str) {
            self.streams.remove(pos);
            info!("Removed stream: {}", stream_str);

            // Send UNSUBSCRIBE message to WebSocket
            let unsubscribe_msg = json!({
                "method": "UNSUBSCRIBE",
                "params": [stream_str],
                "id": self.streams.len() as u64, // Unique ID for unsubscribe request
            });
            let message = Message::Text(unsubscribe_msg.to_string());
            self.send_message(message).await?;
        }

        Ok(())
    }

    // List all active streams
    pub async fn list_streams(&mut self) -> Result<(), WsError> {
        let list_msg = json!({
            "method": "LIST_SUBSCRIPTIONS",
            "id": 3
        });
        let message = Message::Text(list_msg.to_string());
        self.send_message(message).await?;
        Ok(())
    }

    // Build the combined URL with active streams
    fn build_combined_url(&self) -> String {
        let streams = self.streams.join("/");
        format!(
            "{}/stream?streams={}",
            dotenv::var("BINANCE_WS_ENDPOINT").expect("BINANCE_WS_ENDPOINT should be valid"),
            streams
        )
    }

    // Subscribe to the combined stream
    pub async fn subscribe_to_combined_stream(&mut self) -> Result<(), WsError> {
        let url = self.build_combined_url();
        info!("Subscribing to combined stream: {}", url);

        let (ws_stream, _) = connect_async(url).await?;
        let (write, read) = ws_stream.split();

        self.write = write;
        self.read = read;

        info!("Successfully subscribed to combined stream.");

        Ok(())
    }

    // Handle Ping/Pong to keep the connection alive
    pub async fn handle_ping_pong(&mut self) -> Result<(), Box<dyn Error>> {
        info!("Starting Ping/Pong handler...");
        loop {
            match self.read.next().await {
                Some(Ok(Message::Ping(payload))) => {
                    debug!("Received Ping with payload: {:?}", payload);
                    self.send_message(Message::Pong(payload.clone())).await?;
                    info!("Sent Pong with payload: {:?}", &payload);
                }
                Some(Ok(Message::Pong(_))) => {
                    debug!("Received Pong (unsolicited)");
                }
                Some(Ok(Message::Text(text))) => {
                    info!("Received text message: {}", text);
                }
                Some(Ok(Message::Binary(_))) => {
                    debug!("Received binary message");
                }
                Some(Err(e)) => {
                    error!("Error while receiving message: {}", e);
                    break;
                }
                None => {
                    error!("WebSocket closed or error occurred.");
                    break;
                }
                _ => {
                    debug!("Received unsupported message type");
                }
            }
            sleep(Duration::from_secs(30)).await; // Sleep to manage the cycle and reduce load
        }

        info!("Ping/Pong handler stopped.");
        Ok(())
    }

    pub async fn get_listen_key() -> anyhow::Result<()> {
        info!("Getting listening key ...");
        let api_key = dotenv::var("API_KEY")?;
        let api_secret = dotenv::var("SECRET_KEY")?;
        let api_url = dotenv::var("BINANCE_API_ENDPOINT")?;
        debug!("{}", format!("{api_key} | {api_secret} | {api_url}"));

        let client = Client::new();

        let url = format!("{api_url}/api/v3/userDataStream");

        let res = client
            .post(url)
            .header("X-MBX-APIKEY", api_key)
            .send()
            .await?;

        debug!("Response: {:?}", res);

        if !res.status().is_success() {
            return Err(anyhow::format_err!("Error Fetching ListenKey"));
        }

        Ok(())
    }

    pub async fn init_user_stream(&mut self) -> Result<()> {
        debug!("Initializing User Stream ...");
        WSClient::get_listen_key().await?;
        Ok(())
    }
}
