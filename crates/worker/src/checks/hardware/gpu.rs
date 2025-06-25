use crate::console::Console;
use lazy_static::lazy_static;
use nvml_wrapper::Nvml;
use shared::models::node::GpuSpecs;
use std::sync::Mutex;

#[allow(dead_code)]
const BYTES_TO_MB: u64 = 1024 * 1024;

// Use lazy_static to initialize NVML once and reuse it
lazy_static! {
    static ref NVML: Mutex<Option<Nvml>> = Mutex::new(None);
}

#[derive(Debug)]
#[allow(dead_code)]
struct GpuDevice {
    name: String,
    memory: u64,
    driver_version: String,
    count: u32,
    indices: Vec<u32>,
}

pub fn detect_gpu() -> Vec<GpuSpecs> {
    Console::title("GPU Detection");

    let gpu_devices = get_gpu_status();
    if gpu_devices.is_empty() {
        Console::user_error("No GPU devices detected");
        return vec![];
    }

    gpu_devices
        .into_iter()
        .map(|device| GpuSpecs {
            count: Some(device.count),
            model: Some(device.name.to_lowercase()),
            memory_mb: Some((device.memory / BYTES_TO_MB) as u32),
            indices: Some(device.indices),
        })
        .collect()
}

fn get_gpu_status() -> Vec<GpuDevice> {
    let mut nvml_guard = NVML.lock().unwrap();

    // Read WORKER_VISIBLE_DEVICES environment variable
    let visible_devices: Option<Vec<u32>> =
        std::env::var("WORKER_VISIBLE_DEVICES").ok().and_then(|s| {
            if s.trim().is_empty() {
                None
            } else {
                Some(
                    s.split(',')
                        .filter_map(|idx| idx.trim().parse::<u32>().ok())
                        .collect(),
                )
            }
        });

    // Initialize NVML if not already initialized
    if nvml_guard.is_none() {
        // Try to load the NVIDIA management library dynamically
        let lib_paths = [
            "libnvidia-ml.so.1",                           // Standard Linux path
            "/usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1", // Explicit path as fallback
            "/usr/lib/libnvidia-ml.so.1",                  // CUDA installation path
        ];

        let mut success = false;
        for path in lib_paths {
            match Nvml::builder().lib_path(std::ffi::OsStr::new(path)).init() {
                Ok(nvml) => {
                    *nvml_guard = Some(nvml);
                    success = true;
                    break;
                }
                Err(_) => continue,
            }
        }

        if !success {
            Console::user_error(
                "Failed to initialize NVML: could not load NVIDIA management library (libnvidia-ml.so.1)",
            );
            return vec![];
        }
    }

    let nvml = nvml_guard.as_ref().unwrap();

    // Get device count
    let device_count = match nvml.device_count() {
        Ok(count) => count as usize,
        Err(e) => {
            Console::user_error(&format!("Failed to get device count: {e}"));
            return vec![];
        }
    };

    if device_count == 0 {
        Console::user_error("No GPU devices detected");
        return vec![];
    }

    let mut device_map: std::collections::HashMap<String, GpuDevice> =
        std::collections::HashMap::new();

    for i in 0..device_count {
        let device_index = i as u32;

        // Skip this device if it's not in the visible devices list
        if let Some(ref visible) = visible_devices {
            if !visible.contains(&device_index) {
                continue;
            }
        }

        match nvml.device_by_index(i as u32) {
            Ok(device) => {
                let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
                let memory = device.memory_info().map(|m| m.total).unwrap_or(0);
                let driver_version = nvml
                    .sys_driver_version()
                    .unwrap_or_else(|_| "Unknown".to_string());

                if let Some(existing_device) = device_map.get_mut(&name) {
                    existing_device.count += 1;
                    existing_device.indices.push(i as u32);
                } else {
                    device_map.insert(
                        name.clone(),
                        GpuDevice {
                            name,
                            memory,
                            driver_version,
                            count: 1,
                            indices: vec![i as u32],
                        },
                    );
                }
            }
            Err(e) => {
                Console::user_error(&format!("Failed to get device {i}: {e}"));
            }
        }
    }

    device_map.into_values().collect()
}
