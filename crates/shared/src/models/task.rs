use chrono::Utc;
use redis::{ErrorKind, FromRedisValue, RedisError, RedisResult, RedisWrite, ToRedisArgs, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskState {
    PENDING,
    PULLING,
    RUNNING,
    COMPLETED,
    FAILED,
    PAUSED,
    RESTARTING,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub image: String,
    pub name: String,
    pub env_vars: Option<std::collections::HashMap<String, String>>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
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
    pub updated_at: Option<u64>
}

impl From<TaskRequest> for Task {
    fn from(request: TaskRequest) -> Self {
        Task {
            id: Uuid::new_v4(),
            image: request.image,
            name: request.name,
            command: request.command,
            args: request.args,
            env_vars: request.env_vars,
            state: TaskState::PENDING,
            created_at: Utc::now().timestamp_millis(),
            updated_at: None 
        }
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
