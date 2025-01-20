use actix_web::{
    web::{self, post},
    HttpResponse, Scope,
};
use serde_json::json;

pub async fn heartbeat() -> HttpResponse {
    println!("Heartbeat");
    HttpResponse::Ok().json(json!({"success": true}))
}

pub fn heartbeat_routes() -> Scope {
    web::scope("/heartbeat").route("", post().to(heartbeat))
}
