use alloy::primitives::Address;
use serde::{Deserialize, Serialize};
use shared::models::task::TaskState;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "id")]
    pub address: Address,
    #[serde(rename = "ipAddress")]
    pub ip_address: String,
    pub port: u16,
    #[serde(rename = "status")]
    pub status: NodeStatus,

    pub task_id: Option<String>,
    pub task_state: Option<TaskState>,
}

impl Node {
    #[allow(dead_code)]
    pub fn new(address: Address, ip_address: String, port: u16) -> Self {
        Self {
            address,
            ip_address,
            port,
            status: NodeStatus::Discovered,
            task_id: None,
            task_state: None,
        }
    }

    pub fn from_string(s: &str) -> Self {
        let mut node: Self = serde_json::from_str(s).unwrap();
        if node.status == NodeStatus::Dead {
            node.task_id = None;
            node.task_state = None;
        }
        node
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Discovered,
    WaitingForHeartbeat,
    Healthy,
    Unhealthy,
    Dead,
}
