use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;

pub struct Asset {
    pub symbol: String,
    pub amount: f64,
}

#[derive(Debug, Deserialize)]
pub struct Balance {
    pub asset: String,
    pub free: String,
    pub locked: String,
}

#[derive(Debug, Deserialize)]
struct AccountInfo {
    pub balances: Vec<Balance>,
}

impl Asset {
    pub async fn get_all_assets() -> Result<Vec<Balance>, Box<dyn std::error::Error>> {
        let url = "https://api.binance.com/api/v3/account";
        let timestamp = chrono::Utc::now().timestamp_millis();

        let api_key = dotenv::var("API_KEY")?;
        let secret_key = dotenv::var("SECRET_KEY")?;

        // Prepare query string with timestamp
        let query_string = format!("timestamp={}", timestamp);

        // Generate HMAC signature
        let key = hmac_sha256::HMAC::mac(query_string.as_bytes(), secret_key.as_bytes());
        let signature = hex::encode(key);

        // Append signature to the query string
        let query_with_signature = format!("{}&signature={}", query_string, signature);

        // Set up headers
        let mut headers = HeaderMap::new();
        headers.insert("X-MBX-APIKEY", HeaderValue::from_str(api_key.as_str())?);

        // Make the GET request
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}?{}", url, query_with_signature))
            .headers(headers)
            .send()
            .await?;

        // Parse JSON response
        if response.status().is_success() {
            let account_info: AccountInfo = response.json().await?;
            Ok(account_info.balances)
        } else {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to fetch assets: {}", response.text().await?),
            )))
        }
    }
}

