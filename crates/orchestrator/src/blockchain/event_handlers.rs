use alloy::rpc::types::Log;
use async_trait::async_trait;
use log::info;
use std::collections::HashMap;

use crate::error::OrchestratorError;

#[async_trait]
pub trait EventHandler: Send + Sync {
    #[allow(dead_code)]
    async fn handle_event(&self, log: Log) -> Result<(), OrchestratorError>;
    fn event_signature(&self) -> &str;
    fn name(&self) -> &str;
}

pub struct EventHandlerRegistry {
    handlers: HashMap<String, Vec<Box<dyn EventHandler>>>,
}

impl EventHandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn register(&mut self, event_signature: &str, handler: Box<dyn EventHandler>) {
        info!(
            "Registering event handler '{}' for signature: {}",
            handler.name(),
            event_signature
        );

        self.handlers
            .entry(event_signature.to_string())
            .or_default()
            .push(handler);
    }

    #[allow(dead_code)]
    pub fn get_handlers(&self, event_signature: &str) -> Option<&Vec<Box<dyn EventHandler>>> {
        self.handlers.get(event_signature)
    }

    #[allow(dead_code)]
    pub fn remove_handlers(&mut self, event_signature: &str) {
        self.handlers.remove(event_signature);
    }

    #[allow(dead_code)]
    pub fn list_registered_events(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }
}

impl Default for EventHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
