use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use actix_web::{web, HttpRequest, HttpResponse, Responder};
use log::{error, info};
use serde_json::json;
use shared::models::api::ApiResponse;

use crate::{validator, SyntheticDataValidator};

pub(crate) struct State {
    pub synthetic_validator: Option<SyntheticDataValidator<shared::web3::wallet::WalletProvider>>,
    pub validator_health: Arc<tokio::sync::Mutex<validator::ValidatorHealth>>,
}

pub(crate) async fn health_check(_: HttpRequest, state: web::Data<State>) -> impl Responder {
    // Maximum allowed time between validation loops (2 minutes)
    const MAX_VALIDATION_INTERVAL_SECS: u64 = 120;

    let validator_health = state.validator_health.lock().await;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if validator_health.last_validation_timestamp() == 0 {
        // Validation hasn't run yet, but we're still starting up
        return HttpResponse::Ok().json(json!({
            "status": "starting",
            "message": "Validation loop hasn't started yet"
        }));
    }

    let elapsed = now - validator_health.last_validation_timestamp();

    if elapsed > MAX_VALIDATION_INTERVAL_SECS {
        return HttpResponse::ServiceUnavailable().json(json!({
        "status": "error",
        "message": format!("Validation loop hasn't run in {} seconds (max allowed: {})", elapsed, MAX_VALIDATION_INTERVAL_SECS),
        "last_loop_duration_ms": validator_health.last_loop_duration_ms(),
    }));
    }

    HttpResponse::Ok().json(json!({
        "status": "ok",
        "last_validation_seconds_ago": elapsed,
        "last_loop_duration_ms": validator_health.last_loop_duration_ms(),
    }))
}

pub(crate) async fn get_rejections(req: HttpRequest, state: web::Data<State>) -> impl Responder {
    match state.synthetic_validator.as_ref() {
        Some(synthetic_validator) => {
            // Parse query parameters
            let query = req.query_string();
            let limit = parse_limit_param(query).unwrap_or(100); // Default limit of 100

            let result = if limit > 0 && limit < 1000 {
                // Use the optimized recent rejections method for reasonable limits
                synthetic_validator
                    .get_recent_rejections(limit as isize)
                    .await
            } else {
                // Fallback to all rejections (but warn about potential performance impact)
                if limit >= 1000 {
                    info!("Large limit requested ({limit}), this may impact performance");
                }
                synthetic_validator.get_all_rejections().await
            };

            match result {
                Ok(rejections) => HttpResponse::Ok().json(ApiResponse {
                    success: true,
                    data: rejections,
                }),
                Err(e) => {
                    error!("Failed to get rejections: {e}");
                    HttpResponse::InternalServerError().json(ApiResponse {
                        success: false,
                        data: format!("Failed to get rejections: {e}"),
                    })
                }
            }
        }
        None => HttpResponse::ServiceUnavailable().json(ApiResponse {
            success: false,
            data: "Synthetic data validator not available",
        }),
    }
}

fn parse_limit_param(query: &str) -> Option<u32> {
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == "limit" {
                return value.parse::<u32>().ok();
            }
        }
    }
    None
}
