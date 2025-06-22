use crate::store::node_store::NodeStore;
use alloy::primitives::Address;
use alloy::providers::RootProvider;
use anyhow::Error;
use futures::stream::{self, StreamExt};
use log::{debug, error, info, warn};
use shared::models::node::DiscoveryNode;
use shared::web3::contracts::core::builder::Contracts;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const MAX_CONCURRENT_SYNCS: usize = 50;

pub struct ChainSync {
    pub node_store: Arc<NodeStore>,
    cancel_token: CancellationToken,
    chain_sync_interval: Duration,
    contracts: Contracts<RootProvider>,
    last_chain_sync: Arc<Mutex<Option<std::time::SystemTime>>>,
}

impl ChainSync {
    pub fn new(
        node_store: Arc<NodeStore>,
        cancellation_token: CancellationToken,
        chain_sync_interval: Duration,
        contracts: Contracts<RootProvider>,
        last_chain_sync: Arc<Mutex<Option<std::time::SystemTime>>>,
    ) -> Self {
        Self {
            node_store,
            cancel_token: cancellation_token,
            chain_sync_interval,
            contracts,
            last_chain_sync,
        }
    }

    async fn sync_single_node(
        node_store: Arc<NodeStore>,
        contracts: Contracts<RootProvider>,
        node: DiscoveryNode,
    ) -> Result<(), Error> {
        let mut n = node.clone();

        // Safely parse provider_address and node_address
        let provider_address = Address::from_str(&node.provider_address).map_err(|e| {
            error!(
                "Failed to parse provider address '{}': {}",
                node.provider_address, e
            );
            anyhow::anyhow!("Invalid provider address")
        })?;

        let node_address = Address::from_str(&node.id).map_err(|e| {
            error!("Failed to parse node address '{}': {}", node.id, e);
            anyhow::anyhow!("Invalid node address")
        })?;

        let node_info = contracts
            .compute_registry
            .get_node(provider_address, node_address)
            .await
            .map_err(|e| {
                error!(
                    "Error retrieving node info for provider {} and node {}: {}",
                    provider_address, node_address, e
                );
                anyhow::anyhow!("Failed to retrieve node info")
            })?;

        let provider_info = contracts
            .compute_registry
            .get_provider(provider_address)
            .await
            .map_err(|e| {
                error!(
                    "Error retrieving provider info for {}: {}",
                    provider_address, e
                );
                anyhow::anyhow!("Failed to retrieve provider info")
            })?;

        let (is_active, is_validated) = node_info;
        n.is_active = is_active;
        n.is_validated = is_validated;
        n.is_provider_whitelisted = provider_info.is_whitelisted;

        // Handle potential errors from async calls
        let is_blacklisted = contracts
            .compute_pool
            .is_node_blacklisted(node.node.compute_pool_id, node_address)
            .await
            .map_err(|e| {
                error!(
                    "Error checking if node {} is blacklisted in pool {}: {}",
                    node_address, node.node.compute_pool_id, e
                );
                anyhow::anyhow!("Failed to check blacklist status")
            })?;
        n.is_blacklisted = is_blacklisted;

        // Only update if the node has changed
        if n.is_active != node.is_active
            || n.is_validated != node.is_validated
            || n.is_provider_whitelisted != node.is_provider_whitelisted
            || n.is_blacklisted != node.is_blacklisted
        {
            match node_store.update_node(n).await {
                Ok(_) => {
                    debug!("Successfully updated node {}", node.id);
                    Ok(())
                }
                Err(e) => {
                    error!("Error updating node {}: {}", node.id, e);
                    Err(anyhow::anyhow!("Failed to update node: {}", e))
                }
            }
        } else {
            debug!("Node {} unchanged, skipping update", node.id);
            Ok(())
        }
    }

    pub async fn run(self) -> Result<(), Error> {
        let ChainSync {
            node_store,
            cancel_token,
            chain_sync_interval,
            last_chain_sync,
            contracts,
        } = self;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(chain_sync_interval);
            info!(
                "Chain sync started with {} second interval",
                chain_sync_interval.as_secs()
            );

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let sync_start = SystemTime::now();
                        info!("Starting chain sync cycle");

                        let nodes = node_store.get_nodes().await;
                        match nodes {
                            Ok(nodes) => {
                                let total_nodes = nodes.len();
                                info!("Syncing {} nodes", total_nodes);

                                // Process nodes in parallel with concurrency limit
                                let results: Vec<Result<(), Error>> = stream::iter(nodes)
                                    .map(|node| {
                                        let node_store = node_store.clone();
                                        let contracts = contracts.clone();
                                        async move {
                                            ChainSync::sync_single_node(node_store, contracts, node).await
                                        }
                                    })
                                    .buffer_unordered(MAX_CONCURRENT_SYNCS)
                                    .collect()
                                    .await;

                                // Count successes and failures
                                let mut success_count = 0;
                                let mut failure_count = 0;
                                for result in results {
                                    match result {
                                        Ok(_) => success_count += 1,
                                        Err(e) => {
                                            failure_count += 1;
                                            warn!("Node sync failed: {}", e);
                                        }
                                    }
                                }

                                // Update the last chain sync time
                                let mut last_sync = last_chain_sync.lock().await;
                                *last_sync = Some(SystemTime::now());

                                let sync_duration = SystemTime::now()
                                    .duration_since(sync_start)
                                    .unwrap_or_default();

                                info!(
                                    "Chain sync completed in {:.2}s: {} successful, {} failed out of {} total nodes",
                                    sync_duration.as_secs_f64(),
                                    success_count,
                                    failure_count,
                                    total_nodes
                                );
                            }
                            Err(e) => {
                                error!("Error getting nodes from store: {}", e);
                            }
                        }
                    }
                    _ = cancel_token.cancelled() => {
                        info!("Chain sync cancelled, shutting down");
                        break;
                    }
                }
            }
            info!("Chain sync task ended");
        });
        Ok(())
    }
}
