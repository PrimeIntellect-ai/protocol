use crate::store::node_store::NodeStore;
use alloy::primitives::Address;
use alloy::providers::RootProvider;
use anyhow::Error;
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
            eprintln!("Failed to parse provider address: {}", e);
            anyhow::anyhow!("Invalid provider address")
        })?;

        let node_address = Address::from_str(&node.id).map_err(|e| {
            eprintln!("Failed to parse node address: {}", e);
            anyhow::anyhow!("Invalid node address")
        })?;

        let node_info = contracts
            .compute_registry
            .get_node(provider_address, node_address)
            .await
            .map_err(|e| {
                eprintln!("Error retrieving node info: {}", e);
                anyhow::anyhow!("Failed to retrieve node info")
            })?;

        let provider_info = contracts
            .compute_registry
            .get_provider(provider_address)
            .await
            .map_err(|e| {
                eprintln!("Error retrieving provider info: {}", e);
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
                eprintln!("Error checking if node is blacklisted: {}", e);
                anyhow::anyhow!("Failed to check blacklist status")
            })?;
        n.is_blacklisted = is_blacklisted;
        match node_store.update_node(n) {
            Ok(_) => (),
            Err(e) => {
                error!("Error updating node: {}", e);
            }
        }

        Ok(())
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
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let nodes = node_store.get_nodes();
                        match nodes {
                            Ok(nodes) => {
                                for node in nodes {
                                    if let Err(e) = ChainSync::sync_single_node(node_store.clone(), contracts.clone(), node).await {
                                        error!("Error syncing node: {}", e);
                                    }
                                }
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
