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
    app_state
        .store_context
        .metrics_store
        .store_metrics(heartbeat.metrics.clone(), node_address);

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
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use serde_json::json;
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
