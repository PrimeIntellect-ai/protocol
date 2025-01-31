use super::{
    gpu::detect_gpu,
    memory::{convert_to_mb, get_memory_info, print_memory_info},
    storage::{get_storage_info, BYTES_TO_GB},
};
use crate::console::Console;
use shared::models::node::{ComputeSpecs, CpuSpecs, GpuSpecs, Node};
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
        mut node_config: Node,
    ) -> Result<Node, Box<dyn std::error::Error>> {
        self.collect_system_info(&mut node_config)?;
        self.print_system_info(&node_config);
        Console::success("All hardware checks passed");
        Ok(node_config)
    }

    fn collect_system_info(
        &self,
        node_config: &mut Node,
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

        let (storage_path, available_space) = if cfg!(target_os = "linux") {
            match super::storage::find_largest_storage() {
                Ok(mount_point) => (Some(mount_point.path), Some(mount_point.available_space)),
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };

        let storage_gb_value = match available_space {
            Some(space) => (space as f64 / BYTES_TO_GB) as u32,
            None => storage_gb,
        };

        node_config.compute_specs = Some(ComputeSpecs {
            cpu: Some(cpu_specs),
            ram_mb: Some(ram_mb),
            storage_gb: Some(storage_gb_value),
            storage_path,
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

        // Convert bytes to MB for RAM and GB for storage
        let ram_mb = convert_to_mb(total_memory);
        let storage_gb = total_storage;

        Ok((ram_mb as u32, storage_gb as u32))
    }

    fn print_system_info(&self, node_config: &Node) {
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

        // Print Memory Info
        if let Some(compute_specs) = &node_config.compute_specs {
            let (total_memory, free_memory) = get_memory_info(&self.sys);
            print_memory_info(total_memory, free_memory);

            // Print Storage Info
            if let Some(storage_gb) = &compute_specs.storage_gb {
                Console::section("Storage Information:");
                Console::info("Total Storage", &format!("{} GB", storage_gb));
            }
            if let Some(storage_path) = &compute_specs.storage_path {
                Console::info("Storage Path for docker mounts:", &storage_path);
            }
        }

        // Print GPU Info
        if let Some(compute_specs) = &node_config.compute_specs {
            if let Some(gpu) = &compute_specs.gpu {
                Console::title("GPU Information:");
                Console::info("Count", &gpu.count.unwrap_or(0).to_string());
                Console::info(
                    "Model",
                    gpu.model.as_ref().unwrap_or(&"Unknown".to_string()),
                );
                // Convert memory from MB to GB and round

                let memory_gb = if let Some(memory_mb) = gpu.memory_mb {
                    memory_mb as f64 / 1024.0
                } else {
                    0.0
                };
                Console::info("Memory", &format!("{:.0} GB", memory_gb));
            } else {
                Console::warning("No compatible GPU detected");
            }
        } else {
            Console::warning("No compute specs available");
        }
    }
}
