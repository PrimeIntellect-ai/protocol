use alloy::primitives::Address;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shared::models::heartbeat::TaskDetails;
use shared::models::node::{ComputeSpecs, DiscoveryNode, NodeLocation};
use shared::models::task::TaskState;
use std::fmt::{self, Display};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct OrchestratorNode {
    #[serde(serialize_with = "serialize_address")]
    #[schema(value_type = String, example = "0x742d35Cc6634C0532925a3b8D6Ac6f29d1e6b8e0")]
    pub address: Address,
    pub ip_address: String,
    pub port: u16,
    pub status: NodeStatus,

    pub task_id: Option<String>,
    pub task_state: Option<TaskState>,
    #[serde(default)]
    pub task_details: Option<TaskDetails>,
    pub version: Option<String>,
    pub p2p_id: Option<String>,
    pub last_status_change: Option<DateTime<Utc>>,
    #[serde(default)]
    pub first_seen: Option<DateTime<Utc>>,

    #[serde(default)]
    pub compute_specs: Option<ComputeSpecs>,
    #[serde(default)]
    pub worker_p2p_id: Option<String>,
    #[serde(default)]
    pub worker_p2p_addresses: Option<Vec<String>>,
    #[serde(default)]
    pub location: Option<NodeLocation>,
}

fn serialize_address<S>(address: &Address, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&address.to_string())
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
            p2p_id: None,
            last_status_change: None,
            first_seen: None,
            task_details: None,
            compute_specs: discovery_node.compute_specs.clone(),
            worker_p2p_id: discovery_node.worker_p2p_id.clone(),
            worker_p2p_addresses: discovery_node.worker_p2p_addresses.clone(),
            location: discovery_node.location.clone(),
        }
    }
}

impl fmt::Display for OrchestratorNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", serde_json::to_string(self).unwrap())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default, ToSchema)]
pub enum NodeStatus {
    #[default]
    Discovered,
    WaitingForHeartbeat,
    Healthy,
    Unhealthy,
    Dead,
    Ejected,
    Banned,
}

impl Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
