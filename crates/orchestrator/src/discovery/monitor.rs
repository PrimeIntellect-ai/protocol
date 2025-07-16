use crate::models::node::NodeStatus;
use crate::models::node::OrchestratorNode;
use crate::plugins::StatusUpdatePlugin;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use alloy::primitives::Address;
use alloy::primitives::U256;
use anyhow::{bail, Context as _, Error, Result};
use chrono::Utc;
use log::{error, info, warn};
use shared::models::node::NodeWithMetadata;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Sender;
use tokio::time::interval;

struct NodeFetcher {
    compute_pool_id: u32,
    kademlia_action_tx: Sender<p2p::KademliaActionWithChannel>,
    provider: alloy::providers::RootProvider,
    contracts: shared::web3::Contracts<alloy::providers::RootProvider>,
}

impl NodeFetcher {
    async fn get_nodes(&self) -> Result<Vec<NodeWithMetadata>> {
        use futures::stream::FuturesUnordered;
        use futures::StreamExt as _;

        // TODO: this actually needs to fetch for compute pool ID only (`self.compute_pool_id`)
        let nodes = get_worker_nodes_from_dht(self.kademlia_action_tx.clone())
            .await
            .context("failed to get worker nodes from DHT")?;
        if nodes.is_empty() {
            return Ok(vec![]);
        }

        // remove duplicates based on node ID
        let mut unique_nodes = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        for node in nodes {
            if seen_ids.insert(node.id.clone()) && node.compute_pool_id == self.compute_pool_id {
                unique_nodes.push(node);
            }
        }

        info!(
            "total unique nodes after deduplication: {}",
            unique_nodes.len()
        );

        let futures = FuturesUnordered::new();
        for node in unique_nodes {
            futures.push(NodeWithMetadata::new_from_contracts(
                node,
                &self.provider,
                &self.contracts,
            ));
        }
        let nodes = futures
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect::<Vec<NodeWithMetadata>>();
        if nodes.is_empty() {
            return Ok(vec![]);
        }

        // for node in &nodes {
        //     if let Err(e) = self.perform_node_updates(node).await {
        //         error!(
        //             "failed to perform update for node with id {}: {e}",
        //             node.node().id
        //         );
        //     }
        // }

        Ok(nodes)
    }
}
struct Updater {
    store_context: Arc<StoreContext>,
    status_change_handlers: Vec<StatusUpdatePlugin>,
}

impl Updater {
    async fn handle_status_change(&self, node: &OrchestratorNode, old_status: NodeStatus) {
        for handler in &self.status_change_handlers {
            if let Err(e) = handler.handle_status_change(node, &old_status).await {
                error!("Status change handler failed: {e}");
            }
        }
    }

    async fn update_node_status(
        &self,
        node_address: &Address,
        new_status: NodeStatus,
    ) -> Result<(), Error> {
        // Get the current node to know the old status
        let old_status = match self.store_context.node_store.get_node(node_address).await? {
            Some(node) => node.status,
            None => bail!("node not found: {}", node_address),
        };

        // Update the status in the store
        self.store_context
            .node_store
            .update_node_status(node_address, new_status.clone())
            .await?;

        // Get the updated node and trigger status change handlers
        if let Some(updated_node) = self.store_context.node_store.get_node(node_address).await? {
            self.handle_status_change(&updated_node, old_status).await;
        }

        Ok(())
    }

    async fn count_healthy_nodes_with_same_peer_id(
        &self,
        node_address: Address,
        peer_id: &p2p::PeerId,
    ) -> Result<u32> {
        let nodes = self.store_context.node_store.get_nodes().await?;
        Ok(nodes
            .iter()
            .filter(|other_node| {
                other_node.address != node_address
                    && other_node.p2p_id == peer_id.to_string()
                    && other_node.status == NodeStatus::Healthy
            })
            .count() as u32)
    }

