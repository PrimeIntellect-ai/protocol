use shared::models::node::Node;
use shared::security::request_signer::sign_request_with_nonce;
use shared::web3::wallet::Wallet;

pub struct DiscoveryService {
    wallet: Wallet,
    base_url: String,
    endpoint: String,
}

impl DiscoveryService {
    pub fn new(wallet: Wallet, base_url: Option<String>, endpoint: Option<String>) -> Self {
        Self {
            wallet,
            base_url: base_url.unwrap_or_else(|| "http://localhost:8089".to_string()),
            endpoint: endpoint.unwrap_or_else(|| "/api/nodes".to_string()),
        }
    }

    pub async fn upload_discovery_info(
        &self,
        node_config: &Node,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request_data = serde_json::to_value(node_config)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

        let signed_request =
            sign_request_with_nonce(&self.endpoint, &self.wallet, Some(&request_data)).await?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-address",
            self.wallet
                .wallet
                .default_signer()
                .address()
                .to_string()
                .parse()
                .unwrap(),
        );
        headers.insert("x-signature", signed_request.signature.parse().unwrap());
        let request_url = format!("{}{}", self.base_url, &self.endpoint);

        let client = reqwest::Client::new();
        let response = client
            .put(&request_url)
            .headers(headers)
            .json(&signed_request.data.unwrap())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "No error message".to_string());
            return Err(format!(
                "Error: Received response with status code {}: {}",
                status, error_text
            )
            .into());
        }

        Ok(())
    }
}

impl Clone for DiscoveryService {
    fn clone(&self) -> Self {
        Self {
            wallet: self.wallet.clone(),
            base_url: self.base_url.clone(),
            endpoint: self.endpoint.clone(),
        }
    }
}
