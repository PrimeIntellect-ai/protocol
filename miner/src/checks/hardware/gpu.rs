use super::types::GpuInfo;
use colored::*;
use nvml_wrapper::Nvml;

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

enum GpuDevice {
    Available {
        name: String,
        memory: u64,
        driver_version: String,
        device_count: usize,
    },
    NotAvailable(String),
}

pub fn detect_gpu() -> Option<GpuInfo> {
    match get_gpu_status() {
        GpuDevice::Available {
            name,
            memory,
            driver_version,
            device_count,
        } => {
            println!("Successfully collected GPU information:");
            println!("  Name: {}", name);
            println!("  Memory: {} bytes", memory);
            println!("  Driver Version: {}", driver_version);
            println!("  Device Count: {}", device_count);

            Some(GpuInfo {
                name: name
                    .to_lowercase()
                    .split_whitespace()
                    .collect::<Vec<&str>>()
                    .join("_"),
                memory,
                cuda_version: driver_version,
                gpu_count: device_count,
            })
        }
        GpuDevice::NotAvailable(err) => {
            println!("GPU not available: {}", err);
            None
        }
    }
}

fn get_gpu_status() -> GpuDevice {
    match Nvml::init() {
        Ok(nvml) => {
            // Get device count first
            let device_count = match nvml.device_count() {
                Ok(count) => count as usize,
                Err(e) => {
                    return GpuDevice::NotAvailable(format!("Failed to get device count: {}", e))
                }
            };

            if device_count == 0 {
                return GpuDevice::NotAvailable("No GPU devices detected".to_string());
            }

            // Get first device
            match nvml.device_by_index(0) {
                Ok(device) => {
                    let name = device.name().unwrap_or_else(|_| "Unknown".to_string());

                    let memory = device.memory_info().map(|m| m.total).unwrap_or(0);

                    let driver_version = nvml
                        .sys_driver_version()
                        .unwrap_or_else(|_| "Unknown".to_string());

                    GpuDevice::Available {
                        name,
                        memory,
                        driver_version,
                        device_count,
                    }
                }
                Err(e) => GpuDevice::NotAvailable(format!("Failed to get device: {}", e)),
            }
        }
        Err(e) => GpuDevice::NotAvailable(format!("Failed to initialize NVML: {}", e)),
    }
}

pub fn print_gpu_info(gpu_info: &GpuInfo) {
    println!("\n{}", "GPU Information:".blue().bold());
    println!("  Model: {}", gpu_info.name);
    println!("  Count: {}", gpu_info.gpu_count);
    println!("  Memory: {:.1} GB", gpu_info.memory as f64 / BYTES_TO_GB);
    println!("  Driver Version: {}", gpu_info.cuda_version);
}
