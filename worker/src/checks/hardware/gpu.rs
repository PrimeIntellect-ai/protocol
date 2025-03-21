use crate::console::Console;
use lazy_static::lazy_static;
use nvml_wrapper::Nvml;
use shared::models::node::GpuSpecs;
use std::sync::Mutex;

#[allow(dead_code)]
const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

// Use lazy_static to initialize NVML once and reuse it
lazy_static! {
    static ref NVML: Mutex<Option<Nvml>> = Mutex::new(None);
}

#[allow(dead_code)]
enum GpuDevice {
    Available {
        name: String,
        memory: u64,
        driver_version: String,
        device_count: usize,
    },
    NotAvailable(String),
}

pub fn detect_gpu() -> Option<GpuSpecs> {
    Console::title("GPU Detection");
    // Changed return type to GpuSpecs
    match get_gpu_status() {
        GpuDevice::Available {
            name,
            memory,
            driver_version: _,
            device_count,
        } => Some(GpuSpecs {
            // Create GpuSpecs directly
            count: Some(device_count as u32),
            model: Some(
                name.to_lowercase()
                    .split_whitespace()
                    .collect::<Vec<&str>>()
                    .join("_"),
            ),
            memory_mb: Some((memory / 1024 / 1024) as u32), // Convert bytes to MB
        }),
        GpuDevice::NotAvailable(_) => {
            //println!("GPU not available: {}", err);
            Console::error("GPU not available");
            None
        }
    }
}

fn get_gpu_status() -> GpuDevice {
    let mut nvml_guard = NVML.lock().unwrap();

    // Initialize NVML if not already initialized
    if nvml_guard.is_none() {
        match Nvml::builder()
            .lib_path(std::ffi::OsStr::new(
                "/usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1",
            ))
            .init()
        {
            Ok(nvml) => *nvml_guard = Some(nvml),
            Err(e) => return GpuDevice::NotAvailable(format!("Failed to initialize NVML: {}", e)),
        }
    }

    let nvml = nvml_guard.as_ref().unwrap();

    // Get device count
    let device_count = match nvml.device_count() {
        Ok(count) => count as usize,
        Err(e) => return GpuDevice::NotAvailable(format!("Failed to get device count: {}", e)),
    };

    if device_count == 0 {
        return GpuDevice::NotAvailable("No GPU devices detected".to_string());
    }

    // Get first device info
    // TODO: Get all devices
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
