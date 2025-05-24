use crate::store::node_store::NodeStore;
use alloy::primitives::Address;
use anyhow::Error;
use futures::StreamExt;
use log::error;
use shared::models::node::DiscoveryNode;
use shared::web3::contracts::core::builder::Contracts;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
pub struct ChainSync {
    pub node_store: Arc<NodeStore>,
    cancel_token: CancellationToken,
    chain_sync_interval: Duration,
    contracts: Arc<Contracts>,
    last_chain_sync: Arc<Mutex<Option<std::time::SystemTime>>>,
}

impl ChainSync {
    pub fn new(
        node_store: Arc<NodeStore>,
        cancellation_token: CancellationToken,
        chain_sync_interval: Duration,
        contracts: Arc<Contracts>,
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
        contracts: Arc<Contracts>,
        mut node: DiscoveryNode,
    ) -> Result<(), Error> {
        // Safely parse provider_address and node_address
        let provider_address = Address::from_str(&node.provider_address).map_err(|e| {
            eprintln!("Failed to parse provider address: {}", e);
            anyhow::anyhow!("Invalid provider address")
        })?;

        let node_address = Address::from_str(&node.id).map_err(|e| {
            eprintln!("Failed to parse node address: {}", e);
            anyhow::anyhow!("Invalid node address")
        })?;

        let (node_info_result, provider_info_result, is_blacklisted_result) = tokio::join!(
            contracts
                .compute_registry
                .get_node(provider_address, node_address),
            contracts.compute_registry.get_provider(provider_address),
            contracts
                .compute_pool
                .is_node_blacklisted(node.node.compute_pool_id, node_address),
        );

        let node_info = node_info_result.map_err(|e| {
            eprintln!("Error retrieving node info: {}", e);
            anyhow::anyhow!("Failed to retrieve node info")
        })?;

        let provider_info = provider_info_result.map_err(|e| {
            eprintln!("Error retrieving provider info: {}", e);
            anyhow::anyhow!("Failed to retrieve provider info")
        })?;

        let is_blacklisted = is_blacklisted_result.map_err(|e| {
            eprintln!("Error checking if node is blacklisted: {}", e);
            anyhow::anyhow!("Failed to check blacklist status")
        })?;

        let (is_active, is_validated) = node_info;
        node.is_active = is_active;
        node.is_validated = is_validated;
        node.is_provider_whitelisted = provider_info.is_whitelisted;
        node.is_blacklisted = is_blacklisted;
        match node_store.update_node(node) {
            Ok(_) => (),
            Err(e) => {
                error!("Error updating node: {}", e);
            }
        }

        Ok(())
    }

    pub async fn run(&self) -> Result<(), Error> {
        let node_store_clone = self.node_store.clone();
        let contracts_clone = self.contracts.clone();
        let cancel_token = self.cancel_token.clone();
        let chain_sync_interval = self.chain_sync_interval;
        let last_chain_sync = self.last_chain_sync.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(chain_sync_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let nodes = node_store_clone.get_nodes();
                        match nodes {
                            Ok(nodes) => {
                                futures::stream::iter(nodes)
                                    .for_each_concurrent(10, |node| {
                                        let node_store = node_store_clone.clone();
                                        let contracts = contracts_clone.clone();
                                        async move {
                                            if let Err(e) = ChainSync::sync_single_node(node_store, contracts, node).await {
                                                error!("Error syncing node: {}", e);
                                            }
                                        }
                                    })
                                    .await;
                                // Update the last chain sync time
                                let mut last_sync = last_chain_sync.lock().await;
                                *last_sync = Some(SystemTime::now());
                            }
                            Err(e) => {
                                error!("Error getting nodes: {}", e);
                            }
                        }
                    }
                    _ = cancel_token.cancelled() => {
                        break;
                    }
                }
            }
        });
        Ok(())
    }
}
