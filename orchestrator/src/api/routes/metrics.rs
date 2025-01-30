use crate::api::server::AppState;
use actix_web::{
    web::{self, get, Data},
    HttpResponse, Scope,
};
use serde_json::json;

async fn get_nodes(app_state: Data<AppState>) -> HttpResponse {
    let nodes = app_state.store_context.node_store.get_nodes();
    HttpResponse::Ok().json(json!({"success": true}))
}

pub fn nodes_routes() -> Scope {
    web::scope("/").route("", get().to(get_nodes))
}
