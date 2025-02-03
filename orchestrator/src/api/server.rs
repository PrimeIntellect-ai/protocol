use crate::api::routes::nodes::nodes_routes;
use crate::api::routes::task::tasks_routes;
use crate::api::routes::{heartbeat::heartbeat_routes, metrics::metrics_routes};
use crate::store::core::StoreContext;
use actix_web::middleware::{Compress, NormalizePath, TrailingSlash};
use actix_web::{middleware, web::Data, App, HttpServer};
use actix_web::{web, HttpResponse};
use anyhow::Error;
use log::info;
use serde_json::json;
use shared::security::api_key_middleware::ApiKeyMiddleware;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use std::sync::Arc;

pub struct AppState {
    pub store_context: Arc<StoreContext>,
}

pub async fn start_server(
    host: &str,
    port: u16,
    store_context: Arc<StoreContext>,
    admin_api_key: String,
) -> Result<(), Error> {
    info!("Starting server at http://{}:{}", host, port);
    let app_state = Data::new(AppState { store_context });
    let node_store = app_state.store_context.node_store.clone();
    let node_store_clone = node_store.clone();
    let validator_state = Arc::new(
        ValidatorState::new(vec![])
            .with_validator(move |address| node_store_clone.get_node(address).is_some()),
    );

    let api_key_middleware = Arc::new(ApiKeyMiddleware::new(admin_api_key));

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .wrap(Compress::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .app_data(web::PayloadConfig::default().limit(2_097_152))
            .service(heartbeat_routes().wrap(ValidateSignature::new(validator_state.clone())))
            .service(nodes_routes().wrap(api_key_middleware.clone()))
            .service(tasks_routes().wrap(api_key_middleware.clone()))
            .service(metrics_routes().wrap(api_key_middleware.clone()))
            .default_service(web::route().to(|| async {
                HttpResponse::NotFound().json(json!({
                    "success": false,
                    "error": "Resource not found"
                }))
            }))
    })
    .bind((host, port))?
    .run()
    .await?;
    Ok(())
}
