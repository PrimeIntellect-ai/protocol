use alloy::primitives::Address;
use redis::{ErrorKind, FromRedisValue, RedisError, RedisResult, RedisWrite, ToRedisArgs, Value};
use serde::{Deserialize, Serialize};
use shared::models::task::Task;
use shared::models::task::TaskRequest;
use std::fmt;
use uuid::Uuid;

pub const ORCHESTRATOR_BASE_KEY: &str = "orchestrator:node:";
pub const ORCHESTRATOR_HEARTBEAT_KEY: &str = "orchestrator:heartbeat:";
pub const ORCHESTRATOR_UNHEALTHY_COUNTER_KEY: &str = "orchestrator:unhealthy_counter:";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "id")]
    pub address: Address,
    #[serde(rename = "ipAddress")]
    pub ip_address: String,
    pub port: u16,
    #[serde(rename = "status")]
    pub status: NodeStatus,
}

impl Node {
    #[allow(dead_code)]
    pub fn new(address: Address, ip_address: String, port: u16) -> Self {
        Self {
            address,
            ip_address,
            port,
            status: NodeStatus::Discovered,
        }
    }

    pub fn orchestrator_key(&self) -> String {
        format!("{}:{}", ORCHESTRATOR_BASE_KEY, self.address)
    }

    #[allow(dead_code)]
    pub fn heartbeat_key(&self) -> String {
        format!("orchestrator:heartbeat:{}", self.address)
    }

    pub fn from_string(s: &str) -> Self {
        serde_json::from_str(s).unwrap()
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
