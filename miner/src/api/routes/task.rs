use actix_web::{
    web::{self, delete, get, post},
    HttpResponse, Scope,
};
use serde::{Deserialize, Serialize};
use uuid;
use validator::Validate;

/// Request payload for creating a new task
#[derive(Deserialize, Serialize, Validate)]
pub struct CreateTaskRequest {
    /// Task name
    #[validate(length(
        min = 1,
        max = 256,
        message = "Task name must be between 1 and 256 characters"
    ))]
    name: String,

    /// Task parameters (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<std::collections::HashMap<String, String>>,
}

/// Response structure for task operations
#[derive(Deserialize, Serialize)]
pub struct TaskResponse {
    /// Unique task identifier
    id: String,
    /// Task name
    name: String,
    /// Current status of the task (pending, running, completed, failed)
    status: String,
    /// Optional error message if task failed
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Error response for validation failures
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

/// Create a new task
///
/// # Request Body
/// - `name`: Task name
/// - `parameters`: Optional task parameters
///
/// # Returns
/// - 202 Accepted: Task created successfully
/// - 400 Bad Request: Invalid input
/// - 500 Internal Server Error: Server-side error
async fn create_task(task: web::Json<CreateTaskRequest>) -> HttpResponse {
    // Validate input
    if let Err(errors) = task.0.validate() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: errors.to_string(),
        });
    }

    // Create placeholder task response
    let response = TaskResponse {
        id: format!("task_{}", uuid::Uuid::new_v4()),
        name: task.name.clone(),
        status: "pending".to_string(),
        error: None,
    };

    HttpResponse::Accepted().json(response)
}

/// Get status of a task
///
/// # Path Parameters
/// - `id`: Task identifier
///
/// # Returns
/// - 200 OK: Task found
/// - 404 Not Found: Task not found
/// - 500 Internal Server Error: Server-side error
async fn get_task(path: web::Path<String>) -> HttpResponse {
    let task_id = path.into_inner();

    // Validate task ID format
    if !task_id.starts_with("task_") {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid task ID format".to_string(),
        });
    }

    // Return placeholder task status
    let response = TaskResponse {
        id: task_id,
        name: "Example Task".to_string(),
        status: "running".to_string(),
        error: None,
    };

    HttpResponse::Ok().json(response)
}

/// Delete a task
///
/// # Path Parameters
/// - `id`: Task identifier
///
/// # Returns
/// - 200 OK: Task deleted successfully
/// - 404 Not Found: Task not found
/// - 500 Internal Server Error: Server-side error
async fn delete_task(path: web::Path<String>) -> HttpResponse {
    let task_id = path.into_inner();

    // Validate task ID format
    if !task_id.starts_with("task_") {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid task ID format".to_string(),
        });
    }

    // Return placeholder deleted task response
    let response = TaskResponse {
        id: task_id,
        name: "Example Task".to_string(),
        status: "deleted".to_string(),
        error: None,
    };

    HttpResponse::Ok().json(response)
}

pub fn task_routes() -> Scope {
    web::scope("/task")
        .route("", post().to(create_task))
        .route("/{id}", get().to(get_task))
        .route("/{id}", delete().to(delete_task))
}
