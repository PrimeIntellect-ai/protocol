use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};

use crate::utils::json_parser::json_to_pyobject;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub message_type: MessageType,
    pub content: serde_json::Value,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageType {
    PoolOwner,
    Validator,
}

#[derive(Clone)]
pub struct MessageQueue {
    pool_owner_queue: Arc<Mutex<VecDeque<Message>>>,
    validator_queue: Arc<Mutex<VecDeque<Message>>>,
    shutdown_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

impl MessageQueue {
    pub fn new() -> Self {
        Self {
            pool_owner_queue: Arc::new(Mutex::new(VecDeque::new())),
            validator_queue: Arc::new(Mutex::new(VecDeque::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the background message listener
    pub(crate) async fn start_listener(&self) -> Result<(), String> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        // Store the shutdown sender
        {
            let mut tx_guard = self.shutdown_tx.lock().await;
            *tx_guard = Some(shutdown_tx);
        }

        let pool_owner_queue = self.pool_owner_queue.clone();
        let validator_queue = self.validator_queue.clone();

        // Spawn background task to simulate incoming p2p messages
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(5));
            let mut counter = 0u64;

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        // Mock pool owner messages
                        if counter % 2 == 0 {
                            let message = Message {
                                message_type: MessageType::PoolOwner,
                                content: serde_json::json!({
                                    "type": "inference_request",
                                    "task_id": format!("task_{}", counter),
                                    "prompt": format!("Test prompt {}", counter),
                                }),
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                            };

                            let mut queue = pool_owner_queue.lock().await;
                            queue.push_back(message);
                            log::debug!("Added mock pool owner message to queue");
                        }

                        // Mock validator messages
                        if counter % 3 == 0 {
                            let message = Message {
                                message_type: MessageType::Validator,
                                content: serde_json::json!({
                                    "type": "validation_request",
                                    "task_id": format!("validation_{}", counter),
                                }),
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                            };

                            let mut queue = validator_queue.lock().await;
                            queue.push_back(message);
                            log::debug!("Added mock validator message to queue");
                        }

                        counter += 1;
                    }
                    _ = shutdown_rx.recv() => {
                        log::info!("Message listener shutting down");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the background listener
    #[allow(unused)]
    pub(crate) async fn stop_listener(&self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(()).await;
        }
        Ok(())
    }
    /// Get the next message from the pool owner queue
    pub(crate) async fn get_pool_owner_message(&self) -> Option<PyObject> {
        let mut queue = self.pool_owner_queue.lock().await;
        queue
            .pop_front()
            .map(|msg| Python::with_gil(|py| json_to_pyobject(py, &msg.content)))
    }

    /// Get the next message from the validator queue
    pub(crate) async fn get_validator_message(&self) -> Option<PyObject> {
        let mut queue = self.validator_queue.lock().await;
        queue
            .pop_front()
            .map(|msg| Python::with_gil(|py| json_to_pyobject(py, &msg.content)))
    }

    /// Push a message to the appropriate queue (for testing or internal use)
    #[allow(unused)]
    pub(crate) async fn push_message(&self, message: Message) -> Result<(), String> {
        match message.message_type {
            MessageType::PoolOwner => {
                let mut queue = self.pool_owner_queue.lock().await;
                queue.push_back(message);
            }
            MessageType::Validator => {
                let mut queue = self.validator_queue.lock().await;
                queue.push_back(message);
            }
        }
        Ok(())
    }

    /// Get queue sizes for monitoring
    #[allow(unused)]
    pub(crate) async fn get_queue_sizes(&self) -> (usize, usize) {
        let pool_owner_size = self.pool_owner_queue.lock().await.len();
        let validator_size = self.validator_queue.lock().await.len();
        (pool_owner_size, validator_size)
    }
}
