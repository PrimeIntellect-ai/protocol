use serde::{Deserialize, Serialize};
use std::ops::Deref;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Node {
    pub id: String,
    pub provider_address: String,
    pub ip_address: String,
    pub port: u16,
    pub compute_pool_id: u32,
    pub compute_specs: Option<ComputeSpecs>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ComputeSpecs {
    // GPU specifications
    pub gpu: Option<GpuSpecs>,
    // CPU specifications
    pub cpu: Option<CpuSpecs>,
    // Memory and storage specifications
    pub ram_mb: Option<u32>,
    pub storage_gb: Option<u32>,
    pub storage_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GpuSpecs {
    pub count: Option<u32>,
    pub model: Option<String>,
    pub memory_mb: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CpuSpecs {
    pub cores: Option<u32>,
    pub model: Option<String>,
}

// Discover node contains validation info and is typically returned by the discovery svc

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiscoveryNode {
    #[serde(flatten)]
    pub node: Node,
    pub is_validated: bool,
    pub is_active: bool,
    #[serde(default)]
    pub is_provider_whitelisted: bool,
}

impl DiscoveryNode {
    pub fn with_updated_node(&self, new_node: Node) -> Self {
        DiscoveryNode {
            node: new_node,
            is_validated: self.is_validated,
            is_active: self.is_active,
            is_provider_whitelisted: self.is_provider_whitelisted,
        }
    }
}

impl Deref for DiscoveryNode {
    type Target = Node;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl From<Node> for DiscoveryNode {
    fn from(node: Node) -> Self {
        DiscoveryNode {
            node,
            is_validated: false, // Default values for new discovery nodes
            is_active: false,
            is_provider_whitelisted: false,
        }
    }
}
