use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Node {
    pub id: String,
    pub provider_address: Option<String>,
    pub ip_address: String,
    pub port: u16,
    pub compute_pool_id: Option<u32>,
    pub last_seen: Option<u32>,

    // Specifications for compute resources
    pub compute_specs: Option<ComputeSpecs>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ComputeSpecs {
    // GPU specifications
    pub gpu: Option<GpuSpecs>,
    // CPU specifications
    pub cpu: Option<CpuSpecs>,
    // Memory and storage specifications
    pub ram_mb: Option<u32>,
    pub storage_gb: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GpuSpecs {
    pub count: Option<u32>,
    pub model: Option<String>,
    pub memory_mb: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CpuSpecs {
    pub cores: Option<u32>,
    pub model: Option<String>,
}