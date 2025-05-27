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
    let address = req
        .headers()
        .get("x-address")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "Missing required x-address header"
            }))
        })
        .unwrap();

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

            // TODO: This is a temporary integration of node groups plugin functionality.
            // We have a plan to move this to proper expander traits that will handle
            // variable expansion in a more modular and extensible way. This current
            // implementation gets the job done but is not ideal from an architectural
            // perspective. The expander traits will allow us to:
            // 1. Register multiple expanders for different variable types
            // 2. Handle expansion in a more generic way
            // 3. Make testing easier by allowing mock expanders
            // 4. Support more complex variable expansion patterns
            // For now, this direct integration with node_groups_plugin works but will
            // be refactored in the future.
            if let Some(node_group_plugin) = &app_state.node_groups_plugin {
                let plugin = node_group_plugin.clone();

                match plugin.get_node_group(address) {
                    Ok(Some(group)) => {
                        file_name = file_name.replace("${node_group_id}", &group.id);
                        let idx = plugin.get_idx_in_group(&group, address).unwrap();
                        file_name = file_name.replace("${node_group_index}", &idx.to_string());
                    }
                    Ok(None) => {
                        log::warn!(
                            "Node group not found for address for upload request: {}",
                            address
                        );
                    }
                    Err(e) => {
                        log::error!("Error getting node group: {}", e);
                    }
                }
            }
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

            let rate_limit_key = format!("rate_limit:storage:upload:{}", address);
            let expiry_seconds = 3600; // 1 hour

            // Increment the counter or create it if it doesn't exist
            let new_count: RedisResult<i64> = redis_con.incr(&rate_limit_key, 1);

            match new_count {
                Ok(count) => {
                    // Set expiry if this is the first request (count == 1)
                    if count == 1 {
                        let _: RedisResult<()> = redis_con.expire(&rate_limit_key, expiry_seconds);
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
    use crate::plugins::StatusUpdatePlugin;
    use crate::{
        api::tests::helper::{create_test_app_state, create_test_app_state_with_nodegroups},
        models::node::{NodeStatus, OrchestratorNode},
        plugins::node_groups::{NodeGroupConfiguration, NodeGroupsPlugin},
    };
    use actix_web::{test, web::post, App};
    use alloy::primitives::Address;
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

    #[actix_web::test]
    async fn test_with_node_and_group() {
        let app_state = create_test_app_state_with_nodegroups().await;

        let node = OrchestratorNode {
            address: Address::ZERO,
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            p2p_id: Some("test_p2p_id".to_string()),
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        app_state.store_context.node_store.add_node(node.clone());

        let node_from_store = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .unwrap();
        assert_eq!(node_from_store.address, node.address);

        let config = NodeGroupConfiguration {
            name: "test-config".to_string(),
            min_group_size: 1,
            max_group_size: 1,
            compute_requirements: None,
        };

        let plugin = NodeGroupsPlugin::new(
            vec![config],
            app_state.redis_store.clone(),
            app_state.store_context.clone(),
        );

        let _ = plugin
            .handle_status_change(&node, &NodeStatus::Healthy)
            .await;

        let group = plugin.get_node_group(&node.address.to_string()).unwrap();
        assert!(group.is_some());
        let group = group.unwrap();

        let task = Task {
            id: Uuid::new_v4(),
            image: "test-image".to_string(),
            name: "test-task".to_string(),
            storage_config: Some(StorageConfig {
                file_name_template: Some(
                    "model_123/${node_group_id}-${node_group_index}-${original_name}".to_string(),
                ),
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
            .insert_header(("x-address", node.address.to_string()))
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
            serde_json::Value::String(format!("model_123/{}-{}-test.parquet", group.id, 0))
        );
    }
}
