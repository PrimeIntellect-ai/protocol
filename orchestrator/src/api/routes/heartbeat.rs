use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use alloy::primitives::Address;
use shared::models::heartbeat::{HeartbeatRequest, HeartbeatResponse};
use std::str::FromStr;

async fn heartbeat(
    heartbeat: web::Json<HeartbeatRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    println!("heartbeat {:?}", heartbeat);
    let task_info = heartbeat.clone();
    let node_address = Address::from_str(&heartbeat.address).unwrap();

    app_state.store_context.node_store.update_node_task(
        node_address,
        task_info.task_id,
        task_info.task_state,
    );
    app_state.store_context.heartbeat_store.beat(&heartbeat);
    app_state
        .store_context
        .metrics_store
        .store_metrics(heartbeat.metrics.clone(), node_address);
    let current_task = app_state.store_context.task_store.get_task();
    let resp: HttpResponse = HeartbeatResponse { current_task }.into();
    resp
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
        };
        app_state.store_context.task_store.set_task(task.into());

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
    }
}
