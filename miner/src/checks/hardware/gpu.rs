use super::types::GpuInfo;
use colored::*;
use nvml_wrapper::Nvml;

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

pub fn detect_gpu() -> Option<GpuInfo> {
    let nvml = Nvml::init().ok()?;
    let device_count = nvml.device_count().ok().filter(|&count| count > 0)?;
    let device = nvml.device_by_index(0).ok()?;

    let name = device
        .name()
        .ok()?
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join("_");

    let memory = device.memory_info().ok()?;
    let driver_version = nvml.sys_driver_version().ok()?;

    Some(GpuInfo {
        name,
        memory: memory.total,
        cuda_version: driver_version,
        gpu_count: device_count as usize,
    })
}

pub fn print_gpu_info(gpu_info: &GpuInfo) {
    println!("\n{}", "GPU Information:".blue().bold());
    println!("  Model: {}", gpu_info.name);
    println!("  Count: {}", gpu_info.gpu_count);
    println!("  Memory: {:.1} GB", gpu_info.memory as f64 / BYTES_TO_GB);
    println!("  Driver Version: {}", gpu_info.cuda_version);
}
