use anyhow::Result;
use hmac::{Hmac, Mac};
use log::{debug, info};
use reqwest::Client;
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Function to create an order on Binance.
pub async fn create_order(
    symbol: &str,
    side: &str,
    order_type: &str,
    time_in_force: &str,
    quantity: f64,
    price: f64,
    recv_window: Option<u64>,
) -> Result<Value> {
    info!("Creating order...");

    // Load environment variables.
    let api_key = dotenv::var("API_KEY")?;
    let api_secret = dotenv::var("SECRET_KEY")?;
    let api_url = dotenv::var("BINANCE_API_ENDPOINT")?;

    let qty = quantity.to_string();
    let pri = price.to_string();

    debug!("API_KEY: {}", api_key);
    debug!("API_SECRET: [HIDDEN]");
    debug!("API_URL: {}", api_url);

    // Prepare the request parameters.
    let mut params = HashMap::new();
    params.insert("symbol", symbol);
    params.insert("side", side);
    params.insert("type", order_type);
    params.insert("quantity", qty.as_str());
    params.insert("price", pri.as_str());
    params.insert("timeInForce", time_in_force);

    // Add a timestamp.
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .to_string();
    params.insert("timestamp", &timestamp);

    // Construct the query string.
    let query_string: String = params
        .iter()
        .map(|(key, value)| format!("{}={}", key, value))
        .collect::<Vec<String>>()
        .join("&");

    // Generate HMAC SHA256 signature.
    let mut mac = Hmac::<Sha256>::new_from_slice(api_secret.as_bytes())?;
    mac.update(query_string.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());

    // Add the signature to the query string.
    let signed_query_string = format!("{}&signature={}", query_string, signature);

    // Send the request.
    let client = Client::new();
    let url = format!("{}/api/v3/order?{}", api_url, signed_query_string);

    debug!("Request URL: {}", url);

    let res = client
        .post(&url)
        .header("X-MBX-APIKEY", api_key)
        .send()
        .await?;

    debug!("Response status: {}", res.status());
    let body = serde_json::from_str(res.text().await?.as_str())?;
    debug!("Response body: {}", body);

    Ok(body)
}
