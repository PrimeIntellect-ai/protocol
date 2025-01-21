use crate::api::server::AppState;
use actix_web::{
    web::{self, post, Data},
    HttpResponse, Scope,
};
use redis::Commands;
use serde::{Deserialize, Serialize};
use serde_json::json;

// It would actually be smart to share these interfaces between miner and orchestrator
#[derive(Debug, Serialize, Deserialize)]
struct HeartbeatRequest {
    address: String,
}

async fn heartbeat(
    heartbeat: web::Json<HeartbeatRequest>,
    app_state: Data<AppState>,
) -> HttpResponse {
    println!("Heartbeat incoming for address: {}", heartbeat.address);
    let mut con = app_state.store.client.get_connection().unwrap();
    let key = format!("orchestrator:node:{}:heartbeat", heartbeat.address);
    let _: () = con
        .set_options(
            &key,
            "1",
            redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(60)),
        )
        .unwrap();
    HttpResponse::Ok().json(json!({"success": true}))
}

pub fn heartbeat_routes() -> Scope {
    web::scope("/heartbeat").route("", post().to(heartbeat))
}

// Tests for the heartbeat route
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api::server::AppState, store::redis::RedisStore};
    use actix_web::http::StatusCode;
    use actix_web::test;
    use actix_web::App;
    use std::sync::Arc;
    use web::Data;

    async fn create_test_app_state() -> Data<AppState> {
        // Create a test Redis store - you might want to use a mock or test instance
        let store = RedisStore::new_test();
        let mut con = store
            .client
            .get_connection()
            .expect("Should connect to test Redis instance");

        redis::cmd("PING")
            .query::<String>(&mut con)
            .expect("Redis should be responsive");

        Data::new(AppState {
            store: Arc::new(store),
        })
    }

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
        let mut con = app_state.store.client.get_connection().unwrap();
        let key = "orchestrator:node:0x0000000000000000000000000000000000000000:heartbeat";
        let value: Option<String> = con.get(key).unwrap();
        println!("Value: {:?}", value);
        assert_eq!(value, Some("1".to_string()));
    }
}
