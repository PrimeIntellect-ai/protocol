use redis::{ErrorKind, FromRedisValue, RedisError, RedisResult, RedisWrite, ToRedisArgs, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    pub image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub image: String,
}

impl From<TaskRequest> for Task {
    fn from(request: TaskRequest) -> Self {
        Task {
            id: Uuid::new_v4(),
            image: request.image,
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
