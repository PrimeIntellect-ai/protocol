use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ComputeSpecs {
    pub gpu: Option<GpuSpecs>,
    pub cpu: Option<CpuSpecs>,
    pub ram_mb: Option<u32>,
    pub storage_gb: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct GpuSpecs {
    pub count: Option<u32>,
    pub model: Option<String>,
    pub memory_mb: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct CpuSpecs {
    pub cores: Option<u32>,
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    pub ip_address: String,
    pub port: u32,
    pub compute_specs: Option<ComputeSpecs>,
    pub compute_pool_id: u64,
}
