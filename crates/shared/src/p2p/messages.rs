use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// P2P message types for validator-worker communication
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum P2PMessage {
    /// Simple ping message for connectivity testing
    Ping { timestamp: SystemTime, nonce: u64 },

    /// Response to ping
    Pong { timestamp: SystemTime, nonce: u64 },
}

/// P2P request wrapper with ID for tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PRequest {
    pub id: String,
    pub message: P2PMessage,
}

/// P2P response wrapper with request ID
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PResponse {
    pub request_id: String,
    pub message: P2PMessage,
}

impl P2PRequest {
    pub fn new(message: P2PMessage) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            message,
        }
    }
}

impl P2PResponse {
    pub fn new(request_id: String, message: P2PMessage) -> Self {
        Self {
            request_id,
            message,
        }
    }
}
