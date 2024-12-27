use sysinfo::{self, System};

#[derive(Debug)]
pub struct SystemInfo {
    pub cpu_cores: usize,
    pub cpu_brand: String,
    pub total_memory: u64,  // In bytes
    pub free_memory: u64,   // In bytes
    pub total_storage: u64, // In bytes
    pub free_storage: u64,  // In bytes
    pub gpu_info: Option<GpuInfo>,
}

#[derive(Debug)]
pub struct GpuInfo {
    pub name: String,
    pub memory: u64, // In bytes
    pub cuda_version: String,
}

#[derive(Debug)]
pub enum SystemCheckError {
    DockerNotInstalled,
    InsufficientDiskSpace,
    InsufficientMemory,
    GPUDriversNotFound,
    Other(String),
}
