use crate::api::server::AppState;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpResponse, Scope,
};
use serde_json::json;
use shared::models::task::Task;
use shared::models::task::TaskRequest;

#[utoipa::path(
    get,
    path = "/tasks",
    responses(
        (status = 200, description = "List of all tasks retrieved successfully", body = Vec<Task>),
        (status = 500, description = "Internal server error")
    ),
    tag = "tasks"
)]
async fn get_all_tasks(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone();
    let tasks = task_store.get_all_tasks().await;
    match tasks {
        Ok(tasks) => HttpResponse::Ok().json(json!({"success": true, "tasks": tasks})),
        Err(e) => HttpResponse::InternalServerError()
            .json(json!({"success": false, "error": e.to_string()})),
    }
}

#[utoipa::path(
    post,
    path = "/tasks",
    request_body = TaskRequest,
    responses(
        (status = 200, description = "Task created successfully", body = Task),
        (status = 400, description = "Bad request - task name already exists or validation failed"),
        (status = 500, description = "Internal server error")
    ),
    tag = "tasks"
)]
async fn create_task(task: web::Json<TaskRequest>, app_state: Data<AppState>) -> HttpResponse {
    let task_request = task.into_inner();
    let task_store = app_state.store_context.task_store.clone();

    // Check if a task with the same name already exists
    match task_store.task_name_exists(&task_request.name).await {
        Ok(exists) => {
            if exists {
                return HttpResponse::BadRequest()
                    .json(json!({"success": false, "error": format!("Task with name '{}' already exists", task_request.name)}));
            }
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(
                json!({"success": false, "error": format!("Failed to check task name: {}", e)}),
            );
        }
    }

    let task = match Task::try_from(task_request) {
        Ok(task) => task,
        Err(e) => {
            return HttpResponse::BadRequest()
                .json(json!({"success": false, "error": e.to_string()}));
        }
    };

    if let Some(group_plugin) = &app_state.node_groups_plugin {
        match group_plugin.get_task_topologies(&task) {
            Ok(topology) => {
                if topology.is_empty() {
                    return HttpResponse::BadRequest().json(json!({"success": false, "error": "No topology found for task but grouping plugin is active."}));
                }
            }
            Err(e) => {
                return HttpResponse::BadRequest()
                    .json(json!({"success": false, "error": e.to_string()}));
            }
        }
    }

    if let Err(e) = task_store.add_task(task.clone()).await {
        return HttpResponse::InternalServerError()
            .json(json!({"success": false, "error": e.to_string()}));
    }
    HttpResponse::Ok().json(json!({"success": true, "task": task}))
}

#[utoipa::path(
    delete,
    path = "/tasks/{id}",
    params(
        ("id" = String, Path, description = "Task ID to delete")
    ),
    responses(
        (status = 200, description = "Task deleted successfully"),
        (status = 500, description = "Internal server error")
    ),
    tag = "tasks"
)]
async fn delete_task(id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone();
    if let Err(e) = task_store.delete_task(id.into_inner()).await {
        return HttpResponse::InternalServerError()
            .json(json!({"success": false, "error": e.to_string()}));
    }
    HttpResponse::Ok().json(json!({"success": true}))
}

#[utoipa::path(
    delete,
    path = "/tasks/all",
    responses(
        (status = 200, description = "All tasks deleted successfully"),
        (status = 500, description = "Internal server error")
    ),
    tag = "tasks"
)]
async fn delete_all_tasks(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone();
    if let Err(e) = task_store.delete_all_tasks().await {
        return HttpResponse::InternalServerError()
            .json(json!({"success": false, "error": e.to_string()}));
    }
    HttpResponse::Ok().json(json!({"success": true}))
}

