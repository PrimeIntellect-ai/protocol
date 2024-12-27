use super::gpu::detect_gpu;
use super::memory::{get_memory_info, print_memory_info};
use super::storage::{get_storage_info, print_storage_info};
use super::types::{SystemCheckError, SystemInfo};
use colored::*;
use std::error::Error;
use sysinfo::{self, System};

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

impl std::fmt::Display for SystemCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::GPUDriversNotFound => write!(f, "GPU drivers not found or incompatible"),
            Self::Other(msg) => write!(f, "System error: {}", msg),
        }
    }
}

impl Error for SystemCheckError {}

pub fn run_hardware_check() -> Result<SystemInfo, SystemCheckError> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let system_info = collect_system_info(&sys)?;
    print_system_info(&system_info);

    println!("\n{}", "✓ All hardware checks passed".green().bold());
    Ok(system_info)
}

fn collect_system_info(sys: &System) -> Result<SystemInfo, SystemCheckError> {
    if sys.cpus().is_empty() {
        return Err(SystemCheckError::Other(
            "Failed to detect CPU information".to_string(),
        ));
    }

    let cpu_cores = sys.cpus().len();
    let cpu_brand = sys.cpus()[0].brand().to_string();
    let (total_memory, free_memory) = get_memory_info(sys);
    let (total_storage, free_storage) = get_storage_info()?;
    let gpu_info = detect_gpu();

    Ok(SystemInfo {
        cpu_cores,
        cpu_brand,
        total_memory,
        free_memory,
        total_storage,
        free_storage,
        gpu_info,
    })
}

fn print_system_info(info: &SystemInfo) {
    println!("\n{}", "Hardware Requirements Check:".blue().bold());
    println!("\n{}", "CPU Information:".blue().bold());
    println!("  Cores: {}", info.cpu_cores);
    println!("  Model: {}", info.cpu_brand);

    print_memory_info(info.total_memory, info.free_memory);
    print_storage_info(info.total_storage, info.free_storage);

    match &info.gpu_info {
        Some(gpu) => {
            println!("\n{}", "GPU Information:".blue().bold());
            println!("  Name: {}", gpu.name);
            println!("  Memory: {:.1} GB", gpu.memory as f64 / BYTES_TO_GB);
            println!("  CUDA Version: {}", gpu.cuda_version);
        }
        None => println!(
            "\n{}",
            "Warning: No compatible GPU detected".yellow().bold()
        ),
    }
}
