use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpRequest, HttpResponse, Scope,
};
use redis::{Commands, RedisResult};
use shared::{
    models::storage::RequestUploadRequest,
    utils::google_cloud::{generate_mapping_file, generate_upload_signed_url},
};
use std::time::Duration;

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

async fn request_upload(
    req: HttpRequest,
    request_upload: web::Json<RequestUploadRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    // Check file size limit first
    if request_upload.file_size > MAX_FILE_SIZE {
        return HttpResponse::PayloadTooLarge().json(serde_json::json!({
            "success": false,
            "error": format!("File size exceeds maximum allowed size of 100MB")
        }));
    }

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

    let task = match app_state
        .store_context
        .task_store
        .get_task(&request_upload.task_id)
    {
        Some(task) => task,
        None => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": format!("Task not found")
            }));
        }
    };

    let storage_config = task.storage_config;

    let mut file_name = request_upload.file_name.to_string();
    if let Some(storage_config) = storage_config {
        if let Some(file_name_template) = storage_config.file_name_template {
            file_name = generate_file_name(&file_name_template, &request_upload.file_name);
        }
    }

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
        &file_name,
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
        &file_name,
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
                        log::info!(
                            "Rate limit count for {}: {}/{} per hour",
                            address,
                            count,
                            app_state.hourly_upload_limit
                        );
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
            #[cfg(test)]
            return HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "signed_url": signed_url,
                "file_name": file_name
            }));
            #[cfg(not(test))]
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

fn generate_file_name(template: &str, original_name: &str) -> String {
    let mut file_name = template.to_string();
    file_name = file_name.replace("${original_name}", original_name);
    file_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use actix_web::{test, web::post, App};
    use shared::models::task::{StorageConfig, Task};
    use uuid::Uuid;

    #[tokio::test]
    async fn test_generate_file_name() {
        let template = "test/${original_name}";
        let original_name = "test";
        let file_name = generate_file_name(template, original_name);
        assert_eq!(file_name, "test/test");
    }

    #[actix_web::test]
    async fn test_request_upload_success() {
        let app_state = create_test_app_state().await;

        let task = Task {
            id: Uuid::new_v4(),
            image: "test-image".to_string(),
            name: "test-task".to_string(),
            storage_config: Some(StorageConfig {
                file_name_template: Some("model_123/user_uploads/${original_name}".to_string()),
            }),
            ..Default::default()
        };

        let task_store = app_state.store_context.task_store.clone();
        task_store.add_task(task.clone());

        assert!(task_store.get_task(&task.id.to_string()).is_some());

        if app_state.s3_credentials.is_none() {
            println!("S3 credentials not configured");
            return;
        }

        let app =
            test::init_service(App::new().app_data(app_state.clone()).service(
                web::scope("/storage").route("/request-upload", post().to(request_upload)),
            ))
            .await;

        let req = test::TestRequest::post()
            .uri("/storage/request-upload")
            .insert_header(("x-address", "test_address"))
            .set_json(&RequestUploadRequest {
                file_name: "test.parquet".to_string(),
                file_size: 1024,
                file_type: "application/octet-stream".to_string(),
                sha256: "test_sha256".to_string(),
                task_id: task.id.to_string(),
            })
            .to_request();

        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["file_name"],
            serde_json::Value::String("model_123/user_uploads/test.parquet".to_string())
        );
    }

    #[actix_web::test]
    async fn test_request_upload_invalid_task() {
        let app_state = create_test_app_state().await;

        if app_state.s3_credentials.is_none() {
            println!("S3 credentials not configured");
            return;
        }

        let app =
            test::init_service(App::new().app_data(app_state.clone()).service(
                web::scope("/storage").route("/request-upload", post().to(request_upload)),
            ))
            .await;

        let non_existing_task_id = Uuid::new_v4().to_string();
        let req = test::TestRequest::post()
            .uri("/storage/request-upload")
            .insert_header(("x-address", "test_address"))
            .set_json(&RequestUploadRequest {
                file_name: "test.txt".to_string(),
                file_size: 1024,
                file_type: "text/plain".to_string(),
                sha256: "test_sha256".to_string(),
                task_id: non_existing_task_id,
            })
            .to_request();

        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(false));
    }
}
