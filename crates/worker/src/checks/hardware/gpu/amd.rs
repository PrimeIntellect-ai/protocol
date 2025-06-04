use super::{GpuDevice, GpuDetector};
use shared::models::node::GpuVendor;

#[cfg(feature = "amd-gpu")]
use crate::console::Console;
#[cfg(feature = "amd-gpu")]
use rocm_smi_lib::RocmSmi;

#[derive(Debug)]
pub struct AmdGpuDetector;

impl AmdGpuDetector {
    pub fn new() -> Self {
        Self
    }
}

impl GpuDetector for AmdGpuDetector {
    fn is_available(&self) -> bool {
        #[cfg(feature = "amd-gpu")]
        {
            // Check if ROCm SMI can be initialized
            match RocmSmi::init() {
                Ok(_) => true,
                Err(_) => false,
            }
        }
        
        #[cfg(not(feature = "amd-gpu"))]
        {
            false
        }
    }
    
    fn detect(&self) -> Vec<GpuDevice> {
        #[cfg(feature = "amd-gpu")]
        {
            get_amd_gpu_status()
        }
        
        #[cfg(not(feature = "amd-gpu"))]
        {
            vec![]
        }
    }
    
    fn vendor(&self) -> GpuVendor {
        GpuVendor::Amd
    }
}

#[cfg(feature = "amd-gpu")]
fn get_amd_gpu_status() -> Vec<GpuDevice> {
    let mut rocm = match RocmSmi::init() {
        Ok(smi) => smi,
        Err(e) => {
            Console::user_error(&format!("Failed to initialize ROCm SMI: {:?}", e));
            return vec![];
        }
    };

    // Get device count - it returns u32 directly, not a Result
    let device_count = rocm.get_device_count() as usize;

    if device_count == 0 {
        return vec![];
    }

    let mut device_map: std::collections::HashMap<String, GpuDevice> =
        std::collections::HashMap::new();

    for i in 0..device_count {
        // Get device identifiers
        let identifiers = match rocm.get_device_identifiers(i as u32) {
            Ok(id) => id,
            Err(e) => {
                Console::user_error(&format!("Failed to get AMD device {} identifiers: {:?}", i, e));
                continue;
            }
        };

        // Get memory info using get_device_memory_data
        let memory = match rocm.get_device_memory_data(i as u32) {
            Ok(mem_data) => mem_data.vram_total,
            Err(_) => 0,
        };

        // Get driver version - use get_rsmi_version which returns a String
        let driver_version = match rocm.get_rsmi_version() {
            Ok(version) => format!("ROCm {}", version),
            Err(_) => "Unknown".to_string(),
        };

        // The name from identifiers is a Result<String, RocmErr>
        let name = match identifiers.name {
            Ok(n) => n,
            Err(e) => {
                Console::user_error(&format!("Failed to get device {} name: {:?}", i, e));
                continue;
            }
        };

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
                    vendor: GpuVendor::Amd,
                },
            );
        }
    }

    device_map.into_values().collect()
}
