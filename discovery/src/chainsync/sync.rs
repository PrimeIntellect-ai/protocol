use crate::store::node_store::NodeStore;
use alloy::primitives::Address;
use anyhow::Result;
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

    pub async fn run(&self) -> Result<()> {
        // TODO loop
        // Check local store
        // Check store on chain
        // Update status

        let node_store_clone = self.node_store.clone();
        let contracts_clone = self.contracts.clone();
        let cancel_token = self.cancel_token.clone();
        let chain_sync_interval = self.chain_sync_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(chain_sync_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // TODO: Implement chain sync
                        let nodes = node_store_clone.get_nodes();
                        for node in nodes {
                            let mut n = node.clone();
                            let provider_address = Address::from_str(&node.provider_address).unwrap();
                            let node_address = Address::from_str(&node.id).unwrap();
                            let node_info = contracts_clone.compute_registry.get_node(provider_address, node_address).await.unwrap();
                            let (is_active, is_validated) = node_info;
                            n.is_active = is_active;
                            n.is_validated = is_validated;
                            node_store_clone.update_node(n);
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
