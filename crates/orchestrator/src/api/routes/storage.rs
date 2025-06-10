use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpRequest, HttpResponse, Scope,
};
use redis::AsyncCommands;
use shared::models::storage::RequestUploadRequest;
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

    let mut redis_con = match app_state
        .redis_store
        .client
        .get_multiplexed_async_connection()
        .await
    {
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
    let current_count: Result<Option<i64>, redis::RedisError> =
        redis_con.get(&rate_limit_key).await;

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
        .await
    {
        Ok(Some(task)) => task,
        Ok(None) => {
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false,
                "error": format!("Task not found")
            }));
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Failed to retrieve task: {}", e)
            }));
        }
    };

    let storage_config = task.storage_config;

    let mut file_name = request_upload.file_name.to_string();
    let mut group_id = None;

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

                match plugin.get_node_group(address).await {
                    Ok(Some(group)) => {
                        group_id = Some(group.id.clone());
                        file_name = file_name.replace("${NODE_GROUP_ID}", &group.id);
                        file_name =
                            file_name.replace("${NODE_GROUP_SIZE}", &group.nodes.len().to_string());
                        let idx = plugin.get_idx_in_group(&group, address).unwrap();
                        file_name = file_name.replace("${NODE_GROUP_INDEX}", &idx.to_string());
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

    // Create a unique key for this file upload based on address, group_id, and file name
    let upload_key = match &group_id {
        Some(gid) => format!("upload:{}:{}:{}", address, gid, &request_upload.file_name),
        None => format!(
            "upload:{}:{}:{}",
            address, "no-group", &request_upload.file_name
        ),
    };
    let upload_exists: Result<Option<String>, redis::RedisError> = redis_con.get(&upload_key).await;
    if let Ok(None) = upload_exists {
        if let Err(e) = redis_con.set::<_, _, ()>(&upload_key, "pending").await {
            log::error!("Failed to set upload status in Redis: {}", e);
        }
    }
    let pattern = match &group_id {
        Some(gid) => format!("upload:{}:{}:*", address, gid),
        None => format!("upload:{}:no-group:*", address),
    };

    let total_uploads: Result<Vec<String>, redis::RedisError> = {
        let mut keys = Vec::new();
        match redis_con.scan_match(&pattern).await {
            Ok(mut iter) => {
                while let Some(key) = iter.next_item().await {
                    keys.push(key);
                }
                Ok(keys)
            }
            Err(e) => Err(e),
        }
    };

    let upload_count = match total_uploads {
        Ok(keys) => keys.len(),
        Err(e) => {
            log::error!("Failed to count uploads: {}", e);
            0
        }
    };
    let file_number = upload_count.saturating_sub(1);

    if file_name.contains("${TOTAL_UPLOAD_COUNT_AFTER}") {
        file_name = file_name.replace("${TOTAL_UPLOAD_COUNT_AFTER}", &upload_count.to_string());
    }
    if file_name.contains("${CURRENT_FILE_INDEX}") {
        file_name = file_name.replace("${CURRENT_FILE_INDEX}", &file_number.to_string());
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

    log::info!(
        "Generating mapping file for sha256: {} to file: {}",
        sha256,
        file_name,
    );

    if let Err(e) = app_state
        .storage_provider
        .generate_mapping_file(sha256, &file_name)
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
    match app_state
        .storage_provider
        .generate_upload_signed_url(
            &file_name,
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
            let new_count: Result<i64, redis::RedisError> =
                redis_con.incr(&rate_limit_key, 1).await;

            match new_count {
                Ok(count) => {
                    // Set expiry if this is the first request (count == 1)
                    if count == 1 {
                        let _: Result<(), redis::RedisError> =
                            redis_con.expire(&rate_limit_key, expiry_seconds).await;
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
    file_name = file_name.replace("${ORIGINAL_NAME}", original_name);
    file_name
}

#[cfg(test)]
mod tests {

    use std::collections::HashMap;

    use super::*;
    use crate::plugins::StatusUpdatePlugin;
    use crate::{
        api::tests::helper::{create_test_app_state, create_test_app_state_with_nodegroups},
        models::node::{NodeStatus, OrchestratorNode},
        plugins::node_groups::{NodeGroupConfiguration, NodeGroupsPlugin},
    };
    use actix_web::{test, web::post, App};
    use alloy::primitives::Address;
    use shared::models::task::{SchedulingConfig, StorageConfig, Task};
    use uuid::Uuid;

    #[tokio::test]
    async fn test_generate_file_name() {
        let template = "test/${ORIGINAL_NAME}";
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
                file_name_template: Some("model_123/user_uploads/${ORIGINAL_NAME}".to_string()),
            }),
            ..Default::default()
        };

        let task_store = app_state.store_context.task_store.clone();
        let _ = task_store.add_task(task.clone()).await;

        assert!(task_store
            .get_task(&task.id.to_string())
            .await
            .unwrap()
            .is_some());

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

        let _ = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await;

        let node_from_store = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .await
            .unwrap();
        assert!(node_from_store.is_some());
        if let Some(node_from_store) = node_from_store {
            assert_eq!(node.address, node_from_store.address);
        }

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
            None,
            None,
        );

        let _ = plugin
            .handle_status_change(&node, &NodeStatus::Healthy)
            .await;

        let task = Task {
            id: Uuid::new_v4(),
            image: "test-image".to_string(),
            name: "test-task".to_string(),
            storage_config: Some(StorageConfig {
                file_name_template: Some(
                    "model_xyz/dataset_1/${NODE_GROUP_ID}-${NODE_GROUP_SIZE}-${NODE_GROUP_INDEX}-${TOTAL_UPLOAD_COUNT_AFTER}-${CURRENT_FILE_INDEX}.parquet".to_string(),
                ),
            }),
            scheduling_config: Some(SchedulingConfig {
                plugins: Some(HashMap::from([(
                    "node_groups".to_string(),
                    HashMap::from([(
                        "allowed_topologies".to_string(),
                        vec!["test-config".to_string()],
                    )]),
                )])),
            }),
            ..Default::default()
        };

        let task_store = app_state.store_context.task_store.clone();
        let _ = task_store.add_task(task.clone()).await;
        let _ = plugin.test_try_form_new_groups().await;

        let group = plugin
            .get_node_group(&node.address.to_string())
            .await
            .unwrap();
        assert!(group.is_some());
        let group = group.unwrap();

        assert!(task_store
            .get_task(&task.id.to_string())
            .await
            .unwrap()
            .is_some());

        let app =
            test::init_service(App::new().app_data(app_state.clone()).service(
                web::scope("/storage").route("/request-upload", post().to(request_upload)),
            ))
            .await;

        // First request with test.parquet
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
            serde_json::Value::String(format!(
                "model_xyz/dataset_1/{}-{}-{}-{}-{}.parquet",
                group.id,
                group.nodes.len(),
                0,
                1,
                0
            ))
        );

        // Second request with same file name - should not increment count
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
            serde_json::Value::String(format!(
                "model_xyz/dataset_1/{}-{}-{}-{}-{}.parquet",
                group.id,
                group.nodes.len(),
                0,
                1,
                0
            ))
        );

        // Third request with different file name - should increment count
        let req = test::TestRequest::post()
            .uri("/storage/request-upload")
            .insert_header(("x-address", node.address.to_string()))
            .set_json(&RequestUploadRequest {
                file_name: "test2.parquet".to_string(),
                file_size: 1024,
                file_type: "application/octet-stream".to_string(),
                sha256: "test_sha256_2".to_string(),
                task_id: task.id.to_string(),
            })
            .to_request();

        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["file_name"],
            serde_json::Value::String(format!(
                "model_xyz/dataset_1/{}-{}-{}-{}-{}.parquet",
                group.id,
                group.nodes.len(),
                0,
                2,
                1
            ))
        );
    }

    #[actix_web::test]
    async fn test_upload_counter_without_node_group() {
        let app_state = create_test_app_state_with_nodegroups().await;

        let node = OrchestratorNode {
            address: Address::ZERO,
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            p2p_id: Some("test_p2p_id".to_string()),
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        let _ = app_state
            .store_context
            .node_store
            .add_node(node.clone())
            .await;

        let node_from_store = app_state
            .store_context
            .node_store
            .get_node(&node.address)
            .await
            .unwrap();
        assert!(node_from_store.is_some());
        if let Some(node) = node_from_store {
            assert_eq!(node.address, node.address);
        }

        let task = Task {
            id: Uuid::new_v4(),
            image: "test-image".to_string(),
            name: "test-task".to_string(),
            storage_config: Some(StorageConfig {
                file_name_template: Some(
                    "model_xyz/dataset_1/${TOTAL_UPLOAD_COUNT_AFTER}-${CURRENT_FILE_INDEX}.parquet"
                        .to_string(),
                ),
            }),
            ..Default::default()
        };

        let task_store = app_state.store_context.task_store.clone();
        let _ = task_store.add_task(task.clone()).await;

        assert!(task_store
            .get_task(&task.id.to_string())
            .await
            .unwrap()
            .is_some());

        let app =
            test::init_service(App::new().app_data(app_state.clone()).service(
                web::scope("/storage").route("/request-upload", post().to(request_upload)),
            ))
            .await;

        // First request with test.parquet
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

        // As defined in the storage template: model_xyz/dataset_1/${upload_count}-${file_number}.parquet
        // Upload count var is 1 while file number is 0
        assert_eq!(
            json["file_name"],
            serde_json::Value::String("model_xyz/dataset_1/1-0.parquet".to_string())
        );

        // Second request with same file name - should not increment count
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

        // Upload count var is 1 while file number is 0
        assert_eq!(
            json["file_name"],
            serde_json::Value::String("model_xyz/dataset_1/1-0.parquet".to_string())
        );

        // Third request with different file name - should increment count
        let req = test::TestRequest::post()
            .uri("/storage/request-upload")
            .insert_header(("x-address", node.address.to_string()))
            .set_json(&RequestUploadRequest {
                file_name: "test2.parquet".to_string(),
                file_size: 1024,
                file_type: "application/octet-stream".to_string(),
                sha256: "test_sha256_2".to_string(),
                task_id: task.id.to_string(),
            })
            .to_request();

        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["file_name"],
            serde_json::Value::String("model_xyz/dataset_1/2-1.parquet".to_string())
        );
    }
}
