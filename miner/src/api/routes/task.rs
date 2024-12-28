use super::types::ErrorResponse;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpResponse, Scope,
};
use serde::{Deserialize, Serialize};
use uuid;
use validator::Validate;
use crate::docker::handler::DockerHandler;
use log::{debug, info};

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
async fn create_task(
    task: web::Json<CreateTaskRequest>,
    docker_handler: Data<DockerHandler>,
) -> HttpResponse {
    // Validate input
    if let Err(errors) = task.validate() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: errors.to_string(),
        });
    }

    let task_id = format!("task_{}", uuid::Uuid::new_v4());
    let task_name = task.name.clone();
    let task_params = task.parameters.clone(); 

    debug!("Attempting to start container with task_id: {}", task_id);
    info!("Creating new task with ID: {}", task_id);

    // Start container for the task
    match docker_handler.start_container(
        "open-webui:4d6775d8830d",
        &task_id,
        task_params,
    ).await {
        Ok(_) => {
            let response = TaskResponse {
                id: task_id,
                name: task_name,
                status: "running".to_string(),
                error: None,
            };
            HttpResponse::Accepted().json(response)
        },
        Err(e) => {
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to start task container: {}", e),
            })
        }
    }
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
async fn get_task(
    path: web::Path<String>,
    docker_handler: Data<DockerHandler>,
) -> HttpResponse {
    let task_id = path.into_inner();

    // Validate task ID format
    if !task_id.starts_with("task_") {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid task ID format".to_string(),
        });
    }

    // TODO: Implement actual container status check
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
async fn delete_task(
    path: web::Path<String>,
    docker_handler: Data<DockerHandler>,
) -> HttpResponse {
    let task_id = path.into_inner();

    // Validate task ID format
    if !task_id.starts_with("task_") {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid task ID format".to_string(),
        });
    }

    // Remove the container
    match docker_handler.remove_container(&task_id).await {
        Ok(_) => {
            let response = TaskResponse {
                id: task_id,
                name: "Example Task".to_string(),
                status: "deleted".to_string(),
                error: None,
            };
            HttpResponse::Ok().json(response)
        },
        Err(e) => {
            HttpResponse::InternalServerError().json(ErrorResponse {
                error: format!("Failed to remove task container: {}", e),
            })
        }
    }
}

pub fn task_routes() -> Scope {
    // Create Docker handler once during route setup
    let docker_handler = Data::new(DockerHandler::new().expect("Failed to initialize Docker handler"));

    web::scope("/task")
        .app_data(docker_handler.clone()) // Clone the Arc internally
        .route("", post().to(create_task))
        .route("/{id}", get().to(get_task))
        .route("/{id}", delete().to(delete_task))
}
