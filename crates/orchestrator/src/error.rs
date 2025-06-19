use std::fmt;

#[derive(Debug)]
pub enum OrchestratorError {
    Custom(String),
    Redis(redis::RedisError),
    Io(std::io::Error),
    SerializationError(serde_json::Error),
    #[allow(dead_code)]
    GroupNotFound(String),
    #[allow(dead_code)]
    NetworkError(String),
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrchestratorError::Custom(msg) => write!(f, "Orchestrator error: {}", msg),
            OrchestratorError::Redis(e) => write!(f, "Redis error: {}", e),
            OrchestratorError::Io(e) => write!(f, "IO error: {}", e),
            OrchestratorError::SerializationError(e) => write!(f, "Serialization error: {}", e),
            OrchestratorError::GroupNotFound(group_id) => {
                write!(f, "Group not found: {}", group_id)
            }
            OrchestratorError::NetworkError(msg) => write!(f, "Network error: {}", msg),
        }
    }
}

impl std::error::Error for OrchestratorError {}

impl From<redis::RedisError> for OrchestratorError {
    fn from(err: redis::RedisError) -> Self {
        OrchestratorError::Redis(err)
    }
}

impl From<std::io::Error> for OrchestratorError {
    fn from(err: std::io::Error) -> Self {
        OrchestratorError::Io(err)
    }
}

impl From<serde_json::Error> for OrchestratorError {
    fn from(err: serde_json::Error) -> Self {
        OrchestratorError::SerializationError(err)
    }
}
