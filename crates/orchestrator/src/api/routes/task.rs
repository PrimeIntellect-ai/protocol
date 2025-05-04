use crate::api::server::AppState;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpResponse, Scope,
};
use serde_json::json;
use shared::models::task::Task;
use shared::models::task::TaskRequest;

async fn get_current_task(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
    match task_store.get_current_task() {
        Some(task) => HttpResponse::Ok().json(json!({"success": true, "task": task})),
        None => HttpResponse::Ok().json(json!({"success": true, "task": null})),
    }
}

async fn get_all_tasks(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone();
    let tasks = task_store.get_all_tasks();
    HttpResponse::Ok().json(json!({"success": true, "tasks": tasks}))
}

async fn create_task(task: web::Json<TaskRequest>, app_state: Data<AppState>) -> HttpResponse {
    let task = Task::from(task.into_inner());
    let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
    task_store.add_task(task);
    HttpResponse::Ok().json(json!({"success": true, "task": "updated_task"}))
}

async fn delete_current_task(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
    task_store.delete_current_task();
    HttpResponse::Ok().json(json!({"success": true, "task": "deleted_task"}))
}

pub fn tasks_routes() -> Scope {
    web::scope("/tasks")
        .route("/current", get().to(get_current_task))
        .route("", get().to(get_all_tasks))
        .route("", post().to(create_task))
        .route("/current", delete().to(delete_current_task))
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
    async fn test_get_current_task() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks/current", get().to(get_current_task)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks/current").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn test_update_task() {
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
            command: None,
            args: None,
            env_vars: None,
        };
        let req = test::TestRequest::post()
            .uri("/tasks")
            .set_json(payload)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
        let task: Task = task_store.get_current_task().unwrap();
        assert_eq!(task.image, "test");
    }

    #[actix_web::test]
    async fn get_task_when_no_task_exists() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks/current", get().to(get_current_task)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks/current").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(json["task"], serde_json::Value::Null);
    }

    #[actix_web::test]
    async fn get_task_after_setting() {
        let app_state = create_test_app_state().await;

        let task: Task = TaskRequest {
            image: "test".to_string(),
            name: "test".to_string(),
            command: None,
            args: None,
            env_vars: None,
        }
        .into();

        let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
        task_store.add_task(task);

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks/current", get().to(get_current_task)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks/current").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["task"]["image"],
            serde_json::Value::String("test".to_string())
        );
    }

    #[actix_web::test]
    async fn test_delete_task() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks/current", delete().to(delete_current_task)),
        )
        .await;

        let task: Task = TaskRequest {
            image: "test".to_string(),
            name: "test".to_string(),
            command: None,
            args: None,
            env_vars: None,
        }
        .into();

        let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
        task_store.add_task(task);

        let req = test::TestRequest::delete()
            .uri("/tasks/current")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let task_value = task_store.get_current_task(); // Use TaskStore
        assert!(task_value.is_none());
    }

    #[actix_web::test]
    async fn test_multiple_tasks_sorting() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_all_tasks))
                .route("/tasks/current", get().to(get_current_task)),
        )
        .await;

        let task_store = app_state.store_context.task_store.clone();

        // Create first task
        let task1: Task = TaskRequest {
            image: "test1".to_string(),
            name: "test1".to_string(),
            command: None,
            args: None,
            env_vars: None,
        }
        .into();
        task_store.add_task(task1.clone());

        // Wait briefly to ensure different timestamps
        thread::sleep(Duration::from_millis(10));

        // Create second task
        let task2: Task = TaskRequest {
            image: "test2".to_string(),
            name: "test2".to_string(),
            command: None,
            args: None,
            env_vars: None,
        }
        .into();
        task_store.add_task(task2.clone());

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

        // Test get current task - should return newest task
        let req = test::TestRequest::get().uri("/tasks/current").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["task"]["image"], "test2");
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
                command: None,
                args: None,
                env_vars: None,
            }
            .into();
            task_store.add_task(task);
            thread::sleep(Duration::from_millis(10));
        }

        let tasks = task_store.get_all_tasks();

        // Verify tasks are ordered by created_at descending
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].image, "test3");
        assert_eq!(tasks[1].image, "test2");
        assert_eq!(tasks[2].image, "test1");

        // Verify current task is the newest one
        let current = task_store.get_current_task().unwrap();
        assert_eq!(current.image, "test3");
    }
}
