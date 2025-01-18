use serde::{Deserialize, Serialize};
use validator::Validate;

/// Request payload for creating a new task
#[derive(Deserialize, Serialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskRequest {
    /// Task name
    #[validate(length(
        min = 1,
        max = 256,
        message = "Task name must be between 1 and 256 characters"
    ))]
    pub name: String, // Made public
    /// Docker image to use
    #[validate(length(min = 1, message = "Image name cannot be empty"))]
    pub image: String, // Made public

    /// Task parameters (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<std::collections::HashMap<String, String>>, // Made public

    /// Docker environment variables (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<std::collections::HashMap<String, String>>, // Made public
}

/// Response structure for task operations
#[derive(Deserialize, Serialize)]
pub struct TaskResponse {
    /// Unique task identifier
    pub id: String, // Made public
    /// Task name
    pub name: String, // Made public
    /// Current status of the task (pending, running, completed, failed)
    pub status: String, // Made public
    /// Docker image used
    pub image: String, // Made public
    /// Creation time of the task
    pub created: i64, // Made public
    /// Optional error message if task failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>, // Made public
}

/// Response structure for listing tasks
#[derive(Serialize)]
pub struct ListTasksResponse {
    pub tasks: Vec<TaskResponse>, // Made public
}
