use crate::console::Console;
use shared::models::node::{GpuSpecs, GpuVendor};
use std::fmt::Debug;

pub mod amd;
pub mod nvidia;

pub const BYTES_TO_MB: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GpuDevice {
    pub name: String,
    pub memory: u64,
    pub driver_version: String,
    pub count: u32,
    pub indices: Vec<u32>,
    pub vendor: GpuVendor,
}

/// Trait for GPU detection implementations
pub trait GpuDetector: Debug {
    /// Check if this detector is available on the system
    fn is_available(&self) -> bool;

    /// Detect GPUs using this implementation
    fn detect(&self) -> Vec<GpuDevice>;
}

/// Main function to detect GPUs from any vendor
pub fn detect_gpu() -> Vec<GpuSpecs> {
    Console::title("GPU Detection");

    let mut all_devices = Vec::new();

    // Try Nvidia detection
    let nvidia_detector = nvidia::NvidiaGpuDetector::new();
    if nvidia_detector.is_available() {
        Console::info("Detecting NVIDIA GPUs", "");
        let nvidia_devices = nvidia_detector.detect();
        Console::info("NVIDIA GPUs found", &nvidia_devices.len().to_string());
        all_devices.extend(nvidia_devices);
    }

    // Try AMD detection only if no NVIDIA GPUs were found
    if all_devices.is_empty() {
        let amd_detector = amd::AmdGpuDetector::new();
        if amd_detector.is_available() {
            Console::info("Detecting AMD GPUs", "");
            let amd_devices = amd_detector.detect();
            Console::info("AMD GPUs found", &amd_devices.len().to_string());
            all_devices.extend(amd_devices);
        }
    }

    if all_devices.is_empty() {
        Console::user_error("No GPU devices detected from any vendor");
        return vec![];
    }

    // Convert to GpuSpecs
    all_devices
        .into_iter()
        .map(|device| GpuSpecs {
            count: Some(device.count),
            model: Some(device.name.to_lowercase()),
            memory_mb: Some((device.memory / BYTES_TO_MB) as u32),
            indices: Some(device.indices),
            vendor: Some(device.vendor),
        })
        .collect()
}
