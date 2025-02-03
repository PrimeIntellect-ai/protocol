use crate::api::server::AppState;
use actix_web::{
    web::{self, get, post, Data},
    HttpResponse, Scope,
};
use serde_json::json;

async fn get_logs(app_state: Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "logs": "Not implemented yet"
    }))
}

async fn restart_task(app_state: Data<AppState>) -> HttpResponse {
    // TODO: Implement actual node restart
    HttpResponse::Ok().json(json!({
        "status": "restart initiated"
    }))
}

pub fn task_routes() -> Scope {
    web::scope("/task")
        .route("/logs", get().to(get_logs))
        .route("/restart", post().to(restart_task))
}
