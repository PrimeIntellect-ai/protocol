use alloy::primitives::Address;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::models::node::DiscoveryNode;
use shared::models::task::TaskState;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorNode {
    pub address: Address,
    pub ip_address: String,
    pub port: u16,
    pub status: NodeStatus,

    pub task_id: Option<String>,
    pub task_state: Option<TaskState>,
    pub version: Option<String>,
    pub last_status_change: Option<DateTime<Utc>>,
}

impl From<DiscoveryNode> for OrchestratorNode {
    fn from(discovery_node: DiscoveryNode) -> Self {
        Self {
            address: discovery_node.id.parse().unwrap(),
            ip_address: discovery_node.ip_address.clone(),
            port: discovery_node.port,
            status: NodeStatus::Discovered,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        }
    }
}

impl OrchestratorNode {
    pub fn from_string(s: &str) -> Self {
        let mut node: Self = serde_json::from_str(s).unwrap();
        if node.status == NodeStatus::Dead || node.status == NodeStatus::Ejected {
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
pub enum NodeStatus {
    Discovered,
    WaitingForHeartbeat,
    Healthy,
    Unhealthy,
    Dead,
    Ejected,
}
