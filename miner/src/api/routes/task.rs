use super::types::ErrorResponse;
use crate::docker::handler::DockerHandler;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpResponse, Scope,
};
use log::{debug, info};
use serde::{Deserialize, Serialize};
use uuid;
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
    name: String,
    /// Docker image to use
    #[validate(length(min = 1, message = "Image name cannot be empty"))]
    image: String,

    /// Task parameters (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<std::collections::HashMap<String, String>>,

    /// Docker environment variables (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    env_vars: Option<std::collections::HashMap<String, String>>,
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
    /// Docker image used
    image: String,
    /// Creation time of the task
    created: i64,
    /// Optional error message if task failed
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Response structure for listing tasks
#[derive(Serialize)]
pub struct ListTasksResponse {
    tasks: Vec<TaskResponse>,
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
    // Extract task from JSON wrapper
    let task = task.into_inner();

    // Validate input
    if let Err(errors) = task.validate() {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: errors.to_string(),
        });
    }

    let task_id = format!("task_{}", uuid::Uuid::new_v4());
    let task_name = task.name.clone();
    let task_env_vars = task.env_vars.clone();

    debug!("Attempting to start container with task_id: {}", task_id);
    info!("Creating new task with ID: {}", task_id);

    // Start container for the task
    match docker_handler
        .start_container(&task.image, &task_id, task_env_vars)
        .await
    {
        Ok(_) => {
            let response = TaskResponse {
                id: task_id,
                name: task_name,
                status: "running".to_string(),
                error: None,
                image: task.image,
                created: 0,
            };
            HttpResponse::Accepted().json(response)
        }
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Failed to start task container: {}", e),
        }),
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
async fn get_task(path: web::Path<String>, docker_handler: Data<DockerHandler>) -> HttpResponse {
    let task_id = path.into_inner();

    // Validate task ID format
    if !task_id.starts_with("task_") {
        return HttpResponse::BadRequest().json(ErrorResponse {
            error: "Invalid task ID format".to_string(),
        });
    }

    match docker_handler.get_container_details(&task_id).await {
        Ok(container) => {
            let response = TaskResponse {
                id: task_id,
                name: container
                    .names
                    .get(0)
                    .map(|n| n.trim_start_matches('/').to_string())
                    .unwrap_or_default(),
                status: container.status,
                image: container.image,
                created: container.created,
                error: None,
            };
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            debug!("Failed to get container details: {}", e);
            HttpResponse::NotFound().json(ErrorResponse {
                error: format!("Task {} not found", task_id),
            })
        }
    }
}

/// List all running tasks
///
/// # Returns
/// - 200 OK: List of running tasks
/// - 500 Internal Server Error: Server-side error
async fn list_tasks(docker_handler: Data<DockerHandler>) -> HttpResponse {
    match docker_handler.list_running_containers().await {
        Ok(containers) => {
            debug!("Found {} total containers", containers.len());

            let tasks: Vec<TaskResponse> = containers
                .into_iter()
                .map(|c| {
                    let name = c
                        .names
                        .get(0)
                        .map(|n| n.trim_start_matches('/').to_string())
                        .unwrap_or_default();
                    debug!(
                        "Creating TaskResponse for container {} with name {}",
                        c.id, name
                    );
                    TaskResponse {
                        id: c
                            .names
                            .get(0)
                            .map(|n| n.trim_start_matches('/').to_string())
                            .unwrap_or(c.id),
                        name,
                        status: c.status,
                        image: c.image,
                        created: c.created,
                        error: None,
                    }
                })
                .collect();

            debug!("Returning {} tasks", tasks.len());
            HttpResponse::Ok().json(ListTasksResponse { tasks })
        }
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Failed to list tasks: {}", e),
        }),
    }
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
async fn delete_task(path: web::Path<String>, docker_handler: Data<DockerHandler>) -> HttpResponse {
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
                image: "".to_string(),
                created: 0,
            };
            HttpResponse::Ok().json(response)
        }
        Err(e) => HttpResponse::InternalServerError().json(ErrorResponse {
            error: format!("Failed to remove task container: {}", e),
        }),
    }
}

pub fn task_routes() -> Scope {
    // Create Docker handler once during route setup
    let docker_handler =
        Data::new(DockerHandler::new().expect("Failed to initialize Docker handler"));

    web::scope("/task")
        .app_data(docker_handler.clone()) // Clone the Arc internally
        .route("", post().to(create_task))
        .route("", get().to(list_tasks))
        .route("/{id}", get().to(get_task))
        .route("/{id}", delete().to(delete_task))
}
