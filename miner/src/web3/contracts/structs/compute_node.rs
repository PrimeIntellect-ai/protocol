use alloy::primitives::Address;

#[derive(Debug, Clone, PartialEq)]
pub struct ComputeNode {
    pub provider: Address,
    pub subkey: Address,
    pub specs_uri: String,
    pub compute_units: u32,   // H100 equivalents
    pub benchmark_score: u32, // some fidelity metric
    pub is_active: bool,
    pub is_validated: bool,
}
