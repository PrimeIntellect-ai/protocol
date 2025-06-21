use anyhow::anyhow;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct Node {
    pub id: String,
    pub provider_address: String,
    pub ip_address: String,
    pub port: u16,
    pub compute_pool_id: u32,
    pub compute_specs: Option<ComputeSpecs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_p2p_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_p2p_addresses: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ComputeSpecs {
    // GPU specifications
    pub gpu: Option<GpuSpecs>,
    // CPU specifications
    pub cpu: Option<CpuSpecs>,
    // Memory and storage specifications
    pub ram_mb: Option<u32>,
    pub storage_gb: Option<u32>,
    pub storage_path: String,
}

impl Default for ComputeSpecs {
    fn default() -> Self {
        Self {
            gpu: None,
            cpu: None,
            ram_mb: None,
            storage_gb: None,
            storage_path: "/var/lib/prime-worker".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct ComputeRequirements {
    // List of alternative GPU requirements (OR logic)
    pub gpu: Vec<GpuRequirements>,
    pub cpu: Option<CpuSpecs>,
    pub ram_mb: Option<u32>,
    pub storage_gb: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GpuRequirements {
    pub count: Option<u32>,
    pub model: Option<String>,
    // per Card
    pub memory_mb: Option<u32>,
    pub memory_mb_min: Option<u32>,
    pub memory_mb_max: Option<u32>,
    // System wide GPU memory per gpu type
    pub total_memory_min: Option<u32>,
    pub total_memory_max: Option<u32>,
    pub indices: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct GpuSpecs {
    pub count: Option<u32>,
    pub model: Option<String>,
    pub memory_mb: Option<u32>,
    pub indices: Option<Vec<u32>>,
}

impl fmt::Display for ComputeRequirements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.gpu.is_empty() {
            writeln!(f, "GPU Requirements (any of the following):")?;
            for (i, gpu) in self.gpu.iter().enumerate() {
                writeln!(f, "  Option {}: {}", i + 1, gpu)?;
            }
        }

        if let Some(cpu) = &self.cpu {
            writeln!(f, "CPU: {}", cpu)?;
        }

        if let Some(ram) = self.ram_mb {
            writeln!(f, "RAM: {} MB", ram)?;
        }

        if let Some(storage) = self.storage_gb {
            writeln!(f, "Storage: {} GB", storage)?;
        }

        Ok(())
    }
}

impl fmt::Display for GpuRequirements {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();

        if let Some(count) = self.count {
            parts.push(format!("{} GPU(s)", count));
        }

        if let Some(model) = &self.model {
            parts.push(format!("Model: {}", model));
        }

        if let Some(memory) = self.memory_mb {
            parts.push(format!("Memory: {} MB", memory));
        }

        if parts.is_empty() {
            write!(f, "No specific GPU requirements")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

impl fmt::Display for GpuSpecs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();

        if let Some(count) = self.count {
            parts.push(format!("{} GPU(s)", count));
        }

        if let Some(model) = &self.model {
            parts.push(format!("Model: {}", model));
        }

        if let Some(memory) = self.memory_mb {
            parts.push(format!("Memory: {} MB", memory));
        }

        if parts.is_empty() {
            write!(f, "No specific GPU requirements")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct CpuSpecs {
    pub cores: Option<u32>,
    pub model: Option<String>,
}

impl fmt::Display for CpuSpecs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();

        if let Some(cores) = self.cores {
            parts.push(format!("{} cores", cores));
        }

        if let Some(model) = &self.model {
            parts.push(format!("Model: {}", model));
        }

        if parts.is_empty() {
            write!(f, "No specific CPU requirements")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

// Parser for compute requirements string
impl FromStr for ComputeRequirements {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut requirements = ComputeRequirements::default();
        let mut current_gpu_spec = GpuRequirements::default();
        let mut gpu_spec_started = false;

        for part in s.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let parts: Vec<&str> = part.splitn(2, '=').collect();
            if parts.len() != 2 {
                return Err(anyhow!("Invalid key-value pair format: '{}'", part));
            }

            let key = parts[0].trim();
            let value = parts[1].trim();

            match key {
                // --- GPU Specifications ---
                "gpu:count" => {
                    // If we have a complete GPU spec, push it before starting a new one
                    if gpu_spec_started && current_gpu_spec.count.is_some() {
                        requirements.gpu.push(current_gpu_spec);
                        current_gpu_spec = GpuRequirements::default();
                    }

                    gpu_spec_started = true;
                    current_gpu_spec.count = Some(
                        value
                            .parse::<u32>()
                            .map_err(|e| anyhow!("Invalid gpu:count value '{}': {}", value, e))?,
                    );
                }
                "gpu:model" => {
                    if !gpu_spec_started {
                        gpu_spec_started = true;
                    }
                    current_gpu_spec.model = Some(value.to_string());
                }
                "gpu:memory_mb" => {
                    if !gpu_spec_started {
                        gpu_spec_started = true;
                    }
                    if current_gpu_spec.memory_mb_min.is_some()
                        || current_gpu_spec.memory_mb_max.is_some()
                    {
                        return Err(anyhow!(
                            "Cannot specify both exact memory and min/max memory"
                        ));
                    }
                    current_gpu_spec.memory_mb =
                        Some(value.parse::<u32>().map_err(|e| {
                            anyhow!("Invalid gpu:memory_mb value '{}': {}", value, e)
                        })?);
                }
                "gpu:memory_mb_min" => {
                    if !gpu_spec_started {
                        gpu_spec_started = true;
                    }
                    if current_gpu_spec.memory_mb.is_some() {
                        return Err(anyhow!(
                            "Cannot specify both exact memory and min/max memory"
                        ));
                    }

                    if current_gpu_spec.memory_mb_max.is_some()
                        && current_gpu_spec.memory_mb_max.unwrap() < value.parse::<u32>().unwrap()
                    {
                        return Err(anyhow!(
                            "Invalid gpu:memory_mb_min value '{}': {}",
                            value,
                            "min value is greater than max value"
                        ));
                    }

                    current_gpu_spec.memory_mb_min = Some(value.parse::<u32>().map_err(|e| {
                        anyhow!("Invalid gpu:memory_mb_min value '{}': {}", value, e)
                    })?);
                }
                "gpu:memory_mb_max" => {
                    if !gpu_spec_started {
                        gpu_spec_started = true;
                    }
                    // Ensure we don't have exact memory already set
                    if current_gpu_spec.memory_mb.is_some() {
                        return Err(anyhow!(
                            "Cannot specify both exact memory and min/max memory"
                        ));
                    }

                    if current_gpu_spec.memory_mb_min.is_some()
                        && current_gpu_spec.memory_mb_min.unwrap() > value.parse::<u32>().unwrap()
                    {
                        return Err(anyhow!(
                            "Invalid gpu:memory_mb_max value '{}': {}",
                            value,
                            "max value is less than min value"
                        ));
                    }

                    current_gpu_spec.memory_mb_max = Some(value.parse::<u32>().map_err(|e| {
                        anyhow!("Invalid gpu:memory_mb_max value '{}': {}", value, e)
                    })?);
                }
                // --- Total GPU Memory Specifications ---
                "gpu:total_memory_min" => {
                    if !gpu_spec_started {
                        gpu_spec_started = true;
                    }
                    if current_gpu_spec.total_memory_max.is_some()
                        && current_gpu_spec.total_memory_max.unwrap()
                            < value.parse::<u32>().unwrap()
                    {
                        return Err(anyhow!(
                            "Invalid gpu:total_memory_min value '{}': {}",
                            value,
                            "min value is greater than max value"
                        ));
                    }

                    current_gpu_spec.total_memory_min =
                        Some(value.parse::<u32>().map_err(|e| {
                            anyhow!("Invalid gpu:total_memory_min value '{}': {}", value, e)
                        })?);
                }
                "gpu:total_memory_max" => {
                    if !gpu_spec_started {
                        gpu_spec_started = true;
                    }
                    if current_gpu_spec.total_memory_min.is_some()
                        && current_gpu_spec.total_memory_min.unwrap()
                            > value.parse::<u32>().unwrap()
                    {
                        return Err(anyhow!(
                            "Invalid gpu:total_memory_max value '{}': {}",
                            value,
                            "max value is less than min value"
                        ));
                    }

                    current_gpu_spec.total_memory_max =
                        Some(value.parse::<u32>().map_err(|e| {
                            anyhow!("Invalid gpu:total_memory_max value '{}': {}", value, e)
                        })?);
                }
                // --- CPU Specifications ---
                "cpu:cores" => {
                    let mut cpu = requirements.cpu.take().unwrap_or_default();
                    cpu.cores = Some(
                        value
                            .parse::<u32>()
                            .map_err(|e| anyhow!("Invalid cpu:cores value '{}': {}", value, e))?,
                    );
                    requirements.cpu = Some(cpu);
                }

                // --- Memory and Storage ---
                "ram_mb" => {
                    requirements.ram_mb = Some(
                        value
                            .parse::<u32>()
                            .map_err(|e| anyhow!("Invalid ram_mb value '{}': {}", value, e))?,
                    );
                }
                "storage_gb" => {
                    requirements.storage_gb = Some(
                        value
                            .parse::<u32>()
                            .map_err(|e| anyhow!("Invalid storage_gb value '{}': {}", value, e))?,
                    );
                }
                _ => return Err(anyhow!("Unknown requirement key: '{}'", key)),
            }
        }

        // Push the last GPU spec if it exists and has any properties set
        if gpu_spec_started
            && (current_gpu_spec.count.is_some()
                || current_gpu_spec.model.is_some()
                || current_gpu_spec.memory_mb.is_some()
                || current_gpu_spec.memory_mb_min.is_some()
                || current_gpu_spec.memory_mb_max.is_some()
                || current_gpu_spec.total_memory_min.is_some()
                || current_gpu_spec.total_memory_max.is_some())
        {
            requirements.gpu.push(current_gpu_spec);
        }

        Ok(requirements)
    }
}
use log::{debug, info};

impl ComputeSpecs {
    /// Checks if the current compute specs meet the given requirements.
    pub fn meets(&self, requirements: &ComputeRequirements) -> bool {
        // Check CPU (if required)
        if let Some(req_cpu) = &requirements.cpu {
            if !self
                .cpu
                .as_ref()
                .is_some_and(|spec_cpu| spec_cpu.meets(req_cpu))
            {
                info!(
                    "CPU requirements not met: required {:?}, have {:?}",
                    req_cpu, self.cpu
                );
                return false;
            }
        }

        // Check RAM (if required)
        if let Some(req_ram) = requirements.ram_mb {
            if self.ram_mb.is_none_or(|spec_ram| spec_ram < req_ram) {
                info!(
                    "RAM requirements not met: required {} MB, have {:?} MB",
                    req_ram, self.ram_mb
                );
                return false;
            }
        }

        // Check Storage (if required)
        if let Some(req_storage) = requirements.storage_gb {
            if self
                .storage_gb
                .is_none_or(|spec_storage| spec_storage < req_storage)
            {
                info!(
                    "Storage requirements not met: required {} GB, have {:?} GB",
                    req_storage, self.storage_gb
                );
                return false;
            }
        }
        // Check GPU (OR logic applied here)
        if !requirements.gpu.is_empty() {
            // Requirements specify GPUs, so the node must have a GPU spec...
            let Some(spec_gpu) = &self.gpu else {
                debug!("GPU requirements not met: GPU required but none available");
                return false;
            };
            // ...and that GPU spec must meet *at least one* of the requirement options.
            if !requirements
                .gpu
                .iter()
                .any(|req_gpu| spec_gpu.meets(req_gpu))
            {
                debug!("GPU requirements not met");
                return false;
            }
        }
        // If requirements.gpu is empty, no specific GPU is needed, so this part passes.

        // All checked requirements are met
        true
    }
}

impl GpuSpecs {
    /// Checks if the current GPU spec meets a single required GPU spec.
    fn meets(&self, requirement: &GpuRequirements) -> bool {
        // Check count (if required)
        if let Some(req_count) = requirement.count {
            // Node must have exactly the required count. Node having None is okay only if req_count is 0.
            match self.count {
                None => {
                    if req_count > 0 {
                        return false;
                    }
                }
                Some(spec_count) => {
                    if spec_count != req_count {
                        return false;
                    }
                }
            }
        }

        if let Some(req_model) = &requirement.model {
            if !(self.model.as_ref().is_some_and(|spec_model| {
                let normalized_spec = spec_model.to_lowercase().replace(' ', "_");

                // Split the requirement model string by commas and check if any match
                req_model
                    .split(',')
                    .map(|m| m.trim().to_lowercase().replace(' ', "_"))
                    .any(|normalized_req| {
                        // Try both with and without underscores for flexible matching
                        let spec_no_underscore = normalized_spec.replace('_', "");
                        let req_no_underscore = normalized_req.replace('_', "");

                        normalized_spec.contains(&normalized_req)
                            || normalized_req.contains(&normalized_spec)
                            || spec_no_underscore.contains(&req_no_underscore)
                            || req_no_underscore.contains(&spec_no_underscore)
                    })
            })) {
                return false;
            }
        }

        // Check memory per GPU (if required)
        if let Some(req_mem) = requirement.memory_mb {
            if self.memory_mb.is_none_or(|spec_mem| spec_mem < req_mem) {
                return false;
            }
        }

        // Check memory range if specified
        if let Some(req_min) = requirement.memory_mb_min {
            if self.memory_mb.is_none_or(|spec_mem| spec_mem < req_min) {
                return false;
            }
        }
        if let Some(req_max) = requirement.memory_mb_max {
            if self.memory_mb.is_none_or(|spec_mem| spec_mem > req_max) {
                return false;
            }
        }

        // Check total memory requirements (count * memory_mb)
        if let (Some(req_total_min), Some(gpu_count), Some(gpu_memory)) =
            (requirement.total_memory_min, self.count, self.memory_mb)
        {
            let total_memory = gpu_count * gpu_memory;
            if total_memory < req_total_min {
                return false;
            }
        }

        if let (Some(req_total_max), Some(gpu_count), Some(gpu_memory)) =
            (requirement.total_memory_max, self.count, self.memory_mb)
        {
            let total_memory = gpu_count * gpu_memory;
            if total_memory > req_total_max {
                return false;
            }
        }

        // All checked fields meet the requirement
        true
    }
}

impl CpuSpecs {
    /// Checks if the current CPU spec meets the required CPU spec.
    fn meets(&self, requirement: &CpuSpecs) -> bool {
        // Check cores (if required)
        if let Some(req_cores) = requirement.cores {
            if self.cores.is_none_or(|spec_cores| spec_cores < req_cores) {
                return false;
            }
        }

        true
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct NodeLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct DiscoveryNode {
    #[serde(flatten)]
    pub node: Node,
    pub is_validated: bool,
    pub is_active: bool,
    #[serde(default)]
    pub is_provider_whitelisted: bool,
    #[serde(default)]
    pub is_blacklisted: bool,
    #[serde(default)]
    pub last_updated: Option<DateTime<Utc>>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub location: Option<NodeLocation>,
}

impl DiscoveryNode {
    pub fn with_updated_node(&self, new_node: Node) -> Self {
        DiscoveryNode {
            node: new_node,
            is_validated: self.is_validated,
            is_active: self.is_active,
            is_provider_whitelisted: self.is_provider_whitelisted,
            is_blacklisted: self.is_blacklisted,
            last_updated: Some(Utc::now()),
            created_at: self.created_at,
            location: self.location.clone(),
        }
    }
}

impl Deref for DiscoveryNode {
    type Target = Node;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl From<Node> for DiscoveryNode {
    fn from(node: Node) -> Self {
        DiscoveryNode {
            node,
            is_validated: false, // Default values for new discovery nodes
            is_active: false,
            is_provider_whitelisted: false,
            is_blacklisted: false,
            last_updated: None,
            created_at: Some(Utc::now()),
            location: None,
        }
    }
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create ComputeSpecs for testing
    fn create_compute_specs(
        gpu_count: Option<u32>,
        gpu_model: Option<&str>,
        gpu_mem: Option<u32>,
        cpu_cores: Option<u32>,
        ram: Option<u32>,
        storage: Option<u32>,
    ) -> ComputeSpecs {
        ComputeSpecs {
            gpu: if gpu_count.is_some() || gpu_model.is_some() || gpu_mem.is_some() {
                Some(GpuSpecs {
                    count: gpu_count,
                    model: gpu_model.map(String::from),
                    memory_mb: gpu_mem,
                    ..Default::default()
                })
            } else {
                None
            },
            cpu: if cpu_cores.is_some() {
                Some(CpuSpecs {
                    cores: cpu_cores,
                    model: None,
                })
            } else {
                None
            },
            ram_mb: ram,
            storage_gb: storage,
            storage_path: "/var/lib/prime-worker-test".to_string(),
        }
    }

    #[test]
    fn test_requirements_parser_simple() {
        let req_str = "gpu:count=1;gpu:model=A100;gpu:memory_mb=40000;ram_mb=64000;storage_gb=500";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert_eq!(requirements.gpu.len(), 1);
        let gpu_req = &requirements.gpu[0];
        assert_eq!(gpu_req.count, Some(1));
        assert_eq!(gpu_req.model, Some("A100".to_string()));
        assert_eq!(gpu_req.memory_mb, Some(40000));
        assert_eq!(requirements.ram_mb, Some(64000));
        assert_eq!(requirements.storage_gb, Some(500));
        assert!(requirements.cpu.is_none());
    }

    #[test]
    fn test_requirements_parser_gpu_or_logic() {
        let req_str = "gpu:count=8;gpu:model=H100;gpu:memory_mb=80000 ; gpu:count=4;gpu:model=H100;gpu:memory_mb=80000 ; ram_mb=128000; storage_gb=1000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert_eq!(requirements.gpu.len(), 2);

        // First GPU option
        assert_eq!(requirements.gpu[0].count, Some(8));
        assert_eq!(requirements.gpu[0].model, Some("H100".to_string()));
        assert_eq!(requirements.gpu[0].memory_mb, Some(80000));

        // Second GPU option
        assert_eq!(requirements.gpu[1].count, Some(4));
        assert_eq!(requirements.gpu[1].model, Some("H100".to_string()));
        // Memory wasn't repeated for the second option in the *string*, parser should pick it up if specified like gpu:count=4;gpu:model=H100;gpu:memory_mb=80000
        // Let's re-run with memory specified for the second option
        let req_str_mem_repeat = "gpu:count=8;gpu:model=H100;gpu:memory_mb=80000 ; gpu:count=4;gpu:model=H100;gpu:memory_mb=80000 ; ram_mb=128000; storage_gb=1000";
        let requirements_mem_repeat = ComputeRequirements::from_str(req_str_mem_repeat).unwrap();
        assert_eq!(requirements_mem_repeat.gpu.len(), 2);
        assert_eq!(requirements_mem_repeat.gpu[1].memory_mb, Some(80000));

        // Common requirements
        assert_eq!(requirements.ram_mb, Some(128000));
        assert_eq!(requirements.storage_gb, Some(1000));
    }

    #[test]
    fn test_requirements_parser_gpu_minimal() {
        // Only specify count for the second option
        let req_str = "gpu:count=8;gpu:model=H100 ; gpu:count=16; ram_mb=128000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert_eq!(requirements.gpu.len(), 2);
        assert_eq!(requirements.gpu[0].count, Some(8));
        assert_eq!(requirements.gpu[0].model, Some("H100".to_string()));
        assert!(requirements.gpu[0].memory_mb.is_none()); // No memory specified for first

        assert_eq!(requirements.gpu[1].count, Some(16));
        assert!(requirements.gpu[1].model.is_none()); // No model specified for second
        assert!(requirements.gpu[1].memory_mb.is_none()); // No memory specified for second

        assert_eq!(requirements.ram_mb, Some(128000));
    }

    #[test]
    fn test_requirements_parser_no_gpu() {
        let req_str = "ram_mb=32000;storage_gb=250;cpu:cores=8";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert!(requirements.gpu.is_empty());
        assert_eq!(requirements.ram_mb, Some(32000));
        assert_eq!(requirements.storage_gb, Some(250));
        assert!(requirements.cpu.is_some());
        assert_eq!(requirements.cpu.as_ref().unwrap().cores, Some(8));
    }

    #[test]
    fn test_requirements_parser_invalid() {
        assert!(ComputeRequirements::from_str("gpu:count=abc").is_err());
        assert!(ComputeRequirements::from_str("ram_mb=100;gpu_model=xyz").is_err()); // Invalid key
        assert!(ComputeRequirements::from_str("gpu:count=1=2").is_err()); // Invalid format
    }

    // --- Meeting Requirements Tests ---

    #[test]
    fn test_meets_exact_match() {
        let specs = create_compute_specs(
            Some(4),
            Some("nvidia_a100_80gb_pcie"),
            Some(40000),
            Some(16),
            Some(64000),
            Some(500),
        );
        let req_str = "gpu:count=4;gpu:model=A100;gpu:memory_mb=40000;cpu:cores=16;ram_mb=64000;storage_gb=500";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements));
    }
    #[test]
    fn test_a100_range_case() {
        let specs = create_compute_specs(
            Some(1),
            Some("nvidia_a100_80gb_pcie"),
            Some(40000),
            Some(16),
            Some(64000),
            Some(700),
        );
        let req_str = "gpu:count=4;gpu:model=a100,h100,h200;gpu:count=1;gpu:model=a100,h100,h200;storage_gb=700";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements));
    }

    #[test]
    fn test_meets_more_than_required() {
        let specs = create_compute_specs(
            Some(8),
            Some("NVIDIA A100 80GB"),
            Some(80000),
            Some(32),
            Some(128000),
            Some(1000),
        );
        // Requirements are lower
        let req_str = "gpu:count=8;gpu:model=A100;gpu:memory_mb=40000;cpu:cores=16;ram_mb=64000;storage_gb=500";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements));
    }

    #[test]
    fn test_meets_fails_ram() {
        let specs = create_compute_specs(
            Some(4),
            Some("A100"),
            Some(40000),
            Some(16),
            Some(32000),
            Some(500),
        ); // RAM too low
        let req_str = "gpu:count=4;gpu:model=A100;gpu:memory_mb=40000;cpu:cores=16;ram_mb=64000;storage_gb=500";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(!specs.meets(&requirements));
    }

    #[test]
    fn test_meets_fails_gpu_count() {
        let specs = create_compute_specs(
            Some(2),
            Some("A100"),
            Some(40000),
            Some(16),
            Some(64000),
            Some(500),
        ); // GPU count too low
        let req_str = "gpu:count=4;gpu:model=A100;gpu:memory_mb=40000;cpu:cores=16;ram_mb=64000;storage_gb=500";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(!specs.meets(&requirements));
    }

    #[test]
    fn test_meets_fails_gpu_model() {
        let specs = create_compute_specs(
            Some(4),
            Some("RTX 3090"),
            Some(24000),
            Some(16),
            Some(64000),
            Some(500),
        ); // Wrong GPU model
        let req_str = "gpu:count=4;gpu:model=A100;gpu:memory_mb=40000;cpu:cores=16;ram_mb=64000;storage_gb=500";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(!specs.meets(&requirements));
    }

    #[test]
    fn test_meets_gpu_or_option1() {
        // Node has 8x H100
        let specs = create_compute_specs(
            Some(8),
            Some("NVIDIA H100"),
            Some(80000),
            Some(64),
            Some(256000),
            Some(2000),
        );
        // Requirements allow 8x H100 OR 16x A100
        let req_str = "gpu:count=8;gpu:model=H100;gpu:memory_mb=80000 ; gpu:count=16;gpu:model=A100;gpu:memory_mb=80000 ; ram_mb=128000; storage_gb=1000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements)); // Should meet the first GPU option
    }

    #[test]
    fn test_meets_gpu_or_option2() {
        // Node has 16x A100
        let specs = create_compute_specs(
            Some(16),
            Some("NVIDIA A100"),
            Some(80000),
            Some(64),
            Some(256000),
            Some(2000),
        );
        // Requirements allow 8x H100 OR 16x A100
        let req_str = "gpu:count=8;gpu:model=H100;gpu:memory_mb=80000 ; gpu:count=16;gpu:model=A100;gpu:memory_mb=80000 ; ram_mb=128000; storage_gb=1000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements)); // Should meet the second GPU option
    }

    #[test]
    fn test_meets_gpu_or_fails_both() {
        // Node has 4x A100
        let specs = create_compute_specs(
            Some(4),
            Some("NVIDIA A100"),
            Some(80000),
            Some(64),
            Some(256000),
            Some(2000),
        );
        // Requirements allow 8x H100 OR 16x A100
        let req_str = "gpu:count=8;gpu:model=H100;gpu:memory_mb=80000 ; gpu:count=16;gpu:model=A100;gpu:memory_mb=80000 ; ram_mb=128000; storage_gb=1000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(!specs.meets(&requirements)); // Fails both GPU options (count is too low)
    }

    #[test]
    fn test_meets_no_gpu_required() {
        // Node has a GPU
        let specs_with_gpu = create_compute_specs(
            Some(1),
            Some("RTX 3060"),
            Some(12000),
            Some(8),
            Some(32000),
            Some(500),
        );
        // Node has no GPU
        let specs_no_gpu = create_compute_specs(None, None, None, Some(8), Some(32000), Some(500));
        // Requirement doesn't mention GPU
        let req_str = "ram_mb=16000;storage_gb=200;cpu:cores=4";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert!(specs_with_gpu.meets(&requirements)); // Meets because GPU isn't required
        assert!(specs_no_gpu.meets(&requirements)); // Meets because GPU isn't required
    }

    #[test]
    fn test_meets_gpu_required_node_has_none() {
        // Node has no GPU
        let specs = create_compute_specs(None, None, None, Some(8), Some(32000), Some(500));
        // Requirement needs a GPU
        let req_str = "gpu:count=1;gpu:model=A100;ram_mb=16000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(!specs.meets(&requirements)); // Fails because node lacks GPU
    }

    #[test]
    fn test_meets_optional_fields_in_req() {
        // Node has specific specs
        let specs = create_compute_specs(
            Some(4),
            Some("NVIDIA H100"),
            Some(80000),
            Some(64),
            Some(256000),
            Some(2000),
        );
        // Requirements only specify GPU count and RAM (model/memory are optional)
        let req_str = "gpu:count=4; ram_mb=128000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert_eq!(requirements.gpu.len(), 1);
        assert!(requirements.gpu[0].model.is_none());
        assert!(requirements.gpu[0].memory_mb.is_none());
        assert!(specs.meets(&requirements)); // Should meet as count and RAM are sufficient
    }

    #[test]
    fn test_meets_optional_fields_in_spec() {
        // Node spec is missing GPU memory info
        let specs = ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: Some(4),
                model: Some("A100".to_string()),
                ..Default::default()
            }),
            cpu: Some(CpuSpecs {
                cores: Some(16),
                model: None,
            }),
            ram_mb: Some(64000),
            storage_gb: Some(500),
            storage_path: "/var/lib/prime-worker-test".to_string(),
        };
        // Requirement needs specific memory
        let req_str_mem = "gpu:count=4;gpu:model=A100;gpu:memory_mb=40000";
        let requirements_mem = ComputeRequirements::from_str(req_str_mem).unwrap();
        // Requirement doesn't need specific memory
        let req_str_no_mem = "gpu:count=4;gpu:model=A100";
        let requirements_no_mem = ComputeRequirements::from_str(req_str_no_mem).unwrap();

        assert!(!specs.meets(&requirements_mem)); // Fails because spec memory is None, but req needs Some(40000)
        assert!(specs.meets(&requirements_no_mem)); // Passes because req doesn't care about memory
    }
    #[test]
    fn test_meets_min_max_gpu_memory() {
        // Node spec with specific GPU memory
        let specs = ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: Some(4),
                model: Some("A100".to_string()),
                memory_mb: Some(40000),
                ..Default::default()
            }),
            cpu: Some(CpuSpecs {
                cores: Some(16),
                model: None,
            }),
            ram_mb: Some(64000),
            storage_gb: Some(500),
            storage_path: "/var/lib/prime-worker-test".to_string(),
        };

        // Test case 1: Requirements with memory range that includes node's memory
        let req_str_in_range =
            "gpu:count=4;gpu:model=A100;gpu:memory_mb_min=30000;gpu:memory_mb_max=50000";
        let requirements_in_range = ComputeRequirements::from_str(req_str_in_range).unwrap();
        assert!(
            specs.meets(&requirements_in_range),
            "Should meet requirements as memory is within range"
        );

        // Test case 2: Requirements with memory range below node's memory
        let req_str_below =
            "gpu:count=4;gpu:model=A100;gpu:memory_mb_min=20000;gpu:memory_mb_max=35000";
        let requirements_below = ComputeRequirements::from_str(req_str_below).unwrap();
        assert!(
            !specs.meets(&requirements_below),
            "Should not meet requirements as memory is above max range"
        );

        // Test case 3: Requirements with memory range above node's memory
        let req_str_above =
            "gpu:count=4;gpu:model=A100;gpu:memory_mb_min=45000;gpu:memory_mb_max=60000";
        let requirements_above = ComputeRequirements::from_str(req_str_above).unwrap();
        assert!(
            !specs.meets(&requirements_above),
            "Should not meet requirements as memory is below min range"
        );

        // Test case 4: Invalid specification of both exact memory and range
        let req_str_invalid = "gpu:count=4;gpu:model=A100;gpu:memory_mb=40000;gpu:memory_mb_min=30000;gpu:memory_mb_max=50000";
        let result = ComputeRequirements::from_str(req_str_invalid);
        assert!(
            result.is_err(),
            "Should fail because cannot specify both exact memory and min/max memory"
        );
    }

    #[test]
    fn test_gpu_model_case_insensitive_matching() {
        let specs = create_compute_specs(
            Some(1),
            Some("NVIDIA_A100_80GB_PCIE"),
            Some(40000),
            None,
            None,
            None,
        );
        let req_str = "gpu:model=nvidia_a100_80gb_pcie";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements));
    }

    #[test]
    fn test_gpu_model_no_match() {
        let specs = create_compute_specs(
            Some(1),
            Some("AMD Radeon RX 7900"),
            Some(20000),
            None,
            None,
            None,
        );
        let req_str = "gpu:model=nvidia,rtx";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(!specs.meets(&requirements));
    }

    #[test]
    fn test_parser_empty_values() {
        assert!(ComputeRequirements::from_str("gpu:count=").is_err());
        assert!(ComputeRequirements::from_str("gpu:model=").is_ok()); // Empty model might be valid
    }

    #[test]
    fn test_parser_whitespace_handling() {
        let req_str = " gpu:count = 1 ; gpu:model = A100 ; ram_mb = 32000 ";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert_eq!(requirements.gpu[0].count, Some(1));
        assert_eq!(requirements.gpu[0].model, Some("A100".to_string()));
    }

    #[test]
    fn test_parser_empty_string() {
        let requirements = ComputeRequirements::from_str("").unwrap();
        assert!(requirements.gpu.is_empty());
        assert!(requirements.cpu.is_none());
    }

    #[test]
    fn test_complex_gpu_or_logic() {
        let specs = create_compute_specs(
            Some(2),
            Some("rtx4090"),
            Some(24000),
            None,
            Some(64000),
            None,
        );

        // Requirements: (8x H100 80GB) OR (4x A100 40GB) OR (2x RTX4090 24GB)
        let req_str = "gpu:count=8;gpu:model=H100;gpu:memory_mb=80000;gpu:count=4;gpu:model=A100;gpu:memory_mb=40000;gpu:count=2;gpu:model=RTX4090;gpu:memory_mb=24000;ram_mb=62000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(specs.meets(&requirements));
    }

    #[test]
    fn test_gpu_memory_min_max_validation() {
        // Test that min > max is rejected
        let req_str = "gpu:memory_mb_min=40000;gpu:memory_mb_max=20000";
        assert!(ComputeRequirements::from_str(req_str).is_err());

        // Test that max > min is accepted
        let req_str = "gpu:memory_mb_min=20000;gpu:memory_mb_max=40000";
        assert!(ComputeRequirements::from_str(req_str).is_ok());
    }

    #[test]
    fn test_total_memory_parsing() {
        let req_str =
            "gpu:count=4;gpu:model=A100;gpu:total_memory_min=160000;gpu:total_memory_max=320000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert_eq!(requirements.gpu.len(), 1);
        let gpu_req = &requirements.gpu[0];
        assert_eq!(gpu_req.count, Some(4));
        assert_eq!(gpu_req.model, Some("A100".to_string()));
        assert_eq!(gpu_req.total_memory_min, Some(160000));
        assert_eq!(gpu_req.total_memory_max, Some(320000));
    }

    #[test]
    fn test_total_memory_validation() {
        // Test that total_memory_min > total_memory_max is rejected
        let req_str = "gpu:total_memory_min=400000;gpu:total_memory_max=200000";
        assert!(ComputeRequirements::from_str(req_str).is_err());

        // Test that total_memory_max > total_memory_min is accepted
        let req_str = "gpu:total_memory_min=200000;gpu:total_memory_max=400000";
        assert!(ComputeRequirements::from_str(req_str).is_ok());
    }

    #[test]
    fn test_meets_total_memory_requirements() {
        // Node has 4x A100 with 40GB each = 160GB total
        let specs =
            create_compute_specs(Some(4), Some("NVIDIA A100"), Some(40000), None, None, None);

        // Test case 1: Total memory requirement within range
        let req_str =
            "gpu:count=4;gpu:model=A100;gpu:total_memory_min=120000;gpu:total_memory_max=200000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(
            specs.meets(&requirements),
            "Should meet total memory requirements within range"
        );

        // Test case 2: Total memory requirement too high (min)
        let req_str = "gpu:count=4;gpu:model=A100;gpu:total_memory_min=200000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(
            !specs.meets(&requirements),
            "Should not meet requirements as total memory is below min"
        );

        // Test case 3: Total memory requirement too low (max)
        let req_str = "gpu:count=4;gpu:model=A100;gpu:total_memory_max=120000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(
            !specs.meets(&requirements),
            "Should not meet requirements as total memory is above max"
        );

        // Test case 4: Exact total memory match
        let req_str =
            "gpu:count=4;gpu:model=A100;gpu:total_memory_min=160000;gpu:total_memory_max=160000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();
        assert!(
            specs.meets(&requirements),
            "Should meet exact total memory requirements"
        );
    }

    #[test]
    fn test_meets_total_memory_missing_fields() {
        // Node has count but no memory specified
        let specs_no_memory = ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: Some(4),
                model: Some("A100".to_string()),
                memory_mb: None, // No memory specified
                ..Default::default()
            }),
            ..Default::default()
        };

        // Node has memory but no count specified
        let specs_no_count = ComputeSpecs {
            gpu: Some(GpuSpecs {
                count: None, // No count specified
                model: Some("A100".to_string()),
                memory_mb: Some(40000),
                ..Default::default()
            }),
            ..Default::default()
        };

        let req_str = "gpu:model=A100;gpu:total_memory_min=120000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        // Both should pass because total memory check is skipped when count or memory is missing
        assert!(
            specs_no_memory.meets(&requirements),
            "Should pass when memory is not specified"
        );
        assert!(
            specs_no_count.meets(&requirements),
            "Should pass when count is not specified"
        );
    }

    #[test]
    fn test_meets_total_memory_or_logic() {
        // Node has 8x H100 with 80GB each = 640GB total
        let specs =
            create_compute_specs(Some(8), Some("NVIDIA H100"), Some(80000), None, None, None);

        // Requirements: (4x A100 with 160GB total) OR (8x H100 with 500GB+ total)
        let req_str = "gpu:count=4;gpu:model=A100;gpu:total_memory_min=160000;gpu:count=8;gpu:model=H100;gpu:total_memory_min=500000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert_eq!(requirements.gpu.len(), 2);
        assert!(
            specs.meets(&requirements),
            "Should meet the second GPU option with total memory requirement"
        );
    }

    #[test]
    fn test_complex_total_memory_scenario() {
        // Node has 2x RTX 4090 with 24GB each = 48GB total
        let specs = create_compute_specs(Some(2), Some("RTX 4090"), Some(24000), None, None, None);

        // Requirements allow multiple options with different total memory requirements
        let req_str = "gpu:count=8;gpu:model=H100;gpu:total_memory_min=600000;gpu:count=4;gpu:model=A100;gpu:total_memory_min=160000;gpu:count=2;gpu:model=RTX4090;gpu:total_memory_min=40000;gpu:total_memory_max=60000";
        let requirements = ComputeRequirements::from_str(req_str).unwrap();

        assert_eq!(requirements.gpu.len(), 3);
        assert!(
            specs.meets(&requirements),
            "Should meet the third GPU option with total memory range"
        );
    }
    #[test]
    fn test_multiple_gpu_counts() {
        let requirements = ComputeRequirements::from_str("gpu:count=1;gpu:memory_mb_min=24000;gpu:memory_mb_max=24999;gpu:count=2;gpu:count=3;gpu:count=4;").unwrap();
        assert_eq!(requirements.gpu.len(), 4);
        assert_eq!(requirements.gpu[0].count, Some(1));
        assert_eq!(requirements.gpu[0].memory_mb_min, Some(24000));
        assert_eq!(requirements.gpu[0].memory_mb_max, Some(24999));
        assert_eq!(requirements.gpu[1].count, Some(2));
        assert_eq!(requirements.gpu[1].memory_mb_min, None);
        assert_eq!(requirements.gpu[1].memory_mb_max, None);
        assert_eq!(requirements.gpu[2].count, Some(3));
        assert_eq!(requirements.gpu[3].count, Some(4));
    }
}
