use anyhow::Result;
use shared::models::node::Node;
use shared::web3::wallet::Wallet;

pub(crate) struct DiscoveryService {
    wallet: Wallet,
    base_urls: Vec<String>,
}

impl DiscoveryService {
    pub(crate) fn new(wallet: Wallet, base_urls: Vec<String>, _endpoint: Option<String>) -> Self {
        let urls = if base_urls.is_empty() {
            vec!["http://localhost:8089".to_string()]
        } else {
            base_urls
        };
        Self {
            wallet,
            base_urls: urls,
        }
    }

    pub(crate) async fn upload_discovery_info(&self, node_config: &Node) -> Result<()> {
        let node_data = serde_json::to_value(node_config)?;

        shared::discovery::upload_node_to_discovery(&self.base_urls, &node_data, &self.wallet).await
    }
}

impl Clone for DiscoveryService {
    fn clone(&self) -> Self {
        Self {
            wallet: self.wallet.clone(),
            base_urls: self.base_urls.clone(),
        }
    }
}
