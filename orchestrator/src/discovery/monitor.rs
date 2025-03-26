use crate::models::node::NodeStatus;
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
    pub async fn fetch_nodes_from_discovery(&self) -> Result<Vec<DiscoveryNode>, Error> {
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
            .collect::<Vec<DiscoveryNode>>();

        Ok(nodes)
    }

    async fn sync_single_node_with_discovery(&self, discovery_node: &DiscoveryNode) -> Result<(), Error> {
        let node = OrchestratorNode::from(discovery_node.clone());
        match self.store_context.node_store.get_node(&node.address) {
            Some(existing_node) => {
                if discovery_node.is_validated && !discovery_node.is_provider_whitelisted {
                    self.store_context
                        .node_store
                        .update_node_status(&node.address, NodeStatus::Ejected);
                }
                // If a node is already in ejected state (and hence cannot recover) but the provider
                // gets whitelisted, we need to mark it as dead so it can actually recover again
                if discovery_node.is_validated
                    && discovery_node.is_provider_whitelisted
                    && node.status == NodeStatus::Ejected
                {
                    self.store_context
                        .node_store
                        .update_node_status(&node.address, NodeStatus::Dead);
                }
                if !discovery_node.is_active && existing_node.status == NodeStatus::Healthy {
                    // Node is active False but we have it in store and it is healthy
                    // This means that the node likely got kicked by e.g. the validator
                    // We simply remove it from the store now and will rediscover it later?
                    println!(
                        "Node {} is no longer active on chain, marking as dead",
                        node.address
                    );
                    if !discovery_node.is_provider_whitelisted {
                        self.store_context
                            .node_store
                            .update_node_status(&node.address, NodeStatus::Ejected);
                    } else {
                        self.store_context
                            .node_store
                            .update_node_status(&node.address, NodeStatus::Dead);
                    }
                }

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
        Ok(())
    }

    async fn get_nodes(&self) -> Result<Vec<OrchestratorNode>, Error> {
        let discovery_nodes = self.fetch_nodes_from_discovery().await?;

        for discovery_node in &discovery_nodes {
            if let Err(e) = self.sync_single_node_with_discovery(discovery_node).await {
                error!("Error syncing node with discovery: {}", e);
            }
        }

        Ok(discovery_nodes
            .into_iter()
            .map(OrchestratorNode::from)
            .collect())
    }
}
