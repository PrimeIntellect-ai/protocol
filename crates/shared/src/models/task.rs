use std::collections::HashMap;

use chrono::Utc;
use redis::{ErrorKind, FromRedisValue, RedisError, RedisResult, RedisWrite, ToRedisArgs, Value};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum TaskState {
    PENDING,
    PULLING,
    RUNNING,
    COMPLETED,
    FAILED,
    PAUSED,
    RESTARTING,
    #[default]
    UNKNOWN,
}

impl From<&str> for TaskState {
    fn from(s: &str) -> Self {
        match s {
            "PENDING" => TaskState::PENDING,
            "PULLING" => TaskState::PULLING,
            "RUNNING" => TaskState::RUNNING,
            "COMPLETED" => TaskState::COMPLETED,
            "FAILED" => TaskState::FAILED,
            "PAUSED" => TaskState::PAUSED,
            "RESTARTING" => TaskState::RESTARTING,
            "UNKNOWN" => TaskState::UNKNOWN,
            _ => TaskState::UNKNOWN, // Default case
        }
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let state_str = match self {
            TaskState::PENDING => "PENDING",
            TaskState::PULLING => "PULLING",
            TaskState::RUNNING => "RUNNING",
            TaskState::COMPLETED => "COMPLETED",
            TaskState::FAILED => "FAILED",
            TaskState::PAUSED => "PAUSED",
            TaskState::RESTARTING => "RESTARTING",
            TaskState::UNKNOWN => "UNKNOWN",
        };
        write!(f, "{}", state_str)
    }
}

