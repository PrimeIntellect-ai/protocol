use crate::{api::server::AppState, models::node::NodeStatus};
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use log::error;
use serde_json::json;
use shared::models::{
    api::ApiResponse,
    heartbeat::{HeartbeatRequest, HeartbeatResponse},
};
use std::collections::HashSet;
use std::str::FromStr;

async fn heartbeat(
    heartbeat: web::Json<HeartbeatRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    let task_info = heartbeat.clone();
    let node_address = match Address::from_str(&heartbeat.address) {
        Ok(address) => address,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": "Invalid node address format"
            }));
        }
    };

    // Track heartbeat request in metrics
    app_state
        .metrics
        .increment_heartbeat_requests(&heartbeat.address);

    let node_opt = app_state
        .store_context
        .node_store
        .get_node(&node_address)
        .await;
    match node_opt {
        Ok(Some(node)) => {
            if node.status == NodeStatus::Banned {
                return HttpResponse::BadRequest().json(json!({
                    "success": false,
                    "error": "Node is banned"
                }));
            }
        }
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": "Node not found"
            }));
        }
    }
    if let Err(e) = app_state
        .store_context
        .node_store
        .update_node_task(node_address, task_info.task_id, task_info.task_state)
        .await
    {
        error!("Error updating node task: {}", e);
    }

    if let Some(p2p_id) = &heartbeat.p2p_id {
        if let Err(e) = app_state
            .store_context
            .node_store
            .update_node_p2p_id(&node_address, p2p_id)
            .await
        {
            error!("Error updating node p2p id: {}", e);
        }
    }

    if let Err(e) = app_state
        .store_context
        .heartbeat_store
        .beat(&heartbeat)
        .await
    {
        error!("Heartbeat Error: {}", e);
    }
    if let Some(metrics) = heartbeat.metrics.clone() {
        // Get all previously reported metrics for this node
        let previous_metrics = match app_state
            .store_context
            .metrics_store
            .get_metrics_for_node(node_address)
            .await
        {
            Ok(metrics) => metrics,
            Err(e) => {
                error!("Error getting metrics for node: {}", e);
                Default::default()
            }
        };

        // Create a HashSet of new metrics for efficient lookup
        let new_metrics_set: HashSet<_> = metrics
            .iter()
            .map(|metric| (&metric.key.task_id, &metric.key.label))
            .collect();

        // Clean up stale metrics from Redis only
        // The sync service will handle all Prometheus updates
        for (task_id, task_metrics) in previous_metrics {
            for (label, _value) in task_metrics {
                let prev_key = (&task_id, &label);
                if !new_metrics_set.contains(&prev_key) {
                    // Remove from Redis metrics store
                    if let Err(e) = app_state
                        .store_context
                        .metrics_store
                        .delete_metric(&task_id, &label, &node_address.to_string())
                        .await
                    {
                        error!("Error deleting metric: {}", e);
                    }
                }
            }
        }

        // Store new metrics in Redis only
        if let Err(e) = app_state
            .store_context
            .metrics_store
            .store_metrics(Some(metrics.clone()), node_address)
            .await
        {
            error!("Error storing metrics: {}", e);
        }
    }

    let current_task = app_state.scheduler.get_task_for_node(node_address).await;
    match current_task {
        Ok(Some(task)) => {
            let resp: HttpResponse = ApiResponse::new(
                true,
                HeartbeatResponse {
                    current_task: Some(task),
                },
            )
            .into();
            resp
        }
        _ => HttpResponse::Ok().json(ApiResponse::new(
            true,
            HeartbeatResponse { current_task: None },
        )),
    }
}

