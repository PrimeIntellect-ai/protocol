use super::docker::check_docker_installed;
use super::gpu::detect_gpu;
use super::types::{SystemCheckError, SystemInfo};
use colored::*;
use std::error::Error;
use std::fs;
use std::path::Path;
use sysinfo::{self, System};

const BYTES_TO_GB: f64 = 1024.0 * 1024.0 * 1024.0;

impl std::fmt::Display for SystemCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::DockerNotInstalled => write!(f, "Docker is not installed"),
            Self::InsufficientDiskSpace => write!(f, "Insufficient disk space"),
            Self::InsufficientMemory => write!(f, "Insufficient system memory"),
            Self::GPUDriversNotFound => write!(f, "GPU drivers not found or incompatible"),
            Self::Other(msg) => write!(f, "System error: {}", msg),
        }
    }
}

impl Error for SystemCheckError {}

pub fn run_system_check() -> Result<SystemInfo, SystemCheckError> {
    check_docker_installed()?;

    let mut sys = System::new_all();
    sys.refresh_all();

    let system_info = collect_system_info(&sys)?;
    print_system_info(&system_info);

    println!("\n{}", "✓ All system checks passed".green().bold());
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

    let total_memory = sys.total_memory();
    let free_memory = sys.available_memory();

    let storage_info = get_storage_info()?;

    let gpu_info = detect_gpu();

    Ok(SystemInfo {
        cpu_cores,
        cpu_brand,
        total_memory,
        free_memory,
        total_storage: storage_info.0,
        free_storage: storage_info.1,
        gpu_info,
    })
}

fn get_storage_info() -> Result<(u64, u64), SystemCheckError> {
    let path = Path::new("/");
    let fs_stats = fs::metadata(path)
        .map_err(|e| SystemCheckError::Other(format!("Failed to get storage info: {}", e)))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let total = fs_stats.blocks() * fs_stats.blksize();
        let free = fs_stats.blocks() * fs_stats.blksize();
        Ok((total, free))
    }

    #[cfg(not(unix))]
    {
        Err(SystemCheckError::Other(
            "Storage detection not supported on this platform".to_string(),
        ))
    }
}

fn print_system_info(info: &SystemInfo) {
    println!("\n{}", "System Requirements Check:".blue().bold());

    println!("\n{}", "CPU Information:".blue().bold());
    println!("  Cores: {}", info.cpu_cores);
    println!("  Model: {}", info.cpu_brand);

    println!("\n{}", "Memory Information:".blue().bold());
    println!(
        "  Total Memory: {:.1} GB",
        info.total_memory as f64 / BYTES_TO_GB
    );
    println!(
        "  Free Memory: {:.1} GB",
        info.free_memory as f64 / BYTES_TO_GB
    );

    println!("\n{}", "Storage Information:".blue().bold());
    println!(
        "  Total Storage: {:.1} GB",
        info.total_storage as f64 / BYTES_TO_GB
    );
    println!(
        "  Free Storage: {:.1} GB",
        info.free_storage as f64 / BYTES_TO_GB
    );

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
