use crate::api::server::AppState;
use crate::types::ORCHESTRATOR_HEARTBEAT_KEY;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use redis::Commands;
use shared::models::heartbeat::{HeartbeatRequest, HeartbeatResponse};

async fn heartbeat(
    heartbeat: web::Json<HeartbeatRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    println!("Heartbeat incoming for address: {}", heartbeat.address);
    let mut con = app_state.store.client.get_connection().unwrap();
    let key = format!("{}:{}", ORCHESTRATOR_HEARTBEAT_KEY, heartbeat.address);

    let current_task = app_state.task_store.get_task();
    let _: () = con
        .set_options(
            &key,
            "1",
            redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(60)),
        )
        .unwrap();
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

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(json!({"address": "0x0000000000000000000000000000000000000000"}))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(json["current_task"], serde_json::Value::Null);

        let mut con = app_state.store.client.get_connection().unwrap();
        let key = "orchestrator:heartbeat:0x0000000000000000000000000000000000000000:heartbeat";
        let value: Option<String> = con.get(key).unwrap();
        assert_eq!(value, None);
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

        let task = TaskRequest {
            image: "test".to_string(),
        };
        app_state.task_store.set_task(task.into());

        let req = test::TestRequest::post()
            .uri("/heartbeat")
            .set_json(json!({"address": "0x0000000000000000000000000000000000000000"}))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = test::read_body(resp).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        println!("json: {:?}", json);
        assert_eq!(json["success"], serde_json::Value::Bool(true));
        assert_eq!(
            json["current_task"]["image"],
            serde_json::Value::String("test".to_string())
        );
    }
}