    async fn perform_node_updates(&self, node: &NodeWithMetadata) -> Result<()> {
        let node_address = node.node().id.parse::<Address>()?;

        // Check if there's any healthy node with the same peer ID
        // TODO: can this case still happen? i think so if there's stale provider records in the dht
        let healthy_nodes_with_same_peer_id = self
            .count_healthy_nodes_with_same_peer_id(
                node_address,
                &node.node().worker_p2p_id.parse::<p2p::PeerId>()?,
            )
            .await
            .context("failed to count healthy nodes with same peer ID")?;

        match self.store_context.node_store.get_node(&node_address).await {
            Ok(Some(existing_node)) => {
                // If there's a healthy node with same IP and port, and this node isn't healthy, mark it dead
                if healthy_nodes_with_same_peer_id > 0
                    && existing_node.status != NodeStatus::Healthy
                {
                    info!(
                        "Node {} shares peer ID {} with a healthy node, marking as dead",
                        node_address,
                        node.node().worker_p2p_id
                    );
                    self.update_node_status(&node_address, NodeStatus::Dead)
                        .await
                        .context("failed to update node status to Dead")?;
                    return Ok(());
                }

                if node.is_validated() && !node.is_provider_whitelisted() {
                    info!(
                        "Node {node_address} is validated but not provider whitelisted, marking as ejected"
                    );
                    self.update_node_status(&node_address, NodeStatus::Ejected)
                        .await
                        .context("failed to update node status to Ejected")?;
                }

                // If a node is already in ejected state (and hence cannot recover) but the provider
                // gets whitelisted, we need to mark it as dead so it can actually recover again
                if node.is_validated()
                    && node.is_provider_whitelisted()
                    && existing_node.status == NodeStatus::Ejected
                {
                    info!(
                        "Node {node_address} is validated and provider whitelisted. Local store status was ejected, marking as dead so node can recover"
                    );
                    self.update_node_status(&node_address, NodeStatus::Dead)
                        .await
                        .context("failed to update node status to Dead")?;
                }

                if !node.is_active() && existing_node.status == NodeStatus::Healthy {
                    // Node is active False but we have it in store and it is healthy
                    // This means that the node likely got kicked by e.g. the validator
                    // Add a grace period check to avoid immediately marking nodes that just became healthy
                    let should_mark_inactive =
                        if let Some(last_status_change) = existing_node.last_status_change {
                            let grace_period = chrono::Duration::minutes(5); // 5 minute grace period
                            let now = chrono::Utc::now();
                            now.signed_duration_since(last_status_change) > grace_period
                        } else {
                            // If no last_status_change, assume it's been healthy for a while
                            true
                        };

                    if should_mark_inactive {
                        info!(
                            "Node {node_address} is no longer active on chain, marking as ejected"
                        );
                        if !node.is_provider_whitelisted() {
                            self.update_node_status(&node_address, NodeStatus::Ejected)
                                .await
                                .context("failed to update node status to Ejected")?;
                        } else {
                            self.update_node_status(&node_address, NodeStatus::Dead)
                                .await
                                .context("failed to update node status to Dead")?;
                        }
                    } else {
                        info!(
                            "Node {node_address} is no longer active on chain but recently became healthy, waiting before marking inactive"
                        );
                    }
                }

                if existing_node.ip_address != node.node().ip_address {
                    info!(
                        "Node {} IP changed from {} to {}",
                        node_address,
                        existing_node.ip_address,
                        node.node().ip_address
                    );
                    let mut existing_node = existing_node.clone();
                    existing_node.ip_address = node.node().ip_address.clone();
                    self.store_context
                        .node_store
                        .add_node(existing_node)
                        .await
                        .context("failed to update node IP address")?;
                }

                if existing_node.location.is_none() && node.location().is_some() {
                    info!(
                        "Node {} location changed from None to {:?}",
                        node_address,
                        node.location()
                    );
                    if let Some(location) = node.location() {
                        self.store_context
                            .node_store
                            .update_node_location(&node_address, location)
                            .await
                            .context("failed to update node location")?;
                    }
                }

                if existing_node.status == NodeStatus::Dead {
                    if let (Some(last_change), Some(last_updated)) =
                        (existing_node.last_status_change, node.last_updated())
                    {
                        if last_change < last_updated {
                            info!("Node {node_address} is dead but has been updated on discovery, marking as discovered");

                            if existing_node.compute_specs != node.node().compute_specs {
                                info!(
                                    "Node {node_address} compute specs changed, marking as discovered"
                                );
                                let mut node = existing_node.clone();
                                node.compute_specs = node.compute_specs.clone();
                                self.store_context
                                    .node_store
                                    .add_node(node.clone())
                                    .await
                                    .context("failed to update node compute specs")?;
                            }
                            self.update_node_status(&node_address, NodeStatus::Discovered)
                                .await
                                .context("failed to update node status to Discovered")?;
                        }
                    }
                }

                if node.latest_balance() == U256::ZERO {
                    info!("Node {node_address} has zero balance, marking as low balance");
                    self.update_node_status(&node_address, NodeStatus::LowBalance)
                        .await
                        .context("failed to update node status to LowBalance")?;
                }
            }
            Ok(None) => {
                // Don't add new node if there's already a healthy node with same peer ID
                if healthy_nodes_with_same_peer_id > 0 {
                    info!(
                        "Skipping new node {} as peer ID {} is already used by a healthy node",
                        node_address,
                        node.node().worker_p2p_id
                    );
                    return Ok(());
                }

                info!("Discovered new validated node: {node_address}");
                let mut node = OrchestratorNode::from(node);
                node.first_seen = Some(Utc::now());
                self.store_context
                    .node_store
                    .add_node(node)
                    .await
                    .context("failed to add node to store")?;
            }
            Err(e) => {
                return Err(e.context("failed to get node from store"));
            }
        }
        Ok(())
    }
}

