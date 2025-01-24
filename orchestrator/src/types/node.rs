use alloy::primitives::Address;
use serde::{Deserialize, Serialize};
use shared::models::task::TaskState;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorNode{
    #[serde(rename = "id")]
    pub address: Address,
    #[serde(rename = "ipAddress")]
    pub ip_address: String,
    pub port: u16,
    #[serde(rename = "status")]
    pub status: OrchestratorNodeStatus,

    pub task_id: Option<String>,
    pub task_state: Option<TaskState>,
}

impl OrchestratorNode {
    #[allow(dead_code)]
    pub fn new(address: Address, ip_address: String, port: u16) -> Self {
        Self {
            address,
            ip_address,
            port,
            status: OrchestratorNodeStatus::Discovered,
            task_id: None,
            task_state: None,
        }
    }

    pub fn from_string(s: &str) -> Self {
        let mut node: Self = serde_json::from_str(s).unwrap();
        if node.status == OrchestratorNodeStatus::Dead {
            node.task_id = None;
            node.task_state = None;
        }
        node
    }
}

impl fmt::Display for OrchestratorNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OrchestratorNodeStatus {
    Discovered,
    WaitingForHeartbeat,
    Healthy,
    Unhealthy,
    Dead,
}
