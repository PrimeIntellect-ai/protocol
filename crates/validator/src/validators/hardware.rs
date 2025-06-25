use alloy::primitives::Address;
use anyhow::Result;
use log::{debug, error, info};
use shared::{
    models::node::DiscoveryNode,
    web3::{
        contracts::core::builder::Contracts,
        wallet::{Wallet, WalletProvider},
    },
};

use crate::p2p::client::P2PClient;
use crate::validators::hardware_challenge::HardwareChallenge;

/// Hardware validator implementation
///
/// NOTE: This is a temporary implementation that will be replaced with a proper
/// hardware validator in the near future. The current implementation only performs
/// basic matrix multiplication challenges and does not verify actual hardware specs.
pub struct HardwareValidator<'a> {
    wallet: &'a Wallet,
    contracts: Contracts<WalletProvider>,
    p2p_client: Option<&'a P2PClient>,
}

impl<'a> HardwareValidator<'a> {
    pub fn new(
        wallet: &'a Wallet,
        contracts: Contracts<WalletProvider>,
        p2p_client: Option<&'a P2PClient>,
    ) -> Self {
        Self {
            wallet,
            contracts,
            p2p_client,
        }
    }

    async fn validate_node(
        _wallet: &'a Wallet,
        contracts: Contracts<WalletProvider>,
        p2p_client: Option<&'a P2PClient>,
        node: DiscoveryNode,
    ) -> Result<()> {
        let node_address = match node.id.trim_start_matches("0x").parse::<Address>() {
            Ok(addr) => addr,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to parse node address: {}", e));
            }
        };

        let provider_address = match node
            .provider_address
            .trim_start_matches("0x")
            .parse::<Address>()
        {
            Ok(addr) => addr,
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to parse provider address: {}", e));
            }
        };

        // Perform hardware challenge
        if let Some(p2p_client) = p2p_client {
            let hardware_challenge = HardwareChallenge::new(p2p_client);
            let challenge_result = hardware_challenge.challenge_node(&node).await;

            if let Err(e) = challenge_result {
                println!("Challenge failed for node: {}, error: {}", node.id, e);
                error!("Challenge failed for node: {}, error: {}", node.id, e);
                return Err(anyhow::anyhow!("Failed to challenge node: {}", e));
            }
        } else {
            debug!(
                "P2P client not available, skipping hardware challenge for node {}",
                node.id
            );
        }

        debug!("Sending validation transaction for node {}", node.id);

        if let Err(e) = contracts
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

        let contracts = self.contracts.clone();
        let wallet = self.wallet;
        let p2p_client = self.p2p_client;

        // Process non validated nodes sequentially as simple fix
        // to avoid nonce conflicts for now. Will sophisticate this in the future
        for node in non_validated {
            let node_id = node.id.clone();
            match HardwareValidator::validate_node(wallet, contracts.clone(), p2p_client, node)
                .await
            {
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

        let validator = HardwareValidator::new(&coordinator_wallet, contracts, None);

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
        println!("Validation took: {:?}", elapsed);

        assert!(result.is_ok());
    }
}
