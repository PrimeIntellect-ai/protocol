use crate::api::routes::get_nodes::{get_node_by_subkey, get_nodes, get_nodes_for_pool};
use crate::api::routes::node::node_routes;
use crate::store::node_store::NodeStore;
use actix_web::middleware::{Compress, NormalizePath, TrailingSlash};
use actix_web::HttpResponse;
use actix_web::{
    middleware,
    web::Data,
    web::{self, get},
    App, HttpServer,
};
use log::{error, info};
use serde_json::json;
use shared::security::api_key_middleware::ApiKeyMiddleware;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use shared::web3::contracts::core::builder::Contracts;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub node_store: Arc<NodeStore>,
    pub contracts: Option<Arc<Contracts>>,
}

pub async fn start_server(
    host: &str,
    port: u16,
    node_store: Arc<NodeStore>,
    contracts: Arc<Contracts>,
    platform_api_key: String,
) -> std::io::Result<()> {
    info!("Starting server at http://{}:{}", host, port);

    let validators = match contracts.prime_network.get_validator_role().await {
        Ok(validators) => validators,
        Err(e) => {
            error!("❌ Failed to get validator role: {}", e);
            std::process::exit(1);
        }
    };

    let app_state = AppState {
        node_store,
        contracts: Some(contracts),
    };

    // it seems we have a validator for the validator
    let validator_validator = Arc::new(ValidatorState::new(validators));

    // All nodes can register as long as they have a valid signature
    let validate_signatures = Arc::new(ValidatorState::new(vec![]).with_validator(move |_| true));
    let api_key_middleware = Arc::new(ApiKeyMiddleware::new(platform_api_key));

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .wrap(Compress::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .app_data(Data::new(app_state.clone()))
            .app_data(web::PayloadConfig::default().limit(2_097_152))
            .service(
                web::scope("/api/platform")
                    .wrap(api_key_middleware.clone())
                    .route("", get().to(get_nodes)),
            )
            .service(
                web::scope("/api/nodes/{node_id}")
                    .wrap(api_key_middleware.clone())
                    .route("", get().to(get_node_by_subkey)),
            )
            .service(
                web::scope("/api/validator")
                    .wrap(ValidateSignature::new(validator_validator.clone()))
                    .route("", web::get().to(get_nodes)),
            )
            .service(
                web::scope("/api/pool/{pool_id}")
                    .wrap(ValidateSignature::new(validate_signatures.clone()))
                    .route("", get().to(get_nodes_for_pool)),
            )
            .service(node_routes().wrap(ValidateSignature::new(validate_signatures.clone())))
            .default_service(web::route().to(|| async {
                HttpResponse::NotFound().json(json!({
                    "success": false,
                    "error": "Resource not found"
                }))
            }))
    })
    .bind((host, port))?
    .run()
    .await
}
