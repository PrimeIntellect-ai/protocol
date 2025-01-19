use alloy::primitives::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "id")]
    pub address: Address,
    #[serde(rename = "ipAddress")]
    pub ip_address: String,
    pub port: u16,
    #[serde(rename = "status")]
    pub status: Option<NodeStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeStatus {
    Discovered,
    Invited,
}
