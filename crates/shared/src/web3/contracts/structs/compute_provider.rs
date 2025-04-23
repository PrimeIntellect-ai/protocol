use super::compute_node::ComputeNode;
use alloy::primitives::Address;

pub struct ComputeProvider {
    pub provider_address: Address,
    pub is_whitelisted: bool,
    pub active_nodes: u32,
    pub nodes: Vec<ComputeNode>,
}
