#[derive(Debug, Clone, PartialEq)]
pub struct TaskContainer {
    pub task_id: String,
    pub config_hash: String,
    pub gpu_index: Option<u32>,
}

impl TaskContainer {
    pub fn data_dir_name(&self) -> String {
        format!("prime-task-{}", self.task_id)
    }
}

impl std::str::FromStr for TaskContainer {
    type Err = &'static str;

    fn from_str(container_name: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = container_name
            .trim()
            .trim_start_matches('/')
            .split('-')
            .collect();
        if parts.len() >= 8 && parts[0] == "prime" && parts[1] == "task" {
            let task_id = parts[2..7].join("-");

            // Check if the container name has a GPU suffix
            let (config_hash, gpu_index) =
                if parts.len() >= 9 && parts[parts.len() - 1].starts_with("gpu") {
                    // Has GPU suffix: extract GPU index from "gpu0", "gpu1", etc.
                    let gpu_part = parts[parts.len() - 1];
                    let gpu_idx = gpu_part
                        .strip_prefix("gpu")
                        .ok_or("Invalid GPU suffix format")?
                        .parse::<u32>()
                        .map_err(|_| "Invalid GPU index")?;
                    let config_hash = parts[7..parts.len() - 1].join("-");
                    (config_hash, Some(gpu_idx))
                } else {
                    // No GPU suffix: normal container
                    let config_hash = parts[7..].join("-");
                    (config_hash, None)
                };

            Ok(Self {
                task_id,
                config_hash,
                gpu_index,
            })
        } else {
            Err("Invalid container name format")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_task_id_from_container_name() {
        // Test with valid container name
        let container_name = "prime-task-123e4567-e89b-12d3-a456-426614174000-a1b2c3d4";
        let result = TaskContainer::from_str(container_name);
        assert_eq!(
            result.as_ref().map(|c| c.task_id.clone()),
            Ok("123e4567-e89b-12d3-a456-426614174000".to_string())
        );
        assert_eq!(result.as_ref().map(|c| c.gpu_index), Ok(None));

        // Test with leading slash
        let container_name = "/prime-task-123e4567-e89b-12d3-a456-426614174000-a1b2c3d4";
        let result = TaskContainer::from_str(container_name);
        assert_eq!(
            result.as_ref().map(|c| c.task_id.clone()),
            Ok("123e4567-e89b-12d3-a456-426614174000".to_string())
        );
        assert_eq!(result.as_ref().map(|c| c.gpu_index), Ok(None));

        // Test with invalid format
        let container_name = "not-a-prime-task";
        let result = TaskContainer::from_str(container_name);
        assert!(result.is_err());

        // Test with short UUID (should fail)
        let container_name = "prime-task-short-uuid-hash";
        let result = TaskContainer::from_str(container_name);
        assert!(result.is_err());

        // Test with no hash suffix
        let container_name = "prime-task-123e4567-e89b-12d3-a456-426614174000";
        let result = TaskContainer::from_str(container_name);
        assert!(result.is_err());
    }

    #[test]
    fn test_gpu_partitioned_container_names() {
        // Test with GPU suffix
        let container_name =
            " prime-task-c45f0b5b-683b-400a-9452-132d0c1bd00e-73adfcfcfbf417c1-gpu1";
        let result = TaskContainer::from_str(container_name).unwrap();
        println!("result: {:?}", result);
        assert_eq!(result.task_id, "c45f0b5b-683b-400a-9452-132d0c1bd00e");
        assert_eq!(result.config_hash, "73adfcfcfbf417c1");
        assert_eq!(result.gpu_index, Some(1));

        // Test with GPU suffix (gpu1)
        let container_name =
            " prime-task-c45f0b5b-683b-400a-9452-132d0c1bd00e-73adfcfcfbf417c1-gpu2";
        let result = TaskContainer::from_str(container_name).unwrap();
        assert_eq!(result.task_id, "c45f0b5b-683b-400a-9452-132d0c1bd00e");
        assert_eq!(result.config_hash, "73adfcfcfbf417c1");
        assert_eq!(result.gpu_index, Some(2));

        // Test data_dir_name doesn't include GPU suffix
        assert_eq!(
            result.data_dir_name(),
            "prime-task-c45f0b5b-683b-400a-9452-132d0c1bd00e"
        );

        // Test with invalid GPU index (non-numeric)
        let container_name =
            "prime-task-c45f0b5b-683b-400a-9452-132d0c1bd00e-73adfcfcfbf417c1-gpuinvalid";
        let result = TaskContainer::from_str(container_name);
        assert!(result.is_err());
    }
}
