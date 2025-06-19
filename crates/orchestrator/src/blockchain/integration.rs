use alloy::primitives::Address;
use alloy_provider::Provider as AlloyProvider;
use log::info;
use shared::utils::StorageProvider;
use shared::web3::contracts::core::builder::Contracts;
use std::sync::Arc;

use crate::blockchain::{
    event_monitor::ContractConfig, EventHandler, EventMonitor, EventMonitorConfig,
    SoftInvalidationHandler,
};
use crate::error::OrchestratorError;
use crate::plugins::node_groups::NodeGroupsPlugin;

pub struct BlockchainIntegration<P: AlloyProvider> {
    event_monitor: EventMonitor<P>,
    _handlers: Vec<Box<dyn crate::blockchain::EventHandler>>, // Keep handlers alive
}

impl<P: AlloyProvider> BlockchainIntegration<P> {
    pub async fn new(
        ws_url: String,
        synthetic_data_validator_address: Address,
        contracts: Arc<Contracts<P>>,
        group_manager: Arc<NodeGroupsPlugin>,
        storage_provider: Arc<dyn StorageProvider>,
    ) -> Result<Self, OrchestratorError> {
        // Create soft invalidation handler
        let soft_invalidation_handler = Box::new(SoftInvalidationHandler::new(
            group_manager,
            storage_provider.clone(),
        ));

        // Set up event monitor configuration
        let config = EventMonitorConfig {
            ws_url,
            poll_interval_secs: 5,
            contracts: vec![
                ContractConfig {
                    name: "SyntheticDataWorkValidator".to_string(),
                    address: synthetic_data_validator_address,
                    events: vec!["WorkInvalidated".to_string()],
                },
                // Add more contracts here as needed
            ],
        };

        // Create event monitor
        let event_monitor = EventMonitor::new(config, contracts);

        // Register handlers
        let event_signature = soft_invalidation_handler.event_signature().to_string();
        event_monitor
            .register_handler(&event_signature, soft_invalidation_handler)
            .await?;

        info!("Blockchain integration initialized with soft invalidation monitoring");

        Ok(Self {
            event_monitor,
            _handlers: vec![], // Handlers are managed by the event monitor
        })
    }

    pub async fn start(&self) -> Result<(), OrchestratorError> {
        info!("Starting blockchain event monitoring");
        self.event_monitor.start().await
    }

    #[allow(dead_code)]
    pub async fn stop(&self) -> Result<(), OrchestratorError> {
        info!("Stopping blockchain event monitoring");
        self.event_monitor.stop().await
    }

    #[allow(dead_code)]
    pub async fn add_handler(
        &self,
        event_signature: &str,
        handler: Box<dyn crate::blockchain::EventHandler>,
    ) -> Result<(), OrchestratorError> {
        self.event_monitor
            .register_handler(event_signature, handler)
            .await
    }
}

pub struct BlockchainIntegrationBuilder<P: AlloyProvider> {
    ws_url: Option<String>,
    synthetic_data_validator_address: Option<Address>,
    contracts: Option<Arc<Contracts<P>>>,
    group_manager: Option<Arc<NodeGroupsPlugin>>,
    storage_provider: Option<Arc<dyn StorageProvider>>,
}

impl<P: AlloyProvider> BlockchainIntegrationBuilder<P> {
    pub fn new() -> Self {
        Self {
            ws_url: None,
            synthetic_data_validator_address: None,
            contracts: None,
            group_manager: None,
            storage_provider: None,
        }
    }

    pub fn with_websocket_url(mut self, url: String) -> Self {
        self.ws_url = Some(url);
        self
    }

    pub fn with_synthetic_data_validator_address(mut self, address: Address) -> Self {
        self.synthetic_data_validator_address = Some(address);
        self
    }

    pub fn with_contracts(mut self, contracts: Arc<Contracts<P>>) -> Self {
        self.contracts = Some(contracts);
        self
    }

    pub fn with_group_manager(mut self, manager: Arc<NodeGroupsPlugin>) -> Self {
        self.group_manager = Some(manager);
        self
    }

    pub fn with_storage_provider(mut self, provider: Arc<dyn StorageProvider>) -> Self {
        self.storage_provider = Some(provider);
        self
    }

    pub async fn build(self) -> Result<BlockchainIntegration<P>, OrchestratorError> {
        let ws_url = self
            .ws_url
            .ok_or_else(|| OrchestratorError::Custom("WebSocket URL is required".to_string()))?;

        let synthetic_data_validator_address =
            self.synthetic_data_validator_address.ok_or_else(|| {
                OrchestratorError::Custom(
                    "Synthetic data validator address is required".to_string(),
                )
            })?;

        let contracts = self
            .contracts
            .ok_or_else(|| OrchestratorError::Custom("Contracts are required".to_string()))?;

        let group_manager = self
            .group_manager
            .ok_or_else(|| OrchestratorError::Custom("Group manager is required".to_string()))?;

        let storage_provider = self
            .storage_provider
            .ok_or_else(|| OrchestratorError::Custom("Storage provider is required".to_string()))?;

        BlockchainIntegration::new(
            ws_url,
            synthetic_data_validator_address,
            contracts,
            group_manager,
            storage_provider,
        )
        .await
    }
}

impl<P: AlloyProvider> Default for BlockchainIntegrationBuilder<P> {
    fn default() -> Self {
        Self::new()
    }
}
