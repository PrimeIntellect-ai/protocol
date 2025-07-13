use alloy::primitives::Address;
use anyhow::bail;
use anyhow::Result;
use log::{debug, error, info};
use shared::{
    models::node::DiscoveryNode,
    web3::{contracts::core::builder::Contracts, wallet::WalletProvider},
};

use crate::p2p::HardwareChallengeRequest;
use crate::validators::hardware_challenge::HardwareChallenge;

/// Hardware validator implementation
///
/// NOTE: This is a temporary implementation that will be replaced with a proper
/// hardware validator in the near future. The current implementation only performs
/// basic matrix multiplication challenges and does not verify actual hardware specs.
pub struct HardwareValidator {
    contracts: Contracts<WalletProvider>,
    challenge_tx: tokio::sync::mpsc::Sender<HardwareChallengeRequest>,
}

impl HardwareValidator {
    pub fn new(
        contracts: Contracts<WalletProvider>,
        challenge_tx: tokio::sync::mpsc::Sender<HardwareChallengeRequest>,
    ) -> Self {
        Self {
            contracts,
            challenge_tx,
        }
    }

    async fn validate_node(&self, node: DiscoveryNode) -> Result<()> {
        let node_address = match node.id.trim_start_matches("0x").parse::<Address>() {
            Ok(addr) => addr,
            Err(e) => {
                bail!("failed to parse node address: {e:?}");
            }
        };

        let provider_address = match node
            .provider_address
            .trim_start_matches("0x")
            .parse::<Address>()
        {
            Ok(addr) => addr,
            Err(e) => {
                bail!("failed to parse provider address: {e:?}");
            }
        };

        // Perform hardware challenge
        let hardware_challenge = HardwareChallenge::new(self.challenge_tx.clone());
        let challenge_result = hardware_challenge.challenge_node(&node).await;

        if let Err(e) = challenge_result {
            bail!("failed to challenge node: {e:?}");
        }

        debug!("Sending validation transaction for node {}", node.id);

        if let Err(e) = self
            .contracts
            .prime_network
            .validate_node(provider_address, node_address)
            .await
        {
            error!("Failed to validate node: {e}");
            return Err(anyhow::anyhow!("Failed to validate node: {}", e));
        }

        // Small delay to ensure nonce incrementation
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        info!("Node {} successfully validated", node.id);
        Ok(())
    }

    pub async fn validate_nodes(&self, nodes: Vec<DiscoveryNode>) -> Result<()> {
        let non_validated: Vec<_> = nodes.into_iter().filter(|n| !n.is_validated).collect();
        debug!("Non validated nodes: {non_validated:?}");
        info!("Starting validation for {} nodes", non_validated.len());

        // Process non validated nodes sequentially as simple fix
        // to avoid nonce conflicts for now. Will sophisticate this in the future
        for node in non_validated {
            let node_id = node.id.clone();
            match self.validate_node(node).await {
                Ok(_) => (),
                Err(e) => {
                    error!("Failed to validate node {node_id}: {e}");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::models::node::Node;
    use shared::web3::contracts::core::builder::ContractBuilder;
    use shared::web3::wallet::Wallet;
    use std::sync::Arc;
    use url::Url;

    #[tokio::test]
    async fn test_challenge_node() {
        let coordinator_key = "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97";
        let rpc_url: Url = Url::parse("http://localhost:8545").unwrap();
        let coordinator_wallet = Arc::new(Wallet::new(coordinator_key, rpc_url).unwrap());

        let contracts = ContractBuilder::new(coordinator_wallet.provider())
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .build()
            .unwrap();

        let (tx, _rx) = tokio::sync::mpsc::channel(100);
        let validator = HardwareValidator::new(contracts, tx);

        let fake_discovery_node1 = DiscoveryNode {
            is_validated: false,
            node: Node {
                ip_address: "192.168.1.1".to_string(),
                port: 8080,
                compute_pool_id: 1,
                id: Address::ZERO.to_string(),
                provider_address: Address::ZERO.to_string(),
                ..Default::default()
            },
            is_active: true,
            is_provider_whitelisted: true,
            is_blacklisted: false,
            ..Default::default()
        };

        let fake_discovery_node2 = DiscoveryNode {
            is_validated: false,
            node: Node {
                ip_address: "192.168.1.2".to_string(),
                port: 8080,
                compute_pool_id: 1,
                id: Address::ZERO.to_string(),
                provider_address: Address::ZERO.to_string(),
                ..Default::default()
            },
            is_active: true,
            is_provider_whitelisted: true,
            is_blacklisted: false,
            ..Default::default()
        };

        let nodes = vec![fake_discovery_node1, fake_discovery_node2];

        let start_time = std::time::Instant::now();
        let result = validator.validate_nodes(nodes).await;
        let elapsed = start_time.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(11));
        println!("Validation took: {elapsed:?}");

        assert!(result.is_ok());
    }
}