pub struct DiscoveryMonitor {
    interval_s: u64,
    heartbeats: Arc<LoopHeartbeats>,
    updater: Updater,
    node_fetcher: NodeFetcher,
}

impl DiscoveryMonitor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        compute_pool_id: u32,
        interval_s: u64,
        store_context: Arc<StoreContext>,
        heartbeats: Arc<LoopHeartbeats>,
        status_change_handlers: Vec<StatusUpdatePlugin>,
        kademlia_action_tx: Sender<p2p::KademliaActionWithChannel>,
        provider: alloy::providers::RootProvider,
        contracts: shared::web3::Contracts<alloy::providers::RootProvider>,
    ) -> Self {
        Self {
            interval_s,
            heartbeats,
            updater: Updater {
                store_context,
                status_change_handlers,
            },
            node_fetcher: NodeFetcher {
                compute_pool_id,
                kademlia_action_tx,
                provider,
                contracts,
            },
        }
    }

    pub async fn run(self) {
        let Self {
            interval_s,
            heartbeats,
            updater,
            node_fetcher,
        } = self;

        let mut interval = interval(Duration::from_secs(interval_s));

        loop {
            interval.tick().await;
            match node_fetcher.get_nodes().await {
                Ok(nodes) => {
                    for node in &nodes {
                        if let Err(e) = updater.perform_node_updates(node).await {
                            error!(
                                "failed to perform update for node with id {}: {e}",
                                node.node().id
                            );
                        }
                    }

                    info!("Successfully synced {} nodes from discovery", nodes.len());
                }
                Err(e) => {
                    error!("Error syncing nodes from discovery: {e}");
                }
            }
            heartbeats.update_monitor();
        }
    }
}

