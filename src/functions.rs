use hmac::{Hmac, Mac};
use sha2::Sha256;

pub fn hmac_sha256(secret: &str, message: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}