use alloy::primitives::Address;
use alloy::providers::{Provider, ProviderBuilder, WsConnect};
use alloy::rpc::types::{Filter, Log};
use alloy_provider::Provider as AlloyProvider;
use log::{error, info};
use shared::web3::contracts::core::builder::Contracts;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::blockchain::event_handlers::{EventHandler, EventHandlerRegistry};
use crate::error::OrchestratorError;

#[derive(Debug, Clone)]
pub struct EventMonitorConfig {
    pub ws_url: String,
    #[allow(dead_code)]
    pub poll_interval_secs: u64,
    pub contracts: Vec<ContractConfig>,
}

#[derive(Debug, Clone)]
pub struct ContractConfig {
    pub name: String,
    pub address: Address,
    #[allow(dead_code)]
    pub events: Vec<String>,
}

pub struct EventMonitor<P: AlloyProvider> {
    config: EventMonitorConfig,
    #[allow(dead_code)]
    contracts: Arc<Contracts<P>>,
    handler_registry: Arc<RwLock<EventHandlerRegistry>>,
    running: Arc<RwLock<bool>>,
}

impl<P: AlloyProvider> EventMonitor<P> {
    pub fn new(config: EventMonitorConfig, contracts: Arc<Contracts<P>>) -> Self {
        Self {
            config,
            contracts,
            handler_registry: Arc::new(RwLock::new(EventHandlerRegistry::new())),
            running: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn register_handler(
        &self,
        event_signature: &str,
        handler: Box<dyn EventHandler>,
    ) -> Result<(), OrchestratorError> {
        let mut registry = self.handler_registry.write().await;
        registry.register(event_signature, handler);
        Ok(())
    }

    pub async fn start(&self) -> Result<(), OrchestratorError> {
        let mut running = self.running.write().await;
        if *running {
            return Err(OrchestratorError::Custom(
                "Event monitor already running".to_string(),
            ));
        }
        *running = true;
        drop(running);

        info!("Starting blockchain event monitor");

        let ws_connect = WsConnect::new(&self.config.ws_url);
        let provider = ProviderBuilder::new()
            .connect_ws(ws_connect)
            .await
            .map_err(|e| {
                OrchestratorError::Custom(format!("Failed to connect to WebSocket: {}", e))
            })?;

        let mut filters = Vec::new();
        for contract in &self.config.contracts {
            let filter = Filter::new().address(contract.address).from_block(0u64);

            filters.push((contract.clone(), filter));
        }

        let _monitor_handle = tokio::spawn({
            let handler_registry = self.handler_registry.clone();
            let running = self.running.clone();
            let provider = Arc::new(provider);

            async move {
                for (contract, filter) in filters {
                    let provider = provider.clone();
                    let _handler_registry = handler_registry.clone();
                    let running = running.clone();

                    tokio::spawn(async move {
                        let _event_stream = match provider.subscribe_logs(&filter).await {
                            Ok(stream) => stream,
                            Err(e) => {
                                error!("Failed to subscribe to logs for {}: {}", contract.name, e);
                                return;
                            }
                        };

                        info!("Subscribed to events for contract: {}", contract.name);

                        info!(
                            "Monitoring blockchain events for contract: {} (subscription created)",
                            contract.name
                        );
                        while *running.read().await {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    });
                }
            }
        });

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn stop(&self) -> Result<(), OrchestratorError> {
        let mut running = self.running.write().await;
        *running = false;
        info!("Stopping blockchain event monitor");
        Ok(())
    }

    #[allow(dead_code)]
    async fn handle_log(
        log: Log,
        handler_registry: &Arc<RwLock<EventHandlerRegistry>>,
    ) -> Result<(), OrchestratorError> {
        if log.topics().is_empty() {
            return Ok(());
        }

        let event_signature = log.topics()[0];
        let registry = handler_registry.read().await;

        if let Some(handlers) = registry.get_handlers(&event_signature.to_string()) {
            for handler in handlers {
                if let Err(e) = handler.handle_event(log.clone()).await {
                    error!("Event handler failed: {}", e);
                }
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn handler_registry(&self) -> Arc<RwLock<EventHandlerRegistry>> {
        self.handler_registry.clone()
    }
}
