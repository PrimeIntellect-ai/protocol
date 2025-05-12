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
    task::{Task, TaskRequest},
};
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
        task_info.task_id.clone(),
        task_info.task_state.clone(),
    );

    if let Some(p2p_id) = &heartbeat.p2p_id {
        app_state
            .store_context
            .node_store
            .update_node_p2p_id(&node_address, p2p_id);
    }

    app_state.store_context.heartbeat_store.beat(&heartbeat);
    app_state
        .store_context
        .metrics_store
        .store_metrics(heartbeat.metrics.clone(), node_address);

    if let (Some(ref completed_task_id_str), Some(ref state_str)) =
        (&task_info.task_id, &task_info.task_state)
    {
        if state_str.eq_ignore_ascii_case("COMPLETED") {
            match uuid::Uuid::parse_str(completed_task_id_str) {
                Ok(completed_task_id_uuid) => {
                    let task_store = app_state.store_context.task_store.clone();
                    if let Some(completed_task_details) =
                        task_store.get_task_by_id(&completed_task_id_uuid)
                    {
                        if completed_task_details.auto_restart {
                            log::info!(
                                "Task {} reported as COMPLETED by node {}, auto_restart is true. Re-creating task.",
                                completed_task_id_str,
                                node_address
                            );

                            let new_task_request = TaskRequest {
                                image: completed_task_details.image,
                                name: completed_task_details.name,
                                env_vars: completed_task_details.env_vars,
                                command: completed_task_details.command,
                                args: completed_task_details.args,
                                auto_restart: Some(true),
                            };

                            let new_task_to_add = Task::from(new_task_request);

                            task_store.add_task(new_task_to_add.clone());
                            log::info!(
                                "New task {} (name: '{}', image: '{}') created for auto-restart, originating from completed task {}.",
                                new_task_to_add.id,
                                new_task_to_add.name,
                                new_task_to_add.image,
                                completed_task_id_str
                            );
                        } else {
                            log::debug!(
                                "Task {} reported as COMPLETED by node {}, but auto_restart is false.",
                                completed_task_id_str,
                                node_address
                            );
                        }
                    } else {
                        log::warn!(
                            "Task {} reported as COMPLETED by node {}, but its details were not found in TaskStore. Cannot check auto_restart.",
                            completed_task_id_str,
                            node_address
                        );
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Could not parse task_id '{}' from heartbeat (node {}) into UUID for auto-restart check: {}",
                        completed_task_id_str,
                        node_address,
                        e
                    );
                }
            }
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
    use super::*;
    use crate::api::tests::helper::create_test_app_state;
    use crate::models::node::OrchestratorNode;
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use serde_json::json;
    use shared::models::task::{Task, TaskRequest, TaskState};

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
        let req_payload = json!({"address": address});

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
                metrics: None,
                version: None,
                timestamp: None,
                p2p_id: None,
            })
        );
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
            command: None,
            args: None,
            env_vars: None,
            auto_restart: None,
        };
        app_state.store_context.task_store.add_task(task.into());

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
            task_id: None,
            task_state: None,
            metrics: None,
            version: None,
            timestamp: None,
            p2p_id: None,
        };
        assert_eq!(value, heartbeat);
    }

    #[actix_web::test]
    async fn test_heartbeat_restarts_task_when_completed_and_autorestart_true() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .service(heartbeat_routes()),
        )
        .await;

        let node_addr_str = "0x1111111111111111111111111111111111111111";
        let node = OrchestratorNode {
            address: Address::from_str(node_addr_str).unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy, // Assuming node is healthy
            task_id: None,
            task_state: None,
            version: None,
            p2p_id: None,
            last_status_change: None,
        };
        app_state.store_context.node_store.add_node(node);

        let original_task_req = TaskRequest {
            image: "restartable_image".to_string(),
            name: "restartable_task".to_string(),
            command: Some("echo".to_string()),
            args: Some(vec!["done".to_string()]),
            env_vars: None,
            auto_restart: Some(true),
        };
        let original_task = Task::from(original_task_req);
        app_state
            .store_context
            .task_store
            .add_task(original_task.clone());

        let initial_task_count = app_state.store_context.task_store.get_all_tasks().len();
        assert_eq!(initial_task_count, 1);

        let heartbeat_payload = HeartbeatRequest {
            address: node_addr_str.to_string(),
            task_id: Some(original_task.id.to_string()),
            task_state: Some("COMPLETED".to_string()),
            metrics: None,
            version: None,
            timestamp: None,
            p2p_id: None,
        };

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&heartbeat_payload)
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let tasks_after_heartbeat = app_state.store_context.task_store.get_all_tasks();
        assert_eq!(
            tasks_after_heartbeat.len(),
            initial_task_count + 1,
            "A new task should have been created"
        );

        let restarted_task = tasks_after_heartbeat
            .iter()
            .find(|t| t.id != original_task.id && t.name == original_task.name)
            .expect("Restarted task not found or name mismatch");

        assert_eq!(restarted_task.image, original_task.image);
        assert_eq!(restarted_task.state, TaskState::PENDING);
        assert!(
            restarted_task.auto_restart,
            "Restarted task should also have auto_restart true"
        );
        assert_ne!(
            restarted_task.id, original_task.id,
            "Restarted task must have a new ID"
        );

        let original_task_in_store_after = app_state
            .store_context
            .task_store
            .get_task_by_id(&original_task.id);
        assert!(
            original_task_in_store_after.is_some(),
            "Original task should still exist"
        );
    }

    #[actix_web::test]
    async fn test_heartbeat_does_not_restart_task_when_autorestart_false() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .service(heartbeat_routes()),
        )
        .await;

        let node_addr_str = "0x2222222222222222222222222222222222222222";
        let node = OrchestratorNode {
            address: Address::from_str(node_addr_str).unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
            version: None,
            p2p_id: None,
            last_status_change: None,
        };
        app_state.store_context.node_store.add_node(node);

        let task_req = TaskRequest {
            image: "norestart_image".to_string(),
            name: "norestart_task".to_string(),
            command: None,
            args: None,
            env_vars: None,
            auto_restart: Some(false), // Key for this test
        };
        let task_to_complete = Task::from(task_req);
        app_state
            .store_context
            .task_store
            .add_task(task_to_complete.clone());

        let initial_task_count = app_state.store_context.task_store.get_all_tasks().len();

        let heartbeat_payload = HeartbeatRequest {
            address: node_addr_str.to_string(),
            task_id: Some(task_to_complete.id.to_string()),
            task_state: Some("COMPLETED".to_string()),
            metrics: None,
            version: None,
            timestamp: None,
            p2p_id: None,
        };

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&heartbeat_payload)
            .to_request();
        test::call_service(&app, req).await;

        let tasks_after_heartbeat = app_state.store_context.task_store.get_all_tasks();
        assert_eq!(
            tasks_after_heartbeat.len(),
            initial_task_count,
            "No new task should have been created"
        );
    }

    #[actix_web::test]
    async fn test_heartbeat_does_not_restart_task_when_state_not_completed() {
        let app_state = create_test_app_state().await;
        let app = test::init_service(
            App::new()
                .app_data(app_state.clone())
                .service(heartbeat_routes()),
        )
        .await;

        let node_addr_str = "0x3333333333333333333333333333333333333333";
        let node = OrchestratorNode {
            address: Address::from_str(node_addr_str).unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status: NodeStatus::Healthy,
            task_id: None,
            task_state: None,
            version: None,
            p2p_id: None,
            last_status_change: None,
        };
        app_state.store_context.node_store.add_node(node);

        let task_req = TaskRequest {
            image: "running_image".to_string(),
            name: "running_task".to_string(),
            command: None,
            args: None,
            env_vars: None,
            auto_restart: Some(true),
        };
        let running_task = Task::from(task_req);
        app_state
            .store_context
            .task_store
            .add_task(running_task.clone());
        let initial_task_count = app_state.store_context.task_store.get_all_tasks().len();

        let heartbeat_payload = HeartbeatRequest {
            address: node_addr_str.to_string(),
            task_id: Some(running_task.id.to_string()),
            task_state: Some("RUNNING".to_string()),
            metrics: None,
            version: None,
            timestamp: None,
            p2p_id: None,
        };

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(&heartbeat_payload)
            .to_request();
        test::call_service(&app, req).await;

        let tasks_after_heartbeat = app_state.store_context.task_store.get_all_tasks();
        assert_eq!(
            tasks_after_heartbeat.len(),
            initial_task_count,
            "No new task should have been created for non-completed state"
        );
    }
}
