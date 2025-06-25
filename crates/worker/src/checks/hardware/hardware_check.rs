use super::{
    gpu::detect_gpu,
    interconnect::InterconnectCheck,
    memory::{convert_to_mb, get_memory_info, print_memory_info},
    storage::{get_storage_info, BYTES_TO_GB},
    storage_path::StoragePathDetector,
};
use crate::{
    checks::issue::{IssueReport, IssueType},
    console::Console,
};
use shared::models::node::{ComputeSpecs, CpuSpecs, GpuSpecs, Node};
use std::sync::Arc;
use sysinfo::{self, System};
use tokio::sync::RwLock;

pub struct HardwareChecker {
    sys: System,
    issues: Arc<RwLock<IssueReport>>,
}

impl HardwareChecker {
    pub fn new(issues: Option<Arc<RwLock<IssueReport>>>) -> Self {
        let mut sys = System::new_all();

        sys.refresh_all();
        Self {
            sys,
            issues: issues.unwrap_or_else(|| Arc::new(RwLock::new(IssueReport::new()))),
        }
    }

    pub async fn check_hardware(
        &mut self,
        node_config: Node,
        storage_path_override: Option<String>,
    ) -> Result<Node, Box<dyn std::error::Error>> {
        let mut node_config = node_config;
        self.collect_system_info(&mut node_config, storage_path_override)
            .await?;
        self.print_system_info(&node_config);
        Ok(node_config)
    }

    async fn collect_system_info(
        &mut self,
        node_config: &mut Node,
        storage_path_override: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Console::section("Hardware Checks");
        let issue_tracker = self.issues.write().await;
        if self.sys.cpus().is_empty() {
            issue_tracker.add_issue(
                IssueType::InsufficientCpu,
                "Failed to detect CPU information",
            );
            return Err(Box::new(std::io::Error::other(
                "Failed to detect CPU information",
            )));
        }

        let cpu_specs = self.collect_cpu_specs()?;
        let gpu_specs = self.collect_gpu_specs()?;
        let (ram_mb, storage_gb) = self.collect_memory_specs()?;

        // Check minimum requirements
        if cpu_specs.cores.unwrap_or(0) < 4 {
            issue_tracker.add_issue(IssueType::InsufficientCpu, "Minimum 4 CPU cores required");
        }

        if ram_mb < 8192 {
            // 8GB minimum
            issue_tracker.add_issue(IssueType::InsufficientMemory, "Minimum 8GB RAM required");
        }

        if gpu_specs.is_none() {
            issue_tracker.add_issue(IssueType::NoGpu, "No GPU detected");
        }

        // Drop the write lock before calling async method
        drop(issue_tracker);

        // Detect storage path using dedicated detector
        let storage_path_detector = StoragePathDetector::new(self.issues.clone());
        let (storage_path, available_space) = storage_path_detector
            .detect_storage_path(storage_path_override)
            .await?;

        if available_space.is_some() && available_space.unwrap() < 1000 {
            let issue_tracker = self.issues.write().await;
            issue_tracker.add_issue(
                IssueType::InsufficientStorage,
                "Minimum 1000GB storage required",
            );
        }

        let storage_gb_value = match available_space {
            Some(space) => (space as f64 / BYTES_TO_GB) as u32,
            None => storage_gb,
        };

        // Check network speeds
        Console::title("Network Speed Test:");
        Console::progress("Starting network speed test...");
        match InterconnectCheck::check_speeds().await {
            Ok((download_speed, upload_speed)) => {
                Console::info("Download Speed", &format!("{download_speed:.2} Mbps"));
                Console::info("Upload Speed", &format!("{upload_speed:.2} Mbps"));

                if download_speed < 50.0 || upload_speed < 50.0 {
                    let issue_tracker = self.issues.write().await;
                    issue_tracker.add_issue(
                        IssueType::NetworkConnectivityIssue,
                        "Network speed below recommended 50Mbps",
                    );
                }
            }
            Err(_) => {
                let issue_tracker = self.issues.write().await;
                issue_tracker.add_issue(
                    IssueType::NetworkConnectivityIssue,
                    "Failed to perform network speed test",
                );
                Console::warning("Failed to perform network speed test");
            }
        }

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
        let gpu_specs = detect_gpu();
        if gpu_specs.is_empty() {
            return Ok(None);
        }

        let main_gpu = gpu_specs
            .into_iter()
            .max_by_key(|gpu| gpu.count.unwrap_or(0));

        Ok(main_gpu)
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
        // Print CPU Info
        Console::title("CPU Information:");
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
                Console::title("Storage Information:");
                Console::info("Total Storage", &format!("{storage_gb} GB"));
            }
            Console::info(
                "Storage Path for docker mounts",
                &compute_specs.storage_path,
            );
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
                Console::info("Memory", &format!("{memory_gb:.0} GB"));
            }
        } else {
            Console::warning("No compute specs available");
        }
    }
}
