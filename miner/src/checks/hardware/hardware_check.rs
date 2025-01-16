use super::{
    gpu::detect_gpu,
    memory::{get_memory_info, print_memory_info},
    storage::{get_storage_info, print_storage_info},
    types::SystemCheckError,
};
use crate::operations::structs::node::{ComputeSpecs, CpuSpecs, GpuSpecs, NodeConfig};
use colored::*;
use sysinfo::{self, System};

pub struct HardwareChecker {
    sys: System,
}

impl HardwareChecker {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Self { sys }
    }

    pub fn enrich_node_config(
        &self,
        mut node_config: NodeConfig,
    ) -> Result<NodeConfig, SystemCheckError> {
        self.collect_system_info(&mut node_config)?;
        self.print_system_info(&node_config);
        println!("\n{}", "✓ All hardware checks passed".green().bold());
        Ok(node_config)
    }

    fn collect_system_info(&self, node_config: &mut NodeConfig) -> Result<(), SystemCheckError> {
        if self.sys.cpus().is_empty() {
            return Err(SystemCheckError::Other(
                "Failed to detect CPU information".to_string(),
            ));
        }

        let cpu_specs = self.collect_cpu_specs()?;
        let gpu_specs = self.collect_gpu_specs()?;
        let (ram_mb, storage_gb) = self.collect_memory_specs()?;

        node_config.compute_specs = Some(ComputeSpecs {
            cpu: Some(cpu_specs),
            ram_mb: Some(ram_mb),
            storage_gb: Some(storage_gb),
            gpu: gpu_specs,
        });

        Ok(())
    }

    fn collect_cpu_specs(&self) -> Result<CpuSpecs, SystemCheckError> {
        Ok(CpuSpecs {
            cores: Some(self.sys.cpus().len() as u32),
            model: Some(self.sys.cpus()[0].brand().to_string()),
        })
    }

    fn collect_gpu_specs(&self) -> Result<Option<GpuSpecs>, SystemCheckError> {
        Ok(detect_gpu().map(|gpu| GpuSpecs {
            count: Some(gpu.gpu_count as u32),
            model: Some(gpu.name),
            memory_mb: Some((gpu.memory / 1024) as u32), // Convert bytes to MB
        }))
    }

    fn collect_memory_specs(&self) -> Result<(u32, u32), SystemCheckError> {
        let (total_memory, _) = get_memory_info(&self.sys);
        let (total_storage, _) = get_storage_info()?;
        Ok((total_memory as u32, total_storage as u32))
    }

    fn print_system_info(&self, node_config: &NodeConfig) {
        println!("\n{}", "Hardware Requirements Check:".blue().bold());

        // Print CPU Info
        println!("\n{}", "CPU Information:".blue().bold());
        if let Some(compute_specs) = &node_config.compute_specs {
            if let Some(cpu) = &compute_specs.cpu {
                println!("  Cores: {}", cpu.cores.unwrap_or(0));
                println!(
                    "  Model: {}",
                    cpu.model.as_ref().unwrap_or(&"Unknown".to_string())
                );
            }
        }

        // Print Memory and Storage Info
        if let Some(compute_specs) = &node_config.compute_specs {
            print_memory_info(compute_specs.ram_mb.unwrap_or(0) as u64, 0);
            print_storage_info(compute_specs.storage_gb.unwrap_or(0) as u64, 0);
        }

        // Print GPU Info
        match &node_config.compute_specs {
            Some(compute_specs) => {
                if let Some(gpu) = &compute_specs.gpu {
                    println!("\n{}", "GPU Information:".blue().bold());
                    println!("  Count: {}", gpu.count.unwrap_or(0));
                    println!(
                        "  Model: {}",
                        gpu.model.as_ref().unwrap_or(&"Unknown".to_string())
                    );
                    println!("  Memory: {} MB", gpu.memory_mb.unwrap_or(0));
                } else {
                    println!(
                        "\n{}",
                        "Warning: No compatible GPU detected".yellow().bold()
                    );
                }
            }
            None => println!(
                "\n{}",
                "Warning: No compute specs available".yellow().bold()
            ),
        }
    }
}

// Public interface
pub fn run_hardware_check(node_config: NodeConfig) -> Result<NodeConfig, SystemCheckError> {
    HardwareChecker::new().enrich_node_config(node_config)
}
