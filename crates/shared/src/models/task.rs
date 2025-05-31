use std::collections::HashMap;

use chrono::Utc;
use redis::{ErrorKind, FromRedisValue, RedisError, RedisResult, RedisWrite, ToRedisArgs, Value};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskRequest {
    pub image: String,
    pub name: String,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub scheduling_config: Option<SchedulingConfig>,
    pub storage_config: Option<StorageConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub image: String,
    pub name: String,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub state: TaskState,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: Option<u64>,
    #[serde(default)]
    pub scheduling_config: Option<SchedulingConfig>,
    #[serde(default)]
    pub storage_config: Option<StorageConfig>,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            image: String::new(),
            name: String::new(),
            env_vars: None,
            command: None,
            args: None,
            state: TaskState::default(),
            created_at: 0,
            updated_at: None,
            scheduling_config: None,
            storage_config: None,
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

        Ok(Task {
            id: Uuid::new_v4(),
            image: request.image,
            name: request.name,
            command: request.command,
            args: request.args,
            env_vars: request.env_vars,
            state: TaskState::PENDING,
            created_at: Utc::now().timestamp_millis(),
            updated_at: None,
            scheduling_config: request.scheduling_config,
            storage_config: request.storage_config,
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
