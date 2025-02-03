use crate::api::server::AppState;
use actix_web::{
    web::{self, get, post, Data},
    HttpResponse, Scope,
};
use serde_json::json;

async fn get_logs(app_state: Data<AppState>) -> HttpResponse {
    let logs = app_state.docker_service.get_logs().await;
    match logs {
        Ok(logs) => HttpResponse::Ok().json(json!({
            "success": true,
            "logs": logs
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": e.to_string()
        })),
    }
}

async fn restart_task(app_state: Data<AppState>) -> HttpResponse {
    let result = app_state.docker_service.restart_task().await;
    match result {
        Ok(_) => HttpResponse::Ok().json(json!({
            "success": true,
            "message": "Task restarted successfully"
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": e.to_string()
        })),
    }
}

pub fn task_routes() -> Scope {
    web::scope("/task")
        .route("/logs", get().to(get_logs))
        .route("/restart", post().to(restart_task))
}
