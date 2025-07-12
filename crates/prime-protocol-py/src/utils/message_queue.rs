use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};

use crate::utils::json_parser::json_to_pyobject;

/// Generic message that can be sent between components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub content: serde_json::Value,
    pub timestamp: u64,
    pub sender: Option<String>,
}

/// Simple message queue for handling messages
#[derive(Clone)]
pub struct MessageQueue {
    queue: Arc<Mutex<VecDeque<Message>>>,
    max_size: Option<usize>,
    shutdown_tx: Arc<Mutex<Option<mpsc::Sender<()>>>>,
}

impl MessageQueue {
    /// Create a new message queue
    pub fn new(max_size: Option<usize>) -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            max_size,
            shutdown_tx: Arc::new(Mutex::new(None)),
        }
    }

    /// Push a message to the queue
    pub async fn push_message(&self, message: Message) -> Result<(), String> {
        let mut queue = self.queue.lock().await;

        // Check max size if configured
        if let Some(max_size) = self.max_size {
            if queue.len() >= max_size {
                return Err(format!("Queue is full (max size: {})", max_size));
            }
        }

        queue.push_back(message);
        Ok(())
    }

    /// Get the next message from the queue
    pub async fn get_message(&self) -> Option<PyObject> {
        let mut queue = self.queue.lock().await;

        queue
            .pop_front()
            .map(|msg| Python::with_gil(|py| json_to_pyobject(py, &msg.content)))
    }

    /// Get all messages from the queue (draining it)
    pub async fn get_all_messages(&self) -> Vec<PyObject> {
        let mut queue = self.queue.lock().await;

        let messages: Vec<Message> = queue.drain(..).collect();
        messages
            .into_iter()
            .map(|msg| Python::with_gil(|py| json_to_pyobject(py, &msg.content)))
            .collect()
    }

    /// Peek at the next message without removing it
    pub async fn peek_message(&self) -> Option<PyObject> {
        let queue = self.queue.lock().await;

        queue
            .front()
            .map(|msg| Python::with_gil(|py| json_to_pyobject(py, &msg.content)))
    }

    /// Get the size of the queue
    pub async fn get_queue_size(&self) -> usize {
        let queue = self.queue.lock().await;
        queue.len()
    }

    /// Clear the queue
    pub async fn clear(&self) -> Result<(), String> {
        let mut queue = self.queue.lock().await;
        queue.clear();
        Ok(())
    }

    /// Start a mock message listener (for testing/development)
    pub async fn start_mock_listener(&self, frequency: u64) -> Result<(), String> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        // Store the shutdown sender
        {
            let mut tx_guard = self.shutdown_tx.lock().await;
            *tx_guard = Some(shutdown_tx);
        }

        let queue_clone = self.queue.clone();

        // Spawn background task to simulate incoming messages
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(1));
            let mut counter = 0u64;

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if counter % frequency == 0 {
                            let message = Message {
                                content: serde_json::json!({
                                    "type": "mock_message",
                                    "id": format!("mock_{}", counter),
                                    "data": format!("Mock data #{}", counter),
                                }),
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap()
                                    .as_secs(),
                                sender: Some("mock_listener".to_string()),
                            };

                            let mut queue = queue_clone.lock().await;
                            queue.push_back(message);
                            log::debug!("Added mock message to queue");
                        }
                        counter += 1;
                    }
                    _ = shutdown_rx.recv() => {
                        log::info!("Mock message listener shutting down");
                        break;
                    }
                }
            }
        });

        Ok(())
    }

    /// Stop the mock listener
    pub async fn stop_listener(&self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(()).await;
        }
        Ok(())
    }
}