pub fn heartbeat_routes() -> Scope {
    web::scope("/heartbeat").route("", post().to(heartbeat))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use crate::metrics::sync_service::MetricsSyncService;
    use crate::models::node::OrchestratorNode;
    use crate::ServerMode;

    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use serde_json::json;
    use shared::models::metric::MetricEntry;
    use shared::models::metric::MetricKey;
    use shared::models::task::TaskRequest;

    #[actix_web::test]
    async fn test_heartbeat() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/heartbeat", web::post().to(heartbeat)),
        )
        .await;

        let address = "0x0000000000000000000000000000000000000000".to_string();
        let node_address = Address::from_str(&address).unwrap();
        let node = OrchestratorNode {
            address: node_address,
            status: NodeStatus::Healthy,
            ..Default::default()
        };
        let _ = app_state.store_context.node_store.add_node(node).await;
        let req_payload = json!({"address": address, "metrics": [
            {"key": {"task_id": "long-task-1234", "label": "performance/batch_avg_seq_length"}, "value": 1.0},
            {"key": {"task_id": "long-task-1234", "label": "performance/batch_min_seq_length"}, "value": 5.0}
        ]});

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&req_payload)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(json["current_task"], serde_json::Value::Null);

        let node_address = Address::from_str(&address).unwrap();

        let value = app_state
            .store_context
            .heartbeat_store
            .get_heartbeat(&node_address)
            .await
            .unwrap();

        assert_eq!(
            value,
            Some(HeartbeatRequest {
                address: "0x0000000000000000000000000000000000000000".to_string(),
                task_id: None,
                task_state: None,
                metrics: Some(vec![
                    MetricEntry {
                        key: MetricKey {
                            task_id: "long-task-1234".to_string(),
                            label: "performance/batch_avg_seq_length".to_string(),
                        },
                        value: 1.0,
                    },
                    MetricEntry {
                        key: MetricKey {
                            task_id: "long-task-1234".to_string(),
                            label: "performance/batch_min_seq_length".to_string(),
                        },
                        value: 5.0,
                    }
                ]),
                version: None,
                timestamp: None,
                p2p_id: None,
            })
        );

        // Verify metrics are stored in Redis (heartbeat API responsibility)
        let redis_metrics = app_state
            .store_context
            .metrics_store
            .get_metrics_for_node(node_address)
            .await
            .unwrap();
        assert!(redis_metrics.contains_key("long-task-1234"));
        assert!(redis_metrics["long-task-1234"].contains_key("performance/batch_avg_seq_length"));
        assert!(redis_metrics["long-task-1234"].contains_key("performance/batch_min_seq_length"));

        // Test metrics sync service: Redis -> Prometheus
        // Verify Prometheus registry is initially empty (no sync service has run)
        let prometheus_metrics_before = app_state.metrics.export_metrics().unwrap();
        assert!(!prometheus_metrics_before.contains("performance/batch_avg_seq_length"));
        assert!(prometheus_metrics_before.contains(&format!("orchestrator_heartbeat_requests_total{{node_address=\"0x0000000000000000000000000000000000000000\",pool_id=\"{}\"}} 1", app_state.metrics.pool_id)));

        // Create and run sync service manually to test the sync functionality
        let sync_service = MetricsSyncService::new(
            app_state.store_context.clone(),
            app_state.metrics.clone(),
            ServerMode::Full, // Test app uses Full mode
            10,
            None, // No node groups plugin in test
        );

        // Manually trigger a sync operation
        sync_service.sync_metrics_from_redis().await.unwrap();

        // Verify Prometheus registry now contains the metrics from Redis
        let prometheus_metrics_after = app_state.metrics.export_metrics().unwrap();
        assert!(prometheus_metrics_after.contains("performance/batch_avg_seq_length"));
        assert!(prometheus_metrics_after.contains("performance/batch_min_seq_length"));
        assert!(prometheus_metrics_after.contains("long-task-1234"));

        let heartbeat_two = json!({"address": address, "metrics": [
            {"key": {"task_id": "long-task-1235", "label": "performance/batch_len"}, "value": 10.0},
            {"key": {"task_id": "long-task-1235", "label": "performance/batch_min_len"}, "value": 50.0}
        ]});

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&heartbeat_two)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify new metrics in Redis and old metrics cleaned up
        let redis_metrics = app_state
            .store_context
            .metrics_store
            .get_metrics_for_node(node_address)
            .await
            .unwrap();
        assert!(redis_metrics.contains_key("long-task-1235"));
        assert!(redis_metrics["long-task-1235"].contains_key("performance/batch_len"));
        assert!(redis_metrics["long-task-1235"].contains_key("performance/batch_min_len"));
        assert!(!redis_metrics.contains_key("long-task-1234")); // Stale metrics cleaned up
        let aggregated_metrics = app_state
            .store_context
            .metrics_store
            .get_aggregate_metrics_for_all_tasks()
            .await
            .unwrap();
        assert_eq!(aggregated_metrics.len(), 2);
        assert_eq!(aggregated_metrics.get("performance/batch_len"), Some(&10.0));
        assert_eq!(
            aggregated_metrics.get("performance/batch_min_len"),
            Some(&50.0)
        );
        assert_eq!(
            aggregated_metrics.get("performance/batch_avg_seq_length"),
            None
        );
        sync_service.sync_metrics_from_redis().await.unwrap();
        let prometheus_metrics_after_two = app_state.metrics.export_metrics().unwrap();
        assert!(prometheus_metrics_after_two.contains("performance/batch_len"));
        assert!(prometheus_metrics_after_two.contains("performance/batch_min_len"));
        assert!(prometheus_metrics_after_two.contains("long-task-1235"));
        assert!(!prometheus_metrics_after_two.contains("long-task-1234"));

        let heartbeat_three = json!({"address": address, "metrics": [
        ]});

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&heartbeat_three)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify all metrics cleaned up from Redis
        let redis_metrics = app_state
            .store_context
            .metrics_store
            .get_metrics_for_node(node_address)
            .await
            .unwrap();
        assert!(redis_metrics.is_empty()); // All metrics for this node should be gone

        let aggregated_metrics = app_state
            .store_context
            .metrics_store
            .get_aggregate_metrics_for_all_tasks()
            .await
            .unwrap();
        assert_eq!(aggregated_metrics, HashMap::new());
    }

    #[actix_web::test]
    async fn test_heartbeat_with_task() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .route("/heartbeat", web::post().to(heartbeat)),
        )
        .await;

        let address = "0x0000000000000000000000000000000000000000".to_string();

        let task = TaskRequest {
            image: "test".to_string(),
            name: "test".to_string(),
            ..Default::default()
        };

        let node = OrchestratorNode {
            address: Address::from_str(&address).unwrap(),
            status: NodeStatus::Healthy,
            ..Default::default()
        };

        let _ = app_state.store_context.node_store.add_node(node).await;

        let task = match task.try_into() {
            Ok(task) => task,
            Err(e) => panic!("Failed to convert TaskRequest to Task: {}", e),
        };
        let _ = app_state.store_context.task_store.add_task(task).await;

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(json!({"address": "0x0000000000000000000000000000000000000000"}))
            .to_request();

        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["data"]["current_task"]["image"],
            serde_json::Value::String("test".to_string())
        );

        let node_address = Address::from_str(&address).unwrap();
        let value = app_state
            .store_context
            .heartbeat_store
            .get_heartbeat(&node_address)
            .await
            .unwrap();
        // Task has not started yet

        let heartbeat = HeartbeatRequest {
            address: "0x0000000000000000000000000000000000000000".to_string(),
            ..Default::default()
        };
        assert_eq!(value, Some(heartbeat));
    }
}
