use anyhow::Result;
use shared::models::node::Node;
use shared::security::request_signer::sign_request_with_nonce;
use shared::web3::wallet::Wallet;

pub(crate) struct DiscoveryService {
    wallet: Wallet,
    base_urls: Vec<String>,
    endpoint: String,
}

impl DiscoveryService {
    pub(crate) fn new(wallet: Wallet, base_urls: Vec<String>, endpoint: Option<String>) -> Self {
        let urls = if base_urls.is_empty() {
            vec!["http://localhost:8089".to_string()]
        } else {
            base_urls
        };
        Self {
            wallet,
            base_urls: urls,
            endpoint: endpoint.unwrap_or_else(|| "/api/nodes".to_string()),
        }
    }

    async fn upload_to_single_discovery(&self, node_config: &Node, base_url: &str) -> Result<()> {
        let request_data = serde_json::to_value(node_config)?;

        let signed_request =
            sign_request_with_nonce(&self.endpoint, &self.wallet, Some(&request_data))
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;

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
        let request_url = format!("{}{}", base_url, &self.endpoint);
        let client = reqwest::Client::new();
        let response = client
            .put(&request_url)
            .headers(headers)
            .json(
                &signed_request
                    .data
                    .expect("Signed request data should always be present for discovery upload"),
            )
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "No error message".to_string());
            return Err(anyhow::anyhow!(
                "Error: Received response with status code {} from {}: {}",
                status,
                base_url,
                error_text
            ));
        }

        Ok(())
    }

    pub(crate) async fn upload_discovery_info(&self, node_config: &Node) -> Result<()> {
        let mut last_error: Option<String> = None;

        for base_url in &self.base_urls {
            match self.upload_to_single_discovery(node_config, base_url).await {
                Ok(_) => {
                    // Successfully uploaded to one discovery service, return immediately
                    return Ok(());
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                }
            }
        }

        // If we reach here, all discovery services failed
        if let Some(error) = last_error {
            Err(anyhow::anyhow!(
                "Failed to upload to all discovery services. Last error: {}",
                error
            ))
        } else {
            Err(anyhow::anyhow!(
                "Failed to upload to all discovery services"
            ))
        }
    }
}

impl Clone for DiscoveryService {
    fn clone(&self) -> Self {
        Self {
            wallet: self.wallet.clone(),
            base_urls: self.base_urls.clone(),
            endpoint: self.endpoint.clone(),
        }
    }
}
