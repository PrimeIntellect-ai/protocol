// request_signer.rs
use crate::web3::wallet::Wallet;
use alloy::signers::Signer;
pub async fn sign_request(
    endpoint: &str,
    wallet: &Wallet,
    data: Option<&serde_json::Value>,
) -> Result<String, Box<dyn std::error::Error>> {
    let request_data_string = if let Some(data) = data {
        let mut request_data = serde_json::to_value(data)?;
        if let Some(obj) = request_data.as_object_mut() {
            let sorted_keys: Vec<String> = obj.keys().cloned().collect();
            *obj = sorted_keys
                .into_iter()
                .map(|key| (key.clone(), obj.remove(&key).unwrap()))
                .collect();
        }

        serde_json::to_string(&request_data)?
    } else {
        String::new()
    };

    let message = if request_data_string.is_empty() {
        endpoint.to_string()
    } else {
        format!("{}{}", endpoint, request_data_string)
    };
    let signature = wallet
        .signer
        .sign_message(message.as_bytes())
        .await?
        .as_bytes();
    let signature_string = format!("0x{}", hex::encode(signature));

    Ok(signature_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web3::wallet::Wallet;

    use serde_json::json;

    use url::Url;

    #[tokio::test]
    async fn test_sign_request() {
        // Create test wallet with known private key
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        // Test data
        let endpoint = "/api/test";
        let test_data = json!({
            "key2": "value2",
            "key1": "value1"
        });

        // Sign request
        let signature = sign_request(endpoint, &wallet, Some(&test_data))
            .await
            .unwrap();

        // Verify signature starts with "0x"
        assert!(signature.starts_with("0x"));

        // Verify signature length (0x + 130 hex chars for 65 bytes)
        assert_eq!(signature.len(), 132);
    }

    #[tokio::test]
    async fn test_sign_request_with_empty_data() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        let endpoint = "/api/test";
        let empty_data = json!({});

        let signature = sign_request(endpoint, &wallet, Some(&empty_data))
            .await
            .unwrap();
        println!("Signature: {}", signature);
        assert!(signature.starts_with("0x"));
        assert_eq!(signature.len(), 132);
    }

    #[tokio::test]
    async fn test_key_sorting() {
        let private_key = "0000000000000000000000000000000000000000000000000000000000000001";
        let wallet = Wallet::new(
            private_key,
            Url::parse("https://mainnet.infura.io/v3/9aa3d95b3bc440fa88ea12eaa4456161").unwrap(),
        )
        .unwrap();

        let endpoint = "/api/test";

        // Create two objects with same data but different key order
        let data1 = json!({
            "a": "1",
            "b": "2"
        });

        let data2 = json!({
            "b": "2",
            "a": "1"
        });

        let sig1 = sign_request(endpoint, &wallet, Some(&data1)).await.unwrap();
        let sig2 = sign_request(endpoint, &wallet, Some(&data2)).await.unwrap();

        // Signatures should be identical since keys are sorted
        assert_eq!(sig1, sig2);
    }
}