async fn get_worker_nodes_from_dht(
    kademlia_action_tx: tokio::sync::mpsc::Sender<p2p::KademliaActionWithChannel>,
) -> Result<Vec<shared::models::node::Node>, anyhow::Error> {
    let (kad_action, mut result_rx) =
        p2p::KademliaAction::GetProviders(p2p::WORKER_DHT_KEY.as_bytes().to_vec())
            .into_kademlia_action_with_channel();
    if let Err(e) = kademlia_action_tx.send(kad_action).await {
        bail!("failed to send Kademlia action: {e}");
    }

    info!("🔄 Fetching worker nodes from DHT...");
    let mut workers = std::collections::HashSet::new();
    while let Some(result) = result_rx.recv().await {
        match result {
            Ok(res) => {
                match res {
                    p2p::KademliaQueryResult::GetProviders(res) => match res {
                        Ok(res) => match res {
                            p2p::KademliaGetProvidersOk::FoundProviders { key: _, providers } => {
                                workers.extend(providers.into_iter());
                            }
                            _ => {}
                        },
                        Err(e) => {
                            bail!("failed to get providers from DHT: {e}");
                        }
                    },
                    _ => {
                        // this case should never happen
                        bail!("unexpected Kademlia query result: {res:?}");
                    }
                }
            }
            Err(e) => {
                bail!("kademlia action failed: {e}");
            }
        }
    }

    log::debug!("got {} worker nodes from DHT", workers.len());

    let mut nodes = Vec::new();
    for peer_id in workers {
        let record_key = p2p::worker_dht_key_with_peer_id(&peer_id);
        let (kad_action, mut result_rx) =
            p2p::KademliaAction::GetRecord(record_key.as_bytes().to_vec())
                .into_kademlia_action_with_channel();
        if let Err(e) = kademlia_action_tx.send(kad_action).await {
            bail!("failed to send Kademlia action: {e}");
        }

        while let Some(result) = result_rx.recv().await {
            match result {
                Ok(res) => {
                    match res {
                        p2p::KademliaQueryResult::GetRecord(res) => match res {
                            Ok(res) => match res {
                                p2p::KademliaGetRecordOk::FoundRecord(record) => {
                                    match serde_json::from_slice::<shared::models::node::Node>(
                                        &record.record.value,
                                    ) {
                                        Ok(node) => {
                                            nodes.push(node);
                                        }
                                        Err(e) => {
                                            warn!("failed to deserialize node record: {e}");
                                        }
                                    }
                                }
                                _ => {}
                            },
                            Err(e) => {
                                warn!("failed to get record from DHT: {e}");
                            }
                        },
                        _ => {
                            // this case should never happen
                            bail!("unexpected Kademlia query result: {res:?}");
                        }
                    }
                }
                Err(e) => {
                    warn!("kademlia action failed: {e}");
                }
            }
        }
    }

    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use alloy::primitives::Address;
    use shared::models::node::{ComputeSpecs, Node};

    use super::*;
    use crate::models::node::NodeStatus;
    use crate::store::core::{RedisStore, StoreContext};

    #[tokio::test]
    async fn perform_node_updates_ok() {
        let node_address = "0x1234567890123456789012345678901234567890";
        let node = NodeWithMetadata::new(
            Node {
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
                worker_p2p_id: "12D3KooWJj3haDEzxGSbGSAvXCiE9pDYC9xHDdtQe8B2donhfwXL".to_string(),
                ..Default::default()
            },
            true,
            false,
            true,
            false,
            alloy::primitives::U256::from(1000),
            None,
            None,
            None,
        );

        let mut orchestrator_node = OrchestratorNode::from(&node);
        orchestrator_node.status = NodeStatus::Ejected;
        orchestrator_node.address = node_address.parse::<Address>().unwrap();
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
        store_context
            .node_store
            .add_node(orchestrator_node.clone())
            .await
            .unwrap();

        let updater = Updater {
            store_context: store_context.clone(),
            status_change_handlers: vec![],
        };

        let node_from_store = store_context
            .node_store
            .get_node(&orchestrator_node.address)
            .await
            .unwrap();
        assert!(node_from_store.is_some());
        if let Some(node) = node_from_store {
            assert_eq!(node.status, NodeStatus::Ejected);
        }

        updater.perform_node_updates(&node).await.unwrap();

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
    async fn first_seen_timestamp_set_on_new_node() {
        let node_address = "0x2234567890123456789012345678901234567890";
        let node = NodeWithMetadata::new(
            Node {
                id: node_address.to_string(),
                provider_address: node_address.to_string(),
                ip_address: "192.168.1.100".to_string(),
                port: 8080,
                compute_pool_id: 1,
                worker_p2p_id: "12D3KooWJj3haDEzxGSbGSAvXCiE9pDYC9xHDdtQe8B2donhfwXL".to_string(),
                ..Default::default()
            },
            true,
            true,
            true,
            false,
            alloy::primitives::U256::from(1000),
            None,
            None,
            None,
        );

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
        let updater = Updater {
            store_context: store_context.clone(),
            status_change_handlers: vec![],
        };

        let time_before = Utc::now();

        // Sync a new node that doesn't exist in the store
        updater.perform_node_updates(&node).await.unwrap();

        let time_after = Utc::now();

        // Verify the node was added with first_seen timestamp
        let node_from_store = store_context
            .node_store
            .get_node(&node_address.parse::<Address>().unwrap())
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
        let updated_node = NodeWithMetadata::new(
            Node {
                id: node_address.to_string(),
                provider_address: node_address.to_string(),
                ip_address: "192.168.1.101".to_string(), // Changed IP
                port: 8080,
                compute_pool_id: 1,
                worker_p2p_id: "12D3KooWJj3haDEzxGSbGSAvXCiE9pDYC9xHDdtQe8B2donhfwXL".to_string(),
                ..Default::default()
            },
            true,
            true,
            true,
            false,
            alloy::primitives::U256::from(1000),
            None,
            None,
            None,
        );

        // Sync the node again
        updater.perform_node_updates(&updated_node).await.unwrap();

        // Verify the node was updated but first_seen is preserved
        let node_after_resync = store_context
            .node_store
            .get_node(&node_address.parse::<Address>().unwrap())
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
    async fn sync_node_with_same_peer_id() {
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
        let node1 = NodeWithMetadata::new(
            Node {
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
                worker_p2p_id: "12D3KooWJj3haDEzxGSbGSAvXCiE9pDYC9xHDdtQe8B2donhfwXL".to_string(),
                ..Default::default()
            },
            true,
            true,
            true,
            false,
            alloy::primitives::U256::from(1000),
            None,
            None,
            None,
        );

        let mut orchestrator_node1 = OrchestratorNode::from(&node1);
        orchestrator_node1.status = NodeStatus::Healthy;
        orchestrator_node1.address = node1_address.parse::<Address>().unwrap();

        let _ = store_context
            .node_store
            .add_node(orchestrator_node1.clone())
            .await;

        // Create second node with same peer ID
        let node2_address = "0x2234567890123456789012345678901234567890";
        let node2 = NodeWithMetadata::new(
            Node {
                id: node2_address.to_string(),
                provider_address: node2_address.to_string(),
                ip_address: "127.0.0.1".to_string(),
                port: 8080,
                compute_pool_id: 1,
                compute_specs: Some(ComputeSpecs {
                    ram_mb: Some(1024),
                    storage_gb: Some(10),
                    ..Default::default()
                }),
                worker_p2p_id: "12D3KooWJj3haDEzxGSbGSAvXCiE9pDYC9xHDdtQe8B2donhfwXL".to_string(),
                ..Default::default()
            },
            true,
            true,
            true,
            false,
            alloy::primitives::U256::from(1000),
            None,
            None,
            None,
        );

        let updater = Updater {
            store_context: store_context.clone(),
            status_change_handlers: vec![],
        };

        // Try to sync the second node
        updater.perform_node_updates(&node2).await.unwrap();

        // Verify second node was not added
        let node2_result = store_context
            .node_store
            .get_node(&node2_address.parse::<Address>().unwrap())
            .await
            .unwrap();
        assert!(
            node2_result.is_none(),
            "Node with same peer ID should not be added"
        );
    }
}
