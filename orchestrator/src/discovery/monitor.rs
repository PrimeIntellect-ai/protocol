use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use alloy::primitives::Address;
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
    heartbeats: Arc<LoopHeartbeats>,
}

impl<'b> DiscoveryMonitor<'b> {
    pub fn new(
        coordinator_wallet: &'b Wallet,
        compute_pool_id: u32,
        interval_s: u64,
        discovery_url: String,
        store_context: Arc<StoreContext>,
        heartbeats: Arc<LoopHeartbeats>,
    ) -> Self {
        Self {
            coordinator_wallet,
            compute_pool_id,
            interval_s,
            discovery_url,
            store_context,
            heartbeats,
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
            self.heartbeats.update_monitor();
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
            reqwest::header::HeaderValue::from_str(&address)?,
        );
        headers.insert(
            "x-signature",
            reqwest::header::HeaderValue::from_str(&signature)?,
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

    async fn sync_single_node_with_discovery(
        &self,
        discovery_node: &DiscoveryNode,
    ) -> Result<(), Error> {
        let node_address = discovery_node.node.id.parse::<Address>()?;
        match self.store_context.node_store.get_node(&node_address) {
            Some(existing_node) => {
                if discovery_node.is_validated && !discovery_node.is_provider_whitelisted {
                    info!(
                        "Node {} is validated but not provider whitelisted, marking as ejected",
                        node_address
                    );
                    self.store_context
                        .node_store
                        .update_node_status(&node_address, NodeStatus::Ejected);
                }

                // If a node is already in ejected state (and hence cannot recover) but the provider
                // gets whitelisted, we need to mark it as dead so it can actually recover again
                if discovery_node.is_validated
                    && discovery_node.is_provider_whitelisted
                    && existing_node.status == NodeStatus::Ejected
                {
                    info!(
                        "Node {} is validated and provider whitelisted. Local store status was ejected, marking as dead so node can recover",
                        node_address
                    );
                    self.store_context
                        .node_store
                        .update_node_status(&node_address, NodeStatus::Dead);
                }
                if !discovery_node.is_active && existing_node.status == NodeStatus::Healthy {
                    // Node is active False but we have it in store and it is healthy
                    // This means that the node likely got kicked by e.g. the validator
                    // We simply remove it from the store now and will rediscover it later?
                    println!(
                        "Node {} is no longer active on chain, marking as dead",
                        node_address
                    );
                    if !discovery_node.is_provider_whitelisted {
                        self.store_context
                            .node_store
                            .update_node_status(&node_address, NodeStatus::Ejected);
                    } else {
                        self.store_context
                            .node_store
                            .update_node_status(&node_address, NodeStatus::Dead);
                    }
                }

                if existing_node.ip_address != discovery_node.node.ip_address {
                    info!(
                        "Node {} IP changed from {} to {}",
                        node_address, existing_node.ip_address, discovery_node.node.ip_address
                    );
                    let mut node = existing_node.clone();
                    node.ip_address = discovery_node.node.ip_address.clone();
                    self.store_context.node_store.add_node(node.clone());
                }

                if existing_node.status == NodeStatus::Dead {
                    if let (Some(last_change), Some(last_updated)) = (
                        existing_node.last_status_change,
                        discovery_node.last_updated,
                    ) {
                        if last_change < last_updated {
                            info!("Node {} is dead but has been updated on discovery, marking as discovered", node_address);
                            self.store_context
                                .node_store
                                .update_node_status(&node_address, NodeStatus::Discovered);
                        }
                    }
                }
            }
            None => {
                info!("Discovered new validated node: {}", node_address);
                let node = OrchestratorNode::from(discovery_node.clone());
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
            None => {
                info!("Discovered new validated node: {}", node_address);
                let node = OrchestratorNode::from(discovery_node.clone());
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

#[cfg(test)]
mod tests {
    use alloy::primitives::Address;
    use shared::models::node::Node;
    use url::Url;

    use super::*;
    use crate::models::node::NodeStatus;
    use crate::store::core::{RedisStore, StoreContext};

    #[tokio::test]
    async fn test_sync_single_node_with_discovery() {
        let node_address = "0x1234567890123456789012345678901234567890";
        let discovery_node = DiscoveryNode {
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: false,
            node: Node {
                id: node_address.to_string(),
                provider_address: node_address.to_string(),
                ip_address: "127.0.0.1".to_string(),
                port: 8080,
                compute_pool_id: 1,
                compute_specs: None,
            },
            is_blacklisted: false,
            last_updated: None,
            created_at: None,
        };

        let mut orchestrator_node = OrchestratorNode::from(discovery_node.clone());
        orchestrator_node.status = NodeStatus::Ejected;
        orchestrator_node.address = discovery_node.node.id.parse::<Address>().unwrap();

        let store = Arc::new(RedisStore::new_test());
        let mut con = store
            .client
            .get_connection()
            .expect("Should connect to test Redis instance");

        redis::cmd("PING")
            .query::<String>(&mut con)
            .expect("Redis should be responsive");
        redis::cmd("FLUSHALL")
            .query::<String>(&mut con)
            .expect("Redis should be flushed");

        let store_context = Arc::new(StoreContext::new(store.clone()));
        let discovery_store_context = store_context.clone();

        store_context.node_store.add_node(orchestrator_node.clone());

        let fake_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap();

        let discovery_monitor = DiscoveryMonitor::new(
            &fake_wallet,
            1,
            10,
            "http://localhost:8080".to_string(),
            discovery_store_context,
            Arc::new(LoopHeartbeats::new()),
        );

        let store_context_clone = store_context.clone();

        let node_from_store = store_context_clone
            .node_store
            .get_node(&orchestrator_node.address)
            .unwrap();
        assert_eq!(node_from_store.status, NodeStatus::Ejected);

        discovery_monitor
            .sync_single_node_with_discovery(&discovery_node)
            .await
            .unwrap();

        let node_after_sync = &store_context
            .node_store
            .get_node(&orchestrator_node.address)
            .unwrap();
        assert_eq!(node_after_sync.status, NodeStatus::Dead);
    }
}
