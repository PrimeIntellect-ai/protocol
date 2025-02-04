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
    match task_store.get_task() {
        Some(task) => HttpResponse::Ok().json(json!({"success": true, "task": task})),
        None => HttpResponse::Ok().json(json!({"success": false, "task": Option::<Task>::None})),
    }
}

async fn update_task(task: web::Json<TaskRequest>, app_state: Data<AppState>) -> HttpResponse {
    let task = Task::from(task.into_inner());
    let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
    task_store.set_task(task);
    HttpResponse::Ok().json(json!({"success": true, "task": "updated_task"}))
}

async fn delete_task(app_state: Data<AppState>) -> HttpResponse {
    let task_store = app_state.store_context.task_store.clone(); // Use TaskStore
    task_store.delete_task();
    HttpResponse::Ok().json(json!({"success": true, "task": "deleted_task"}))
}

pub fn tasks_routes() -> Scope {
    web::scope("/tasks")
        .route("", get().to(get_current_task))
        .route("", post().to(update_task))
        .route("", delete().to(delete_task))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use shared::models::task::Task;

    #[actix_web::test]
    async fn test_get_current_task() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_current_task)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn test_update_task() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", post().to(update_task)),
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
        let task: Task = task_store.get_task().unwrap();
        assert_eq!(task.image, "test");
    }

    #[actix_web::test]
    async fn get_task_when_no_task_exists() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_current_task)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(false));
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
        task_store.set_task(task);

        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/tasks", get().to(get_current_task)),
        )
        .await;

        let req = test::TestRequest::get().uri("/tasks").to_request();
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
                .route("/tasks", delete().to(delete_task)),
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
        task_store.set_task(task);

        let req = test::TestRequest::delete().uri("/tasks").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let task_value = task_store.get_task(); // Use TaskStore
        assert!(task_value.is_none());
    }
}
