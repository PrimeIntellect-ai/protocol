use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::ops::Deref;
use std::str::FromStr;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Node {
    pub id: String,
    pub provider_address: String,
    pub ip_address: String,
    pub port: u16,
    pub compute_pool_id: u32,
    pub compute_specs: Option<ComputeSpecs>,
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
    pub storage_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GpuSpecs {
    pub count: Option<u32>,
    pub model: Option<String>,
    pub memory_mb: Option<u32>,
}

// Parser for compute specs from e.g. the compute pool uri
impl FromStr for ComputeSpecs {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Initialize an empty ComputeSpecs
        let mut specs = ComputeSpecs {
            cpu: None,
            ram_mb: None,
            storage_path: None,
            gpu: None,
            storage_gb: None,
        };

        // Temporary GPU specs that we'll update as we parse
        let mut gpu_specs = GpuSpecs {
            count: None,
            model: None,
            memory_mb: None,
        };

        // Flag to track if we've found any GPU specs
        let mut has_gpu_specs = false;

        // Split the input string by semicolons
        for part in s.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            // Split each part by the first colon
            let parts: Vec<&str> = part.splitn(2, '=').collect();
            if parts.len() != 2 {
                return Err(anyhow::anyhow!("Invalid key-value pair: {}", part));
            }

            let key = parts[0].trim();
            let value = parts[1].trim();

            println!("key: {}", key);

            // Parse based on the key
            match key {
                // GPU specifications
                "gpu:count" => {
                    has_gpu_specs = true;
                    // Check if it's a range (e.g., 4-8)
                    if value.contains('-') {
                        let range: Vec<&str> = value.split('-').collect();
                        if range.len() == 2 {
                            // For a range, we'll store the minimum as the count
                            let min = range[0]
                                .trim()
                                .parse::<u32>()
                                .map_err(|_| anyhow!("Invalid GPU count range: {}", value))?;
                            gpu_specs.count = Some(min);
                        } else {
                            return Err(anyhow!("Invalid range format: {}", value));
                        }
                    } else {
                        gpu_specs.count = Some(
                            value
                                .parse::<u32>()
                                .map_err(|_| anyhow!("Invalid GPU count: {}", value))?,
                        );
                    }
                }
                "gpu:model" => {
                    has_gpu_specs = true;
                    gpu_specs.model = Some(value.to_string());
                }
                "gpu:memory_mb" => {
                    has_gpu_specs = true;
                    gpu_specs.memory_mb = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| anyhow!("Invalid GPU memory: {}", value))?,
                    );
                }
                // Storage specifications
                "storage_gb" => {
                    specs.storage_gb = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| anyhow!("Invalid storage: {}", value))?,
                    );
                }
                _ => return Err(anyhow!("Unknown key: {}", key)),
            }
        }

        // Add GPU specs if we found any
        if has_gpu_specs {
            specs.gpu = Some(gpu_specs);
        }

        Ok(specs)
    }
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CpuSpecs {
    pub cores: Option<u32>,
    pub model: Option<String>,
}

// Discover node contains validation info and is typically returned by the discovery svc

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_specs_from_str() {
        let compute_specs =
            ComputeSpecs::from_str("gpu:count=1;gpu:model=a_100;storage_gb=100").unwrap();
        let gpu_specs = compute_specs.gpu.unwrap();
        assert_eq!(gpu_specs.count, Some(1));
        assert_eq!(gpu_specs.model, Some("a_100".to_string()));
        assert_eq!(compute_specs.storage_gb, Some(100));
    }

    #[test]
    fn test_compute_specs_from_str_range() {
        let compute_specs = ComputeSpecs::from_str(
            "gpu:count=4-8;gpu:model=a_100,h_100,h_200;gpu:memory_mb=80000;storage_gb=100",
        )
        .unwrap();
        let gpu_specs = compute_specs.gpu.unwrap();
        assert_eq!(gpu_specs.count, Some(4));
        assert!(gpu_specs.model.unwrap().contains("a_100"));
        assert_eq!(gpu_specs.memory_mb, Some(80000));
        assert_eq!(compute_specs.storage_gb, Some(100));
    }
}
