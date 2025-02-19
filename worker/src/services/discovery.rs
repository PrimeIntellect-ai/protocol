use crate::console::Console;
use shared::models::node::Node;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;

pub struct DiscoveryService<'b> {
    wallet: &'b Wallet,
    base_url: String,
    endpoint: String,
}

impl<'b> DiscoveryService<'b> {
    pub fn new(wallet: &'b Wallet, base_url: Option<String>, endpoint: Option<String>) -> Self {
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
        Console::section("📦 Uploading discovery info");

        let request_data = serde_json::to_value(node_config)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

        let signature_string =
            sign_request(&self.endpoint, self.wallet, Some(&request_data)).await?;

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
        headers.insert("x-signature", signature_string.parse().unwrap());
        let request_url = format!("{}{}", self.base_url, &self.endpoint);
        println!("Request URL: {:?}", request_url);

        let response = reqwest::Client::new()
            .put(&request_url)
            .headers(headers)
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(format!(
                "Error: Received response with status code {}",
                response.status()
            )
            .into());
        }

        Ok(())
    }
}