// Scheduling config
// Proper typing and validation currently missing
// Issue: https://github.com/PrimeIntellect-ai/protocol/issues/338
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SchedulingConfig {
    pub plugins: Option<HashMap<String, HashMap<String, Vec<String>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VolumeMount {
    /// Name/path of the volume on the host (supports label replacements)
    pub host_path: String,
    /// Path where the volume should be mounted in the container
    pub container_path: String,
}

impl VolumeMount {
    /// Replace labels in the host_path with actual values
    /// Note: GROUP_ID replacement is handled by the node groups plugin
    /// (temporary until we have an expander trait)
    pub fn replace_labels(&self, task_id: &str, node_address: Option<&str>) -> Self {
        let mut host_path = self.host_path.clone();
        let mut container_path = self.container_path.clone();

        // Replace ${TASK_ID} with actual task ID
        host_path = host_path.replace("${TASK_ID}", task_id);
        container_path = container_path.replace("${TASK_ID}", task_id);

        // Replace ${NODE_ADDRESS} with actual node address if provided
        if let Some(addr) = node_address {
            host_path = host_path.replace("${NODE_ADDRESS}", addr);
            container_path = container_path.replace("${NODE_ADDRESS}", addr);
        }

        // Get current timestamp for ${TIMESTAMP}
        let timestamp = chrono::Utc::now().timestamp().to_string();
        host_path = host_path.replace("${TIMESTAMP}", &timestamp);
        container_path = container_path.replace("${TIMESTAMP}", &timestamp);

        Self {
            host_path,
            container_path,
        }
    }

    /// Validate the volume mount configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.host_path.is_empty() {
            return Err("Host path cannot be empty".to_string());
        }

        if self.container_path.is_empty() {
            return Err("Container path cannot be empty".to_string());
        }

        // Check for supported variables
        let supported_vars = [
            "${TASK_ID}",
            "${GROUP_ID}",
            "${TIMESTAMP}",
            "${NODE_ADDRESS}",
        ];

        let re = regex::Regex::new(r"\$\{[^}]+\}").unwrap();

        // Check host_path
        for cap in re.find_iter(&self.host_path) {
            let var = cap.as_str();
            if !supported_vars.contains(&var) {
                return Err(format!(
                    "Volume mount host_path contains unsupported variable: {}. Supported variables: {:?}",
                    var, supported_vars
                ));
            }
        }

        // Check container_path
        for cap in re.find_iter(&self.container_path) {
            let var = cap.as_str();
            if !supported_vars.contains(&var) {
                return Err(format!(
                    "Volume mount container_path contains unsupported variable: {}. Supported variables: {:?}",
                    var, supported_vars
                ));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskRequest {
    pub image: String,
    pub name: String,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub cmd: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub scheduling_config: Option<SchedulingConfig>,
    pub storage_config: Option<StorageConfig>,
    pub metadata: Option<TaskMetadata>,
    pub volume_mounts: Option<Vec<VolumeMount>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskMetadata {
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub name: String,
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub image: String,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub cmd: Option<Vec<String>>,
    pub entrypoint: Option<Vec<String>>,
    pub state: TaskState,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub scheduling_config: Option<SchedulingConfig>,
    #[serde(default)]
    pub storage_config: Option<StorageConfig>,
    #[serde(default)]
    pub metadata: Option<TaskMetadata>,
    #[serde(default)]
    pub volume_mounts: Option<Vec<VolumeMount>>,
}

impl Task {
    /// Generate a hash of the task configuration for comparison purposes
    pub fn generate_config_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash core configuration
        self.image.hash(&mut hasher);
        self.cmd.hash(&mut hasher);
        self.entrypoint.hash(&mut hasher);

        // Hash environment variables in sorted order for consistency
        if let Some(env_vars) = &self.env_vars {
            let mut sorted_env: Vec<_> = env_vars.iter().collect();
            sorted_env.sort_by_key(|(k, _)| *k);
            for (key, value) in sorted_env {
                key.hash(&mut hasher);
                value.hash(&mut hasher);
            }
        }

        // Hash volume mounts in sorted order for consistency
        if let Some(volume_mounts) = &self.volume_mounts {
            let mut sorted_volumes: Vec<_> = volume_mounts.iter().collect();
            sorted_volumes.sort_by(|a, b| {
                a.host_path
                    .cmp(&b.host_path)
                    .then_with(|| a.container_path.cmp(&b.container_path))
            });
            for volume_mount in sorted_volumes {
                volume_mount.host_path.hash(&mut hasher);
                volume_mount.container_path.hash(&mut hasher);
            }
        }

        hasher.finish()
    }
}

impl Default for Task {
    fn default() -> Self {
        Self {
            name: String::new(),
            id: Uuid::new_v4(),
            image: String::new(),
            env_vars: None,
            cmd: None,
            entrypoint: None,
            state: TaskState::default(),
            created_at: 0,
            updated_at: None,
            scheduling_config: None,
            storage_config: None,
            metadata: None,
            volume_mounts: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StorageConfig {
    pub file_name_template: Option<String>,
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(template) = &self.file_name_template {
            let valid_vars = [
                "${ORIGINAL_NAME}",
                "${NODE_GROUP_ID}",
                "${NODE_GROUP_SIZE}",
                "${NODE_GROUP_INDEX}",
                "${TOTAL_UPLOAD_COUNT_AFTER}",
                "${CURRENT_FILE_INDEX}",
            ];

            let re = regex::Regex::new(r"\$\{[^}]+\}").unwrap();
            for cap in re.find_iter(template) {
                let var = cap.as_str();
                if !valid_vars.contains(&var) {
                    return Err(format!(
                        "Storage config template contains invalid variable: {}",
                        var
                    ));
                }
            }
        }
        Ok(())
    }
}

impl TryFrom<TaskRequest> for Task {
    type Error = String;

    fn try_from(request: TaskRequest) -> Result<Self, Self::Error> {
        if let Some(storage_config) = &request.storage_config {
            storage_config.validate()?;
        }

        if let Some(volume_mounts) = &request.volume_mounts {
            for volume_mount in volume_mounts {
                volume_mount.validate()?;
            }
        }

        Ok(Task {
            name: request.name,
            id: Uuid::new_v4(),
            image: request.image,
            cmd: request.cmd,
            entrypoint: request.entrypoint,
            env_vars: request.env_vars,
            state: TaskState::PENDING,
            created_at: Utc::now().timestamp_millis(),
            updated_at: None,
            scheduling_config: request.scheduling_config,
            storage_config: request.storage_config,
            metadata: request.metadata,
            volume_mounts: request.volume_mounts,
        })
    }
}

impl FromRedisValue for Task {
    fn from_redis_value(v: &Value) -> RedisResult<Self> {
        match v {
            Value::BulkString(s) => {
                let task: Task = serde_json::from_slice(s).map_err(|_| {
                    RedisError::from((
                        ErrorKind::TypeError,
                        "Failed to deserialize Task from string",
                        format!("Invalid JSON string: {:?}", s),
                    ))
                })?;
                Ok(task)
            }
            _ => Err(RedisError::from((
                ErrorKind::TypeError,
                "Response type not compatible with Task",
                format!("Received: {:?}", v),
            ))),
        }
    }
}

impl ToRedisArgs for Task {
    fn write_redis_args<W>(&self, out: &mut W)
    where
        W: ?Sized + RedisWrite,
    {
        let task_json = serde_json::to_string(self).expect("Failed to serialize Task to JSON");
        out.write_arg(task_json.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_mount_label_replacement() {
        let volume_mount = VolumeMount {
            host_path: "/host/data/${TASK_ID}".to_string(),
            container_path: "/container/data/${TASK_ID}".to_string(),
        };

        let processed = volume_mount.replace_labels("task-123", Some("node-addr"));

        assert_eq!(processed.host_path, "/host/data/task-123");
        assert_eq!(processed.container_path, "/container/data/task-123");
    }

    #[test]
    fn test_volume_mount_label_replacement_without_group() {
        let volume_mount = VolumeMount {
            host_path: "/host/data/${TASK_ID}".to_string(),
            container_path: "/container/data/${TASK_ID}".to_string(),
        };

        let processed = volume_mount.replace_labels("task-789", None);

        assert_eq!(processed.host_path, "/host/data/task-789");
        assert_eq!(processed.container_path, "/container/data/task-789");
    }

    #[test]
    fn test_volume_mount_with_timestamp() {
        let volume_mount = VolumeMount {
            host_path: "/host/logs/${TASK_ID}-${TIMESTAMP}".to_string(),
            container_path: "/container/logs".to_string(),
        };

        let processed = volume_mount.replace_labels("task-123", None);

        assert!(processed.host_path.starts_with("/host/logs/task-123-"));
        assert!(processed.host_path.len() > "/host/logs/task-123-".len());
        assert_eq!(processed.container_path, "/container/logs");
    }

    #[test]
    fn test_volume_mount_validation_success() {
        let volume_mount = VolumeMount {
            host_path: "/host/data/${TASK_ID}".to_string(),
            container_path: "/container/data".to_string(),
        };

        assert!(volume_mount.validate().is_ok());
    }

    #[test]
    fn test_volume_mount_validation_with_node_address() {
        let volume_mount = VolumeMount {
            host_path: "/host/data/${NODE_ADDRESS}".to_string(),
            container_path: "/container/data/${TASK_ID}".to_string(),
        };

        assert!(volume_mount.validate().is_ok());
    }

    #[test]
    fn test_volume_mount_validation_empty_host_path() {
        let volume_mount = VolumeMount {
            host_path: "".to_string(),
            container_path: "/container/data".to_string(),
        };

        assert!(volume_mount.validate().is_err());
        assert_eq!(
            volume_mount.validate().unwrap_err(),
            "Host path cannot be empty"
        );
    }

    #[test]
    fn test_volume_mount_validation_empty_container_path() {
        let volume_mount = VolumeMount {
            host_path: "/host/data".to_string(),
            container_path: "".to_string(),
        };

        assert!(volume_mount.validate().is_err());
        assert_eq!(
            volume_mount.validate().unwrap_err(),
            "Container path cannot be empty"
        );
    }

    #[test]
    fn test_volume_mount_validation_unsupported_variable() {
        let volume_mount = VolumeMount {
            host_path: "/host/data/${UNSUPPORTED_VAR}".to_string(),
            container_path: "/container/data".to_string(),
        };

        assert!(volume_mount.validate().is_err());
        assert!(volume_mount
            .validate()
            .unwrap_err()
            .contains("unsupported variable: ${UNSUPPORTED_VAR}"));
    }

    #[test]
    fn test_task_with_volume_mounts() {
        let task_request = TaskRequest {
            image: "ubuntu:latest".to_string(),
            name: "test-task".to_string(),
            volume_mounts: Some(vec![
                VolumeMount {
                    host_path: "/host/data/${TASK_ID}".to_string(),
                    container_path: "/data".to_string(),
                },
                VolumeMount {
                    host_path: "/host/logs/${TASK_ID}".to_string(),
                    container_path: "/logs".to_string(),
                },
            ]),
            ..Default::default()
        };

        let task = Task::try_from(task_request).unwrap();
        assert!(task.volume_mounts.is_some());
        assert_eq!(task.volume_mounts.as_ref().unwrap().len(), 2);
    }
}
