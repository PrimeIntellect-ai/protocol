use crate::models::challenge::{ChallengeRequest, ChallengeResponse};
use crate::models::invite::InviteRequest;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

/// Maximum message size for P2P communication (1MB)
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;

/// P2P message types for validator-worker communication
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload")]
pub enum P2PMessage {
    /// Request auth challenge from worker to validator
    RequestAuthChallenge { message: String },

    /// Auth challenge from worker to validator
    AuthChallenge {
        signed_message: String,
        message: String,
    },

    /// Auth solution from validator to worker
    AuthSolution { signed_message: String },

    /// Auth granted from worker to validator
    AuthGranted {},

    /// Auth rejected from validator to worker
    AuthRejected {},

    /// Simple ping message for connectivity testing
    Ping { timestamp: SystemTime, nonce: u64 },

    /// Response to ping
    Pong { timestamp: SystemTime, nonce: u64 },

    /// Hardware challenge from validator to worker
    HardwareChallenge {
        challenge: ChallengeRequest,
        timestamp: SystemTime,
    },

    /// Hardware challenge response from worker to validator
    HardwareChallengeResponse {
        response: ChallengeResponse,
        timestamp: SystemTime,
    },

    /// Invite request from orchestrator to worker
    Invite(InviteRequest),

    /// Response to invite
    InviteResponse {
        status: String,
        error: Option<String>,
    },

    /// Get task logs from worker
    GetTaskLogs,

    /// Response with task logs
    GetTaskLogsResponse { logs: Result<Vec<String>, String> },

    /// Restart task on worker
    RestartTask,

    /// Response to restart task
    RestartTaskResponse { result: Result<(), String> },
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
