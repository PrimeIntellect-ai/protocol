#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskContainer {
    pub task_id: String,
    pub config_hash: String,
}

impl TaskContainer {
    pub(crate) fn data_dir_name(&self) -> String {
        format!("prime-task-{}", self.task_id)
    }
}

impl std::str::FromStr for TaskContainer {
    type Err = &'static str;

    fn from_str(container_name: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = container_name.trim_start_matches('/').split('-').collect();

        if parts.len() >= 8 && parts[0] == "prime" && parts[1] == "task" {
            let task_id = parts[2..7].join("-");
            let config_hash = parts[7..].join("-");
            Ok(Self {
                task_id,
                config_hash,
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
            result.map(|c| c.task_id),
            Ok("123e4567-e89b-12d3-a456-426614174000".to_string())
        );

        // Test with leading slash
        let container_name = "/prime-task-123e4567-e89b-12d3-a456-426614174000-a1b2c3d4";
        let result = TaskContainer::from_str(container_name);
        assert_eq!(
            result.map(|c| c.task_id),
            Ok("123e4567-e89b-12d3-a456-426614174000".to_string())
        );

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
}
