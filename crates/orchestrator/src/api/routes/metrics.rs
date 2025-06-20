use crate::api::server::AppState;
use actix_web::{
    web::{self, delete, get, post, Data, Path},
    HttpResponse, Scope,
};
use log::error;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct ManualMetricEntry {
    label: String,
    value: f64,
}

#[derive(Deserialize)]
struct DeleteMetricRequest {
    label: String,
    address: String,
}

async fn get_metrics(app_state: Data<AppState>) -> HttpResponse {
    let metrics = match app_state
        .store_context
        .metrics_store
        .get_aggregate_metrics_for_all_tasks()
        .await
    {
        Ok(metrics) => metrics,
        Err(e) => {
            error!("Error getting aggregate metrics for all tasks: {}", e);
            Default::default()
        }
    };
    HttpResponse::Ok().json(json!({"success": true, "metrics": metrics}))
}

async fn get_all_metrics(app_state: Data<AppState>) -> HttpResponse {
    let metrics = match app_state
        .store_context
        .metrics_store
        .get_all_metrics()
        .await
    {
        Ok(metrics) => metrics,
        Err(e) => {
            error!("Error getting all metrics: {}", e);
            Default::default()
        }
    };
    HttpResponse::Ok().json(json!({"success": true, "metrics": metrics}))
}

async fn get_prometheus_metrics(app_state: Data<AppState>) -> HttpResponse {
    match app_state.metrics.export_metrics() {
        Ok(metrics) => HttpResponse::Ok()
            .content_type("text/plain; version=0.0.4")
            .body(metrics),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "success": false,
            "error": format!("Failed to export metrics: {}", e)
        })),
    }
}

// for potential backup restore purposes
async fn create_metric(
    app_state: Data<AppState>,
    metric: web::Json<ManualMetricEntry>,
) -> HttpResponse {
    if let Err(e) = app_state
        .store_context
        .metrics_store
        .store_manual_metrics(metric.label.clone(), metric.value)
        .await
    {
        error!("Error storing manual metric: {}", e);
    }
    HttpResponse::Ok().json(json!({"success": true}))
}

async fn delete_metric(
    app_state: Data<AppState>,
    task_id: Path<String>,
    body: web::Json<DeleteMetricRequest>,
) -> HttpResponse {
    let success = match app_state
        .store_context
        .metrics_store
        .delete_metric(&task_id, &body.label, &body.address)
        .await
    {
        Ok(success) => success,
        Err(e) => {
            error!("Error deleting metric: {}", e);
            false
        }
    };

    HttpResponse::Ok().json(json!({
        "success": success
    }))
}

pub fn metrics_routes() -> Scope {
    web::scope("/metrics")
        .route("", get().to(get_metrics))
        .route("/all", get().to(get_all_metrics))
        .route("/prometheus", get().to(get_prometheus_metrics))
        .route("", post().to(create_metric))
        .route("/{task_id}", delete().to(delete_metric))
}
