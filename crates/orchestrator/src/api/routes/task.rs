use crate::api::server::AppState;
use actix_web::{
    web::{self, delete, get, post, Data},
    HttpResponse, Scope,
};
use serde_json::json;
use shared::models::task::Task;
use shared::models::task::TaskRequest;

async fn get_all_tasks(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone();
    let tasks = task_store.get_all_tasks();
    HttpResponse::Ok().json(json!({"success": true, "tasks": tasks}))
}

async fn create_task(task: web::Json<TaskRequest>, app_state: Data<AppState>) -> HttpResponse {
    let task = Task::from(task.into_inner());
    let task_store = app_state.store_context.task_store.clone();

    task_store.add_task(task.clone());
    HttpResponse::Ok().json(json!({"success": true, "task": task}))
}

async fn delete_task(id: web::Path<String>, app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone();
    task_store.delete_task(id.into_inner());
    HttpResponse::Ok().json(json!({"success": true}))
}

pub fn tasks_routes() -> Scope {
    web::scope("/tasks")
        .route("", get().to(get_all_tasks))
        .route("", post().to(create_task))
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
        .into();

        let task_store = app_state.store_context.task_store.clone();
        task_store.add_task(task);

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
        .into();

        let task_store = app_state.store_context.task_store.clone();
        task_store.add_task(task.clone());

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

        let tasks = task_store.get_all_tasks();
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
        .into();
        task_store.add_task(task1.clone());

        // Wait briefly to ensure different timestamps
        thread::sleep(Duration::from_millis(10));

        // Create second task
        let task2: Task = TaskRequest {
            image: "test2".to_string(),
            name: "test2".to_string(),
            ..Default::default()
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
    }
}
