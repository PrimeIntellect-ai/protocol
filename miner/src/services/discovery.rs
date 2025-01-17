use crate::console::Console;
use crate::operations::structs::node::NodeConfig;
use alloy::signers::Signer;
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

    async fn _generate_signature(
        &self,
        message: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        println!(
            "Signin with address: {:?}",
            self.wallet.wallet.default_signer().address()
        );
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
        Console::section("📦 Uploading discovery info");
        Console::info("Node config", &format!("{:?}", node_config)); // Use Console for logging

        let mut request_data = serde_json::to_value(node_config)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

        // Sort the keys of the request_data
        if let Some(obj) = request_data.as_object_mut() {
            let sorted_keys: Vec<String> = obj.keys().cloned().collect();
            let sorted_obj: serde_json::Map<String, serde_json::Value> = sorted_keys
                .into_iter()
                .map(|key| (key.clone(), obj.remove(&key).unwrap()))
                .collect();
            *obj = sorted_obj;
        }

        let request_data_string = serde_json::to_string(&request_data).unwrap();
        println!("Request data string: {}", request_data_string);

        let url = format!(
            "{}/{}",
            self.endpoint,
            self.wallet.wallet.default_signer().address()
        );
        println!("URL: {}", url);
        println!(
            "Wallet: {:?}",
            self.wallet.wallet.default_signer().address()
        );
        let message = format!("{}{}", url, request_data_string);
        println!("Message: {}", message);
        let signature_string = self._generate_signature(&message).await?;
        println!("Signature string: {}", signature_string);

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
        let request_url = format!("{}{}", self.base_url, &url);
        println!("Request URL: {}", request_url);
        println!("data: {:?}", request_data);

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

        Console::success("✓ Discovery info uploaded successfully"); // Log success message
        Ok(())
    }
}
