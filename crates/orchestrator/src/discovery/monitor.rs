use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use alloy::primitives::Address;
use anyhow::Error;
use anyhow::Result;
use chrono::Utc;
use log::{error, info};
use serde_json;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct DiscoveryMonitor {
    coordinator_wallet: Wallet,
    compute_pool_id: u32,
    interval_s: u64,
    discovery_url: String,
    store_context: Arc<StoreContext>,
    heartbeats: Arc<LoopHeartbeats>,
    http_client: reqwest::Client,
}

impl DiscoveryMonitor {
    pub fn new(
        coordinator_wallet: Wallet,
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
            http_client: reqwest::Client::new(),
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

        let signature = match sign_request(&discovery_route, &self.coordinator_wallet, None).await {
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

        let response = match self
            .http_client
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

    async fn has_healthy_node_with_same_endpoint(
        &self,
        node_address: Address,
        ip_address: &str,
        port: u16,
    ) -> Result<bool, Error> {
        let nodes = self.store_context.node_store.get_nodes().await?;
        Ok(nodes.iter().any(|other_node| {
            other_node.address != node_address
                && other_node.ip_address == ip_address
                && other_node.port == port
                && other_node.status == NodeStatus::Healthy
        }))
    }

    async fn sync_single_node_with_discovery(
        &self,
        discovery_node: &DiscoveryNode,
    ) -> Result<(), Error> {
        let node_address = discovery_node.node.id.parse::<Address>()?;

        // Check if there's any healthy node with the same IP and port
        let has_healthy_node_same_endpoint = self
            .has_healthy_node_with_same_endpoint(
                node_address,
                &discovery_node.node.ip_address,
                discovery_node.node.port,
            )
            .await?;

        match self.store_context.node_store.get_node(&node_address).await {
            Ok(Some(existing_node)) => {
                // If there's a healthy node with same IP and port, and this node isn't healthy, mark it dead
                if has_healthy_node_same_endpoint && existing_node.status != NodeStatus::Healthy {
                    info!(
                        "Node {} shares endpoint {}:{} with a healthy node, marking as dead",
                        node_address, discovery_node.node.ip_address, discovery_node.node.port
                    );
                    if let Err(e) = self
                        .store_context
                        .node_store
                        .update_node_status(&node_address, NodeStatus::Dead)
                        .await
                    {
                        error!("Error updating node status: {}", e);
                    }
                    return Ok(());
                }

                if discovery_node.is_validated && !discovery_node.is_provider_whitelisted {
                    info!(
                        "Node {} is validated but not provider whitelisted, marking as ejected",
                        node_address
                    );
                    if let Err(e) = self
                        .store_context
                        .node_store
                        .update_node_status(&node_address, NodeStatus::Ejected)
                        .await
                    {
                        error!("Error updating node status: {}", e);
                    }
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
                    if let Err(e) = self
                        .store_context
                        .node_store
                        .update_node_status(&node_address, NodeStatus::Dead)
                        .await
                    {
                        error!("Error updating node status: {}", e);
                    }
                }
                if !discovery_node.is_active && existing_node.status == NodeStatus::Healthy {
                    // Node is active False but we have it in store and it is healthy
                    // This means that the node likely got kicked by e.g. the validator
                    // We simply remove it from the store now and will rediscover it later?
                    info!(
                        "Node {} is no longer active on chain, marking as ejected",
                        node_address
                    );
                    if !discovery_node.is_provider_whitelisted {
                        if let Err(e) = self
                            .store_context
                            .node_store
                            .update_node_status(&node_address, NodeStatus::Ejected)
                            .await
                        {
                            error!("Error updating node status: {}", e);
                        }
                    } else if let Err(e) = self
                        .store_context
                        .node_store
                        .update_node_status(&node_address, NodeStatus::Dead)
                        .await
                    {
                        error!("Error updating node status: {}", e);
                    }
                }

                if existing_node.ip_address != discovery_node.node.ip_address {
                    info!(
                        "Node {} IP changed from {} to {}",
                        node_address, existing_node.ip_address, discovery_node.node.ip_address
                    );
                    let mut node = existing_node.clone();
                    node.ip_address = discovery_node.node.ip_address.clone();
                    let _ = self.store_context.node_store.add_node(node.clone()).await;
                }

                if existing_node.status == NodeStatus::Dead {
                    if let (Some(last_change), Some(last_updated)) = (
                        existing_node.last_status_change,
                        discovery_node.last_updated,
                    ) {
                        if last_change < last_updated {
                            info!("Node {} is dead but has been updated on discovery, marking as discovered", node_address);
                            if let Err(e) = self
                                .store_context
                                .node_store
                                .update_node_status(&node_address, NodeStatus::Discovered)
                                .await
                            {
                                error!("Error updating node status: {}", e);
                            }
                        }
                    }
                }
            }
            Ok(None) => {
                // Don't add new node if there's already a healthy node with same IP and port
                if has_healthy_node_same_endpoint {
                    info!(
                        "Skipping new node {} as endpoint {}:{} is already used by a healthy node",
                        node_address, discovery_node.node.ip_address, discovery_node.node.port
                    );
                    return Ok(());
                }

                info!("Discovered new validated node: {}", node_address);
                let mut node = OrchestratorNode::from(discovery_node.clone());
                node.first_seen = Some(Utc::now());
                let _ = self.store_context.node_store.add_node(node.clone()).await;
            }
            Err(e) => {
                error!("Error syncing node with discovery: {}", e);
                return Err(e);
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
    use shared::models::node::{ComputeSpecs, Node};
    use url::Url;

    use super::*;
    use crate::models::node::NodeStatus;
    use crate::store::core::{RedisStore, StoreContext};
    use crate::ServerMode;

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
                compute_specs: Some(ComputeSpecs {
                    ram_mb: Some(1024),
                    storage_gb: Some(10),
                    ..Default::default()
                }),
                ..Default::default()
            },
            is_blacklisted: false,
            ..Default::default()
        };

        let mut orchestrator_node = OrchestratorNode::from(discovery_node.clone());
        orchestrator_node.status = NodeStatus::Ejected;
        orchestrator_node.address = discovery_node.node.id.parse::<Address>().unwrap();
        orchestrator_node.first_seen = Some(Utc::now());
        orchestrator_node.compute_specs = Some(ComputeSpecs {
            gpu: None,
            cpu: None,
            ram_mb: Some(1024),
            storage_gb: Some(10),
            ..Default::default()
        });
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

        let _ = store_context
            .node_store
            .add_node(orchestrator_node.clone())
            .await;

        let fake_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap();

        let mode = ServerMode::Full;

        let discovery_monitor = DiscoveryMonitor::new(
            fake_wallet,
            1,
            10,
            "http://localhost:8080".to_string(),
            discovery_store_context,
            Arc::new(LoopHeartbeats::new(&mode)),
        );

        let store_context_clone = store_context.clone();

        let node_from_store = store_context_clone
            .node_store
            .get_node(&orchestrator_node.address)
            .await
            .unwrap();
        assert!(node_from_store.is_some());
        if let Some(node) = node_from_store {
            assert_eq!(node.status, NodeStatus::Ejected);
        }

        discovery_monitor
            .sync_single_node_with_discovery(&discovery_node)
            .await
            .unwrap();

        let node_after_sync = &store_context
            .node_store
            .get_node(&orchestrator_node.address)
            .await
            .unwrap();
        assert!(node_after_sync.is_some());
        if let Some(node) = node_after_sync {
            assert_eq!(node.status, NodeStatus::Dead);
        }
    }

    #[tokio::test]
    async fn test_first_seen_timestamp_set_on_new_node() {
        let node_address = "0x2234567890123456789012345678901234567890";
        let discovery_node = DiscoveryNode {
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: true,
            node: Node {
                id: node_address.to_string(),
                provider_address: node_address.to_string(),
                ip_address: "192.168.1.100".to_string(),
                port: 8080,
                compute_pool_id: 1,
                ..Default::default()
            },
            is_blacklisted: false,
            ..Default::default()
        };

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

        let fake_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap();

        let mode = ServerMode::Full;

        let discovery_monitor = DiscoveryMonitor::new(
            fake_wallet,
            1,
            10,
            "http://localhost:8080".to_string(),
            store_context.clone(),
            Arc::new(LoopHeartbeats::new(&mode)),
        );

        let time_before = Utc::now();

        // Sync a new node that doesn't exist in the store
        discovery_monitor
            .sync_single_node_with_discovery(&discovery_node)
            .await
            .unwrap();

        let time_after = Utc::now();

        // Verify the node was added with first_seen timestamp
        let node_from_store = store_context
            .node_store
            .get_node(&discovery_node.node.id.parse::<Address>().unwrap())
            .await
            .unwrap();

        assert!(node_from_store.is_some());
        let node = node_from_store.unwrap();

        // Verify first_seen is set
        assert!(node.first_seen.is_some());
        let first_seen = node.first_seen.unwrap();

        // Verify the timestamp is within the expected range
        assert!(first_seen >= time_before && first_seen <= time_after);

        // Verify other fields are set correctly
        assert_eq!(node.status, NodeStatus::Discovered);
        assert_eq!(node.ip_address, "192.168.1.100");

        // Test case: Sync the same node again to verify first_seen is preserved
        // Simulate some time passing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Update discovery data to simulate a change (e.g., IP address change)
        let updated_discovery_node = DiscoveryNode {
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: true,
            node: Node {
                id: node_address.to_string(),
                provider_address: node_address.to_string(),
                ip_address: "192.168.1.101".to_string(), // Changed IP
                port: 8080,
                compute_pool_id: 1,
                ..Default::default()
            },
            is_blacklisted: false,
            ..Default::default()
        };

        // Sync the node again
        discovery_monitor
            .sync_single_node_with_discovery(&updated_discovery_node)
            .await
            .unwrap();

        // Verify the node was updated but first_seen is preserved
        let node_after_resync = store_context
            .node_store
            .get_node(&discovery_node.node.id.parse::<Address>().unwrap())
            .await
            .unwrap()
            .unwrap();

        // Verify first_seen is still the same (preserved)
        assert_eq!(node_after_resync.first_seen, Some(first_seen));

        // Verify IP was updated
        assert_eq!(node_after_resync.ip_address, "192.168.1.101");

        // Status should remain the same
        assert_eq!(node_after_resync.status, NodeStatus::Discovered);
    }

    #[tokio::test]
    async fn test_sync_node_with_same_endpoint() {
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

        // Create first node (will be healthy)
        let node1_address = "0x1234567890123456789012345678901234567890";
        let node1 = DiscoveryNode {
            is_validated: true,
            is_provider_whitelisted: true,
            is_active: true,
            node: Node {
                id: node1_address.to_string(),
                provider_address: node1_address.to_string(),
                ip_address: "127.0.0.1".to_string(),
                port: 8080,
                compute_pool_id: 1,
                compute_specs: Some(ComputeSpecs {
                    ram_mb: Some(1024),
                    storage_gb: Some(10),
                    ..Default::default()
                }),
                ..Default::default()
            },
            is_blacklisted: false,
            ..Default::default()
        };

        let mut orchestrator_node1 = OrchestratorNode::from(node1.clone());
        orchestrator_node1.status = NodeStatus::Healthy;
        orchestrator_node1.address = node1.node.id.parse::<Address>().unwrap();

        let _ = store_context
            .node_store
            .add_node(orchestrator_node1.clone())
            .await;

        // Create second node with same IP and port
        let node2_address = "0x2234567890123456789012345678901234567890";
        let mut node2 = node1.clone();
        node2.node.id = node2_address.to_string();
        node2.node.provider_address = node2_address.to_string();

        let fake_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .unwrap();

        let mode = ServerMode::Full;
        let discovery_monitor = DiscoveryMonitor::new(
            fake_wallet,
            1,
            10,
            "http://localhost:8080".to_string(),
            store_context.clone(),
            Arc::new(LoopHeartbeats::new(&mode)),
        );

        // Try to sync the second node
        discovery_monitor
            .sync_single_node_with_discovery(&node2)
            .await
            .unwrap();

        // Verify second node was not added
        let node2_result = store_context
            .node_store
            .get_node(&node2_address.parse::<Address>().unwrap())
            .await
            .unwrap();
        assert!(
            node2_result.is_none(),
            "Node with same endpoint should not be added"
        );

        // Create third node with same IP but different port (should be allowed)
        let node3_address = "0x3234567890123456789012345678901234567890";
        let mut node3 = node1.clone();
        node3.node.id = node3_address.to_string();
        node3.node.provider_address = node3_address.to_string();
        node3.node.port = 8081; // Different port

        // Try to sync the third node
        discovery_monitor
            .sync_single_node_with_discovery(&node3)
            .await
            .unwrap();

        // Verify third node was added (different port)
        let node3_result = store_context
            .node_store
            .get_node(&node3_address.parse::<Address>().unwrap())
            .await
            .unwrap();
        assert!(
            node3_result.is_some(),
            "Node with different port should be added"
        );
    }
}
