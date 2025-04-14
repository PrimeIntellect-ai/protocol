use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpRequest, HttpResponse, Scope,
};
use redis::{Commands, RedisResult};
use shared::utils::google_cloud::{generate_mapping_file, generate_upload_signed_url};
use std::time::Duration;

#[derive(serde::Deserialize)]
pub struct RequestUploadRequest {
    pub file_name: String,
    pub file_size: u64,
    pub file_type: String,
    pub sha256: String,
}

async fn request_upload(
    req: HttpRequest,
    request_upload: web::Json<RequestUploadRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    let mut redis_con = match app_state.redis_store.client.get_connection() {
        Ok(con) => con,
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to connect to Redis: {}", e)
            }));
        }
    };

    let x_address = req
        .headers()
        .get("x-address")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    // Check rate limit before processing
    if let Some(address) = &x_address {
        let rate_limit_key = format!("rate_limit:storage:upload:{}", address);
        let hourly_limit = app_state.hourly_upload_limit;

        // Check current request count
        let current_count: RedisResult<Option<i64>> = redis_con.get(&rate_limit_key);

        match current_count {
            Ok(Some(count)) if count >= hourly_limit => {
                return HttpResponse::TooManyRequests().json(serde_json::json!({
                    "success": false,
                    "error": format!("Rate limit exceeded. Maximum {} uploads per hour.", hourly_limit)
                }));
            }
            Ok(Some(count)) => {
                log::info!(
                    "Current rate limit count for {}: {}/{} per hour",
                    address,
                    count,
                    hourly_limit
                );
            }
            Ok(None) => {
                log::info!("No rate limit record for {} yet", address);
            }
            Err(e) => {
                log::error!("Redis rate limiting error: {}", e);
                // Continue processing if rate limiting fails
            }
        }
    }

    let file_name = &request_upload.file_name;
    let file_size = &request_upload.file_size;
    let file_type = &request_upload.file_type;
    let sha256 = &request_upload.sha256;

    log::info!(
        "Received upload request for file: {}, size: {}, type: {}, sha256: {}",
        file_name,
        file_size,
        file_type,
        sha256
    );

    // Get credentials from app state
    let credentials = match &app_state.s3_credentials {
        Some(creds) => creds,
        None => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Storage credentials not configured"
            }))
        }
    };

    log::info!(
        "Generating mapping file for sha256: {} to file: {} in bucket: {}",
        sha256,
        file_name,
        app_state.bucket_name.clone().unwrap_or_default()
    );

    if let Err(e) = generate_mapping_file(
        app_state.bucket_name.clone().unwrap().as_str(),
        credentials,
        sha256,
        file_name,
    )
    .await
    {
        log::error!("Failed to generate mapping file: {}", e);
        return HttpResponse::InternalServerError().json(serde_json::json!({
            "success": false,
            "error": format!("Failed to generate mapping file: {}", e)
        }));
    }

    log::info!(
        "Successfully generated mapping file. Generating signed upload URL for file: {}",
        file_name
    );

    // Generate signed upload URL
    match generate_upload_signed_url(
        app_state.bucket_name.clone().unwrap().as_str(),
        file_name,
        credentials,
        Some(file_type.to_string()),
        Duration::from_secs(3600), // 1 hour expiry
        Some(*file_size),
    )
    .await
    {
        Ok(signed_url) => {
            // Increment rate limit counter after successful URL generation
            if let Some(address) = &x_address {
                let rate_limit_key = format!("rate_limit:storage:upload:{}", address);
                let expiry_seconds = 3600; // 1 hour

                // Increment the counter or create it if it doesn't exist
                let new_count: RedisResult<i64> = redis_con.incr(&rate_limit_key, 1);

                match new_count {
                    Ok(count) => {
                        // Set expiry if this is the first request (count == 1)
                        if count == 1 {
                            let _: RedisResult<()> =
                                redis_con.expire(&rate_limit_key, expiry_seconds);
                        }
                        log::info!("Rate limit count for {}: {}/12 per hour", address, count);
                    }
                    Err(e) => {
                        log::error!("Failed to update rate limit counter: {}", e);
                    }
                }
            }

            log::info!(
                "Successfully generated signed upload URL for file: {}",
                file_name
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "signed_url": signed_url
            }))
        }
        Err(e) => {
            log::error!("Failed to generate upload URL: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to generate upload URL: {}", e)
            }))
        }
    }
}

pub fn storage_routes() -> Scope {
    web::scope("/storage").route("/request-upload", post().to(request_upload))
}
