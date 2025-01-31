use crate::models::node::OrchestratorNode;
use crate::store::core::StoreContext;
use anyhow::Error;
use anyhow::Result;
use log::{error, info};
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

    pub async fn run(&self) -> Result<(), Error> {
        let mut interval = interval(Duration::from_secs(self.interval_s));

        loop {
            interval.tick().await;
            match self.get_nodes().await {
                Ok(nodes) => {
                    info!(
                        "Successfully synced {} nodes from discovery service",
                        nodes.len()
                    );
                }
                Err(e) => {
                    error!("Error syncing nodes from discovery service: {}", e);
                }
            }
        }
    }

    pub async fn fetch_nodes_from_discovery(&self) -> Result<Vec<OrchestratorNode>, Error> {
        let discovery_route = format!("/api/pool/{}", self.compute_pool_id);
        let address = self.coordinator_wallet.address().to_string();

        let signature = match sign_request(&discovery_route, self.coordinator_wallet, None).await {
            Ok(sig) => sig,
            Err(e) => {
                error!("Failed to sign discovery request: {}", e);
                return Ok(Vec::new());
            }
        };

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "x-address",
            reqwest::header::HeaderValue::from_str(&address).unwrap(),
        );
        headers.insert(
            "x-signature",
            reqwest::header::HeaderValue::from_str(&signature).unwrap(),
        );

        let response = match reqwest::Client::new()
            .get(format!("{}{}", self.discovery_url, discovery_route))
            .headers(headers)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                error!("Failed to fetch nodes from discovery service: {}", e);
                return Ok(Vec::new());
            }
        };

        let response_text = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                error!("Failed to read discovery response: {}", e);
                return Ok(Vec::new());
            }
        };

        let parsed_response: ApiResponse<Vec<DiscoveryNode>> =
            match serde_json::from_str(&response_text) {
                Ok(resp) => resp,
                Err(e) => {
                    error!("Failed to parse discovery response: {}", e);
                    return Ok(Vec::new());
                }
            };

        let nodes = parsed_response.data;
        let nodes = nodes
            .into_iter()
            .filter(|node| node.is_validated)
            .map(OrchestratorNode::from)
            .collect::<Vec<OrchestratorNode>>();

        Ok(nodes)
    }

    pub async fn get_nodes(&self) -> Result<Vec<OrchestratorNode>, Error> {
        let nodes = self.fetch_nodes_from_discovery().await?;

        for node in &nodes {
            match self.store_context.node_store.get_node(&node.address) {
                Some(existing_node) => {
                    if existing_node.ip_address != node.ip_address {
                        info!(
                            "Node {} IP changed from {} to {}",
                            node.address, existing_node.ip_address, node.ip_address
                        );
                        self.store_context.node_store.add_node(node.clone());
                    }
                }
                None => {
                    info!("Discovered new validated node: {}", node.address);
                    self.store_context.node_store.add_node(node.clone());
                }
            }
        }

        Ok(nodes)
    }
}
