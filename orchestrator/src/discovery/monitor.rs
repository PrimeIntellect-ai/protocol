use crate::models::node::Node;
use crate::store::core::StoreContext;
use anyhow::Error;
use anyhow::Result;
use log::{debug, error, info};
use reqwest::Response;
use serde_json;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
pub struct DiscoveryMonitor<'b> {
    coordinator_wallet: &'b Wallet,
    compute_pool_id: u32,
    interval_s: u64,
    discovery_url: String,
    store_context: Arc<StoreContext>,
}

impl<'b> DiscoveryMonitor<'b> {
    pub fn new(
        coordinator_wallet: &'b Wallet,
        compute_pool_id: u32,
        interval_s: u64,
        discovery_url: String,
        store_context: Arc<StoreContext>,
    ) -> Self {
        Self {
            coordinator_wallet,
            compute_pool_id,
            interval_s,
            discovery_url,
            store_context,
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(self.interval_s));

        loop {
            interval.tick().await;
            debug!("Running DiscoveryMonitor to get nodes...");
            if let Err(e) = self.get_nodes().await {
                error!("Error getting nodes from discovery: {}", e);
            }
        }
    }

    pub async fn fetch_nodes_from_discovery(&self) -> Result<Vec<Node>, Error> {
        let discovery_route = format!("/api/pool/{}", self.compute_pool_id);
        let address = self.coordinator_wallet.address().to_string();

        let signature = sign_request(&discovery_route, self.coordinator_wallet, None)
            .await
            .unwrap();

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-address", address.parse().unwrap());
        headers.insert("x-signature", signature.parse().unwrap());

        let response: Response = reqwest::Client::new()
            .get(format!("{}{}", self.discovery_url, discovery_route))
            .headers(headers)
            .send()
            .await?;

        let response_text = response.text().await?;
        let parsed_response: ApiResponse<Vec<DiscoveryNode>> =
            serde_json::from_str(&response_text)?;
        let nodes = parsed_response.data;

        let nodes = nodes
            .into_iter()
            .filter(|node| node.is_validated) 
            .map(Node::from)
            .collect::<Vec<Node>>();
        Ok(nodes)
    }

    pub async fn get_nodes(&self) -> Result<Vec<Node>, Error> {
        let nodes = self.fetch_nodes_from_discovery().await?;
        for node in &nodes {
            println!("Node: {:?}", node.address);
            let exists = self.store_context.node_store.get_node(&node.address);
            if exists.is_some() {
                continue;
            }
            info!("Discovered new, validated node: {:?}", node.address);
            self.store_context.node_store.add_node(node.clone());
        }
        Ok(nodes)
    }
}
