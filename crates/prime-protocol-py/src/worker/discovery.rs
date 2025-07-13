use serde::{Deserialize, Serialize};

/// Simple node information for discovery upload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleNode {
    /// Unique identifier for the node
    pub id: String,
    /// The external IP address of the node
    pub ip_address: String,
    /// The port for the node (usually 0 for dynamic port allocation)
    pub port: u16,
    /// The Ethereum address of the node
    pub provider_address: String,
    /// The compute pool ID this node belongs to
    pub compute_pool_id: u32,
    /// The P2P peer ID
    pub worker_p2p_id: String,
    /// The P2P multiaddresses for connecting to this node
    pub worker_p2p_addresses: Vec<String>,
}

impl SimpleNode {
    pub fn new(
        ip_address: String,
        port: u16,
        provider_wallet_address: String,
        node_wallet_address: String,
        compute_pool_id: u32,
        peer_id: String,
        multi_addresses: Vec<String>,
    ) -> Self {
        Self {
            id: node_wallet_address,
            ip_address,
            port,
            provider_address: provider_wallet_address,
            compute_pool_id,
            worker_p2p_id: peer_id,
            worker_p2p_addresses: multi_addresses,
        }
    }

    /// Convert to serde_json::Value for upload
    pub fn to_json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "ip_address": self.ip_address,
            "port": self.port,
            "provider_address": self.provider_address,
            "compute_pool_id": self.compute_pool_id,
            "worker_p2p_id": self.worker_p2p_id,
            "worker_p2p_addresses": self.worker_p2p_addresses,
        })
    }
}
