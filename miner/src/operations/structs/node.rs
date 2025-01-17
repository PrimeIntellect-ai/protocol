use serde::{Deserialize, Serialize};
use alloy::primitives::Address;
#[derive(Serialize, Deserialize, Debug)]
pub struct ComputeSpecs {
    pub gpu: Option<GpuSpecs>,
    pub cpu: Option<CpuSpecs>,
    #[serde(rename = "ramMB")]
    pub ram_mb: Option<u32>,
    #[serde(rename = "storageGB")]
    pub storage_gb: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GpuSpecs {
    pub count: Option<u32>,
    pub model: Option<String>,
    #[serde(rename = "memoryMB")]
    pub memory_mb: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CpuSpecs {
    pub cores: Option<u32>,
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NodeConfig {
    #[serde(rename = "providerAddress")]
    pub provider_address: Option<Address>,
    #[serde(rename = "ipAddress")]
    pub ip_address: String,
    pub port: u16,
    #[serde(rename = "computeSpecs")]
    pub compute_specs: Option<ComputeSpecs>,
    #[serde(rename = "computePoolId")]
    pub compute_pool_id: u64,
}
