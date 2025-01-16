use crate::node::config::NodeConfig;
use crate::web3::wallet::Wallet;
use alloy::signers::local::LocalSigner;
use alloy::{
    network::TransactionBuilder,
    primitives::utils::keccak256 as keccak,
    primitives::{Address, U256},
    providers::Provider,
    signers::Signer,
};

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

    async fn _generate_signature(
        &self,
        message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let signature = &self
            .wallet
            .signer
            .sign_message(message.as_bytes())
            .await?
            .as_bytes();
        let signature_string = format!("0x{}", hex::encode(signature));
        Ok(signature_string)
    }

    pub async fn upload_discovery_info(
        &self,
        node_config: &NodeConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}{}", self.base_url, self.endpoint);

        let request_data = serde_json::to_value(&node_config)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
        let request_data_string = serde_json::to_string(&request_data)
            .unwrap()
            .replace("\\", "");

        let message = format!(
            "/api/nodes/{}{}",
            self.wallet.wallet.default_signer().address(),
            request_data_string
        );
        let signature_string = self._generate_signature(&message).await?;

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

        let response = reqwest::Client::new()
            .put(&url)
            .headers(headers)
            .json(&node_config)
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
