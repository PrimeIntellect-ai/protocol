use crate::utils::message_queue::{Message, MessageQueue as GenericMessageQueue};
use pyo3::prelude::*;

/// Validator-specific message queue for incoming validation results
#[derive(Clone)]
pub struct MessageQueue {
    inner: GenericMessageQueue,
}

impl MessageQueue {
    /// Create a new validator message queue for validation results
    pub fn new() -> Self {
        let inner = GenericMessageQueue::new(None);

        Self { inner }
    }

    /// Get the next validation result from nodes
    pub async fn get_validation_result(&self) -> Option<PyObject> {
        self.inner.get_message().await
    }

    /// Push a validation result (for testing or internal use)
    pub async fn push_validation_result(&self, content: serde_json::Value) -> Result<(), String> {
        let message = Message {
            content,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            sender: None, // Will be set to the node ID when implemented
        };

        self.inner.push_message(message).await
    }

    /// Get the number of pending validation results
    pub async fn get_queue_size(&self) -> usize {
        self.inner.get_queue_size().await
    }

    /// Clear all validation results (use with caution)
    pub async fn clear(&self) -> Result<(), String> {
        self.inner.clear().await
    }
}
