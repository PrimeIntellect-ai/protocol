use super::{
    gpu::detect_gpu,
    memory::{get_memory_info, print_memory_info},
    storage::get_storage_info,
};
use crate::console::Console; // Import Console for logging
use crate::operations::structs::node::{ComputeSpecs, CpuSpecs, GpuSpecs, NodeConfig};
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
    ) -> Result<NodeConfig, Box<dyn std::error::Error>> {
        self.collect_system_info(&mut node_config)?;
        self.print_system_info(&node_config);
        Console::success("All hardware checks passed");
        Ok(node_config)
    }

    fn collect_system_info(
        &self,
        node_config: &mut NodeConfig,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.sys.cpus().is_empty() {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to detect CPU information",
            )));
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

    fn collect_cpu_specs(&self) -> Result<CpuSpecs, Box<dyn std::error::Error>> {
        Ok(CpuSpecs {
            cores: Some(self.sys.cpus().len() as u32),
            model: Some(self.sys.cpus()[0].brand().to_string()),
        })
    }

    fn collect_gpu_specs(&self) -> Result<Option<GpuSpecs>, Box<dyn std::error::Error>> {
        Ok(detect_gpu().map(|gpu| GpuSpecs {
            count: Some(gpu.count.unwrap_or(0)),
            model: gpu.model,
            memory_mb: gpu.memory_mb,
        }))
    }

    fn collect_memory_specs(&self) -> Result<(u32, u32), Box<dyn std::error::Error>> {
        let (total_memory, _) = get_memory_info(&self.sys);
        let (total_storage, _) = get_storage_info()?;
        Ok((total_memory as u32, total_storage as u32))
    }

    fn print_system_info(&self, node_config: &NodeConfig) {
        Console::section("Hardware Requirements Check");

        // Print CPU Info
        Console::section("CPU Information:");
        if let Some(compute_specs) = &node_config.compute_specs {
            if let Some(cpu) = &compute_specs.cpu {
                Console::info("Cores", &cpu.cores.unwrap_or(0).to_string());
                Console::info(
                    "Model",
                    cpu.model.as_ref().unwrap_or(&"Unknown".to_string()),
                );
            }
        }

        // Print Memory and Storage Info
        if let Some(compute_specs) = &node_config.compute_specs {
            print_memory_info(compute_specs.ram_mb.unwrap_or(0) as u64, 0);
            // TODO: Print sotrage info
        }

        // Print GPU Info
        match &node_config.compute_specs {
            Some(compute_specs) => {
                if let Some(gpu) = &compute_specs.gpu {
                    Console::title("GPU Information:");
                    Console::info("Count", &gpu.count.unwrap_or(0).to_string());
                    Console::info(
                        "Model",
                        gpu.model.as_ref().unwrap_or(&"Unknown".to_string()),
                    );
                    Console::info("Memory", &gpu.memory_mb.unwrap_or(0).to_string());
                } else {
                    Console::warning("No compatible GPU detected");
                }
            }
            None => Console::warning("No compute specs available"),
        }
    }
}
