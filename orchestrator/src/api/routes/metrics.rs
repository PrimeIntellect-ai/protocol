use crate::api::server::AppState;
use actix_web::{
    web::{self, get, Data},
    HttpResponse, Scope,
};
use serde_json::json;

async fn get_metrics(app_state: Data<AppState>) -> HttpResponse {
    let metrics = app_state
        .store_context
        .metrics_store
        .get_aggregate_metrics_for_all_tasks();
    HttpResponse::Ok().json(json!({"success": true, "metrics": metrics}))
}

pub fn metrics_routes() -> Scope {
    web::scope("/metrics").route("", get().to(get_metrics))
}