pub fn tasks_routes() -> Scope {
    web::scope("/tasks")
        .route("", get().to(get_all_tasks))
        .route("", post().to(create_task))
        .route("/all", delete().to(delete_all_tasks))
        .route("/{id}", delete().to(delete_task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use shared::models::task::Task;
    use std::thread;
    use std::time::Duration;

    #[actix_web::test]
    async fn test_create_task() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", post().to(create_task)),
        )
        .await;

        let payload = TaskRequest {
            image: "test".to_string(),
            name: "test".to_string(),
            ..Default::default()
        };
        let req = test::TestRequest::post()
            .uri("/tasks")
            .set_json(payload)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert!(json["task"]["id"].is_string());
        assert_eq!(json["task"]["image"], "test");
    }

    #[actix_web::test]
    async fn get_task_when_no_task_exists() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_all_tasks)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(json["tasks"], serde_json::Value::Array(vec![]));
    }

    #[actix_web::test]
    async fn get_task_after_setting() {
        let app_state = create_test_app_state().await;

        let task: Task = TaskRequest {
            image: "test".to_string(),
            name: "test".to_string(),
            ..Default::default()
        }
        .try_into()
        .unwrap();

        let task_store = app_state.store_context.task_store.clone();
        let _ = task_store.add_task(task).await;

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_all_tasks)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["tasks"][0]["image"],
            serde_json::Value::String("test".to_string())
        );
    }

    #[actix_web::test]
    async fn test_delete_task() {
        let app_state = create_test_app_state().await;
        let task: Task = TaskRequest {
            image: "test".to_string(),
            name: "test".to_string(),
            ..Default::default()
        }
        .try_into()
        .unwrap();

        let task_store = app_state.store_context.task_store.clone();
        let _ = task_store.add_task(task.clone()).await;

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks/{id}", delete().to(delete_task)),
        )
        .await;

        let req = test::TestRequest::delete()
            .uri(&format!("/tasks/{}", task.id))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let tasks = task_store.get_all_tasks().await.unwrap();
        assert_eq!(tasks.len(), 0);
    }

    #[actix_web::test]
    async fn test_multiple_tasks_sorting() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_all_tasks)),
        )
        .await;

        let task_store = app_state.store_context.task_store.clone();

        // Create first task
        let task1: Task = TaskRequest {
            image: "test1".to_string(),
            name: "test1".to_string(),
            ..Default::default()
        }
        .try_into()
        .unwrap();
        let _ = task_store.add_task(task1.clone()).await;

        // Wait briefly to ensure different timestamps
        thread::sleep(Duration::from_millis(10));

        // Create second task
        let task2: Task = TaskRequest {
            image: "test2".to_string(),
            name: "test2".to_string(),
            ..Default::default()
        }
        .try_into()
        .unwrap();
        let _ = task_store.add_task(task2.clone()).await;

        // Test get all tasks - should be sorted by created_at descending
        let req = test::TestRequest::get().uri("/tasks").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tasks = json["tasks"].as_array().unwrap();

        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0]["image"], "test2");
        assert_eq!(tasks[1]["image"], "test1");
    }

    #[actix_web::test]
    async fn test_task_ordering_with_multiple_additions() {
        let app_state = create_test_app_state().await;
        let task_store = app_state.store_context.task_store.clone();

        // Add tasks in sequence with delays
        for i in 1..=3 {
            let task: Task = TaskRequest {
                image: format!("test{}", i),
                name: format!("test{}", i),
                ..Default::default()
            }
            .try_into()
            .unwrap();
            let _ = task_store.add_task(task).await;
            thread::sleep(Duration::from_millis(10));
        }

        let tasks = task_store.get_all_tasks().await.unwrap();

        // Verify tasks are ordered by created_at descending
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].image, "test3");
        assert_eq!(tasks[1].image, "test2");
        assert_eq!(tasks[2].image, "test1");
    }

    #[actix_web::test]
    async fn test_create_task_with_metadata() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", post().to(create_task)),
        )
        .await;

        let mut labels = std::collections::HashMap::new();
        labels.insert("model".to_string(), "qwen3-4b".to_string());
        labels.insert("dataset".to_string(), "intellect-2-rl-dataset".to_string());
        labels.insert("version".to_string(), "v1".to_string());

        let payload = TaskRequest {
            image: "primeintellect/prime-rl:main".to_string(),
            name: "Qwen3-4B:INTELLECT-2-RL-Dataset".to_string(),
            metadata: Some(shared::models::task::TaskMetadata {
                labels: Some(labels),
            }),
            ..Default::default()
        };

        let req = test::TestRequest::post()
            .uri("/tasks")
            .set_json(payload)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert!(json["task"]["id"].is_string());
        assert_eq!(json["task"]["image"], "primeintellect/prime-rl:main");
        assert_eq!(json["task"]["name"], "Qwen3-4B:INTELLECT-2-RL-Dataset");

        // Verify metadata is preserved
        assert!(json["task"]["metadata"].is_object());
        assert!(json["task"]["metadata"]["labels"].is_object());
        assert_eq!(json["task"]["metadata"]["labels"]["model"], "qwen3-4b");
        assert_eq!(
            json["task"]["metadata"]["labels"]["dataset"],
            "intellect-2-rl-dataset"
        );
        assert_eq!(json["task"]["metadata"]["labels"]["version"], "v1");
    }

    #[actix_web::test]
    async fn test_create_task_with_duplicate_name() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", post().to(create_task)),
        )
        .await;

        // Create first task
        let payload = TaskRequest {
            image: "test".to_string(),
            name: "duplicate-test".to_string(),
            ..Default::default()
        };
        let req = test::TestRequest::post()
            .uri("/tasks")
            .set_json(&payload)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // Try to create second task with same name
        let req2 = test::TestRequest::post()
            .uri("/tasks")
            .set_json(&payload)
            .to_request();
        let resp2 = test::call_service(&app, req2).await;
        assert_eq!(resp2.status(), StatusCode::BAD_REQUEST);

        let body = test::read_body(resp2).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(false));
        assert!(json["error"]
            .as_str()
            .unwrap()
            .contains("Task with name 'duplicate-test' already exists"));
    }
}
