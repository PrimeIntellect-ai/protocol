use crate::{api::server::AppState, models::node::NodeStatus};
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
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
    let node_address = Address::from_str(&heartbeat.address).unwrap();
    let node = app_state.store_context.node_store.get_node(&node_address);
    if let Some(node) = node {
        if node.status == NodeStatus::Banned {
            return HttpResponse::BadRequest().json(json!({
                "success": false,
                "error": "Node is banned"
            }));
        }
    }

    app_state.store_context.node_store.update_node_task(
        node_address,
        task_info.task_id,
        task_info.task_state,
    );

    if let Some(p2p_id) = &heartbeat.p2p_id {
        app_state
            .store_context
            .node_store
            .update_node_p2p_id(&node_address, p2p_id);
    }

    app_state.store_context.heartbeat_store.beat(&heartbeat);
    if let Some(metrics) = heartbeat.metrics.clone() {
        // Get all previously reported metrics for this node
        let previous_metrics = app_state
            .store_context
            .metrics_store
            .get_metrics_for_node(node_address);

        // Create a HashSet of new metrics for efficient lookup
        let new_metrics_set: HashSet<_> = metrics
            .iter()
            .map(|metric| (&metric.key.task_id, &metric.key.label))
            .collect();

        // Clean up stale metrics
        for (task_id, task_metrics) in previous_metrics {
            for (label, _value) in task_metrics {
                let prev_key = (&task_id, &label);
                if !new_metrics_set.contains(&prev_key) {
                    // Remove from Prometheus metrics
                    app_state.metrics.remove_compute_task_gauge(
                        &node_address.to_string(),
                        &task_id,
                        &label,
                    );
                    // Remove from Redis metrics store
                    app_state.store_context.metrics_store.delete_metric(
                        &task_id,
                        &label,
                        &node_address.to_string(),
                    );
                }
            }
        }

        // Store new metrics and update Prometheus
        app_state
            .store_context
            .metrics_store
            .store_metrics(Some(metrics.clone()), node_address);

        for metric in metrics {
            app_state.metrics.record_compute_task_gauge(
                &node_address.to_string(),
                &metric.key.task_id,
                &metric.key.label,
                metric.value,
            );
        }
    }

    let current_task = app_state.scheduler.get_task_for_node(node_address);
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
            .get_heartbeat(&node_address);
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

        let metrics = app_state.metrics.export_metrics().unwrap();
        assert!(metrics.contains("performance/batch_avg_seq_length"));
        assert!(metrics.contains("performance/batch_min_seq_length"));
        assert!(metrics.contains("long-task-1234"));

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

        let metrics = app_state.metrics.export_metrics().unwrap();
        assert!(metrics.contains("long-task-1235"));
        assert!(metrics.contains("performance/batch_len"));
        assert!(metrics.contains("performance/batch_min_len"));
        assert!(!metrics.contains("long-task-1234"));
        let aggregated_metrics = app_state
            .store_context
            .metrics_store
            .get_aggregate_metrics_for_all_tasks();
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

        let heartbeat_three = json!({"address": address, "metrics": [
        ]});

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&heartbeat_three)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let metrics = app_state.metrics.export_metrics().unwrap();
        let aggregated_metrics = app_state
            .store_context
            .metrics_store
            .get_aggregate_metrics_for_all_tasks();
        assert_eq!(aggregated_metrics, HashMap::new());
        assert_eq!(metrics, "");
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
        let task = match task.try_into() {
            Ok(task) => task,
            Err(e) => panic!("Failed to convert TaskRequest to Task: {}", e),
        };
        app_state.store_context.task_store.add_task(task);

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(json!({"address": "0x0000000000000000000000000000000000000000"}))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

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
            .get_heartbeat(&node_address);
        // Task has not started yet

        let value = value.unwrap();
        let heartbeat = HeartbeatRequest {
            address: "0x0000000000000000000000000000000000000000".to_string(),
            ..Default::default()
        };
        assert_eq!(value, heartbeat);
    }
}
