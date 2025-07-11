use crate::utils::message_queue::{Message, MessageQueue as GenericMessageQueue};
use pyo3::prelude::*;

/// Queue types for the worker message queue
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueType {
    PoolOwner,
    Validator,
}

/// Worker-specific message queue with predefined queue types
#[derive(Clone)]
pub struct MessageQueue {
    pool_owner_queue: GenericMessageQueue,
    validator_queue: GenericMessageQueue,
}

impl MessageQueue {
    /// Create a new worker message queue with pool_owner and validator queues
    pub fn new() -> Self {
        Self {
            pool_owner_queue: GenericMessageQueue::new(None),
            validator_queue: GenericMessageQueue::new(None),
        }
    }

    /// Start the background message listener for worker
    pub(crate) async fn start_listener(&self) -> Result<(), String> {
        // Start mock listeners with different frequencies
        // pool_owner messages every 2 seconds, validator messages every 3 seconds
        self.pool_owner_queue.start_mock_listener(2).await?;
        self.validator_queue.start_mock_listener(3).await?;
        Ok(())
    }

    /// Stop the background listener
    pub(crate) async fn stop_listener(&self) -> Result<(), String> {
        self.pool_owner_queue.stop_listener().await?;
        self.validator_queue.stop_listener().await?;
        Ok(())
    }

    /// Get the next message from the pool owner queue
    pub(crate) async fn get_pool_owner_message(&self) -> Option<PyObject> {
        self.pool_owner_queue.get_message().await
    }

    /// Get the next message from the validator queue
    pub(crate) async fn get_validator_message(&self) -> Option<PyObject> {
        self.validator_queue.get_message().await
    }

    /// Push a message to the appropriate queue (for testing or internal use)
    pub(crate) async fn push_message(
        &self,
        queue_type: QueueType,
        content: serde_json::Value,
    ) -> Result<(), String> {
        let message = Message {
            content,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            sender: Some("worker".to_string()),
        };

        match queue_type {
            QueueType::PoolOwner => self.pool_owner_queue.push_message(message).await,
            QueueType::Validator => self.validator_queue.push_message(message).await,
        }
    }

    /// Get queue sizes for monitoring
    pub(crate) async fn get_queue_sizes(&self) -> (usize, usize) {
        let pool_owner_size = self.pool_owner_queue.get_queue_size().await;
        let validator_size = self.validator_queue.get_queue_size().await;
        (pool_owner_size, validator_size)
    }
}
