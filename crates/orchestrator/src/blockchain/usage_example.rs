use alloy::primitives::Address;
use alloy_provider::Provider as AlloyProvider;
use log::info;
use shared::utils::StorageProvider;
use shared::web3::contracts::core::builder::Contracts;
use std::sync::Arc;

use crate::blockchain::BlockchainIntegrationBuilder;
use crate::error::OrchestratorError;
use crate::plugins::node_groups::NodeGroupsPlugin;

#[allow(dead_code)]
pub async fn setup_blockchain_monitoring<P: AlloyProvider>(
    ws_rpc_url: &str,
    synthetic_data_validator_address: &str,
    contracts: Arc<Contracts<P>>,
    group_manager: Arc<NodeGroupsPlugin>,
    storage_provider: Arc<dyn StorageProvider>,
) -> Result<(), OrchestratorError> {
    let validator_address: Address = synthetic_data_validator_address
        .parse()
        .map_err(|e| OrchestratorError::Custom(format!("Invalid address: {}", e)))?;

    let blockchain_integration = BlockchainIntegrationBuilder::new()
        .with_websocket_url(ws_rpc_url.to_string())
        .with_synthetic_data_validator_address(validator_address)
        .with_contracts(contracts)
        .with_group_manager(group_manager)
        .with_storage_provider(storage_provider)
        .build()
        .await?;

    blockchain_integration.start().await?;

    info!("Blockchain monitoring started - listening for soft invalidation events");

    Ok(())
}

pub struct BlockchainConfig {
    #[allow(dead_code)]
    pub ws_rpc_url: String,
    #[allow(dead_code)]
    pub synthetic_data_validator_address: String,
    #[allow(dead_code)]
    pub enabled: bool,
}

impl Default for BlockchainConfig {
    fn default() -> Self {
        Self {
            ws_rpc_url: "wss://eth-mainnet.g.alchemy.com/v2/YOUR_API_KEY".to_string(),
            synthetic_data_validator_address: "0x0000000000000000000000000000000000000000"
                .to_string(),
            enabled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blockchain_config_default() {
        let config = BlockchainConfig::default();
        assert!(!config.enabled);
        assert!(config.ws_rpc_url.contains("wss://"));
    }

    #[test]
    fn test_address_parsing() {
        let valid_address = "0x742d35Cc6634C0532925a3b8D9c7dd6C566b3B35";
        let parsed: Result<Address, _> = valid_address.parse();
        assert!(parsed.is_ok());
    }
}
