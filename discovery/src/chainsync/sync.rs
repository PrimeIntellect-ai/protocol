use crate::store::node_store::NodeStore;
use alloy::primitives::Address;
use anyhow::Error;
use log::error;
use shared::models::node::DiscoveryNode;
use shared::web3::contracts::core::builder::Contracts;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
pub struct ChainSync {
    pub node_store: Arc<NodeStore>,
    cancel_token: CancellationToken,
    chain_sync_interval: Duration,
    contracts: Arc<Contracts>,
}

impl ChainSync {
    pub fn new(
        node_store: Arc<NodeStore>,
        cancellation_token: CancellationToken,
        chain_sync_interval: Duration,
        contracts: Arc<Contracts>,
    ) -> Self {
        Self {
            node_store,
            cancel_token: cancellation_token,
            chain_sync_interval,
            contracts,
        }
    }
    async fn sync_single_node(
        node_store: Arc<NodeStore>,
        contracts: Arc<Contracts>,
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

        // Handle potential errors from async calls
        let is_blacklisted = contracts
            .compute_pool
            .is_node_blacklisted(node.node.compute_pool_id, node_address)
            .await
            .map_err(|e| {
                eprintln!("Error checking if node is blacklisted: {}", e);
                anyhow::anyhow!("Failed to check blacklist status")
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
        n.is_blacklisted = is_blacklisted;
        node_store.update_node(n);

        Ok(())
    }

    pub async fn run(&self) -> Result<(), Error> {
        let node_store_clone = self.node_store.clone();
        let contracts_clone = self.contracts.clone();
        let cancel_token = self.cancel_token.clone();
        let chain_sync_interval = self.chain_sync_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(chain_sync_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let nodes = node_store_clone.get_nodes();
                        for node in nodes {
                            if let Err(e) = ChainSync::sync_single_node(node_store_clone.clone(), contracts_clone.clone(), node).await {
                                error!("Error syncing node: {}", e);
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
