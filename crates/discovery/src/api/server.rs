use crate::api::routes::get_nodes::{get_node_by_subkey, get_nodes, get_nodes_for_pool};
use crate::api::routes::node::node_routes;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use actix_web::middleware::{Compress, NormalizePath, TrailingSlash};
use actix_web::HttpResponse;
use actix_web::{
    middleware,
    web::Data,
    web::{self, get},
    App, HttpServer,
};
use alloy::providers::RootProvider;
use log::{error, info, warn};
use serde_json::json;
use shared::security::api_key_middleware::ApiKeyMiddleware;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use shared::web3::contracts::core::builder::Contracts;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub node_store: Arc<NodeStore>,
    pub contracts: Option<Contracts<RootProvider>>,
    pub last_chain_sync: Arc<Mutex<Option<SystemTime>>>,
    pub max_nodes_per_ip: u32,
    pub chain_sync_enabled: bool,
}

async fn health_check(app_state: web::Data<AppState>) -> HttpResponse {
    if app_state.chain_sync_enabled {
        let sync_status = {
            let last_sync_guard = app_state.last_chain_sync.lock().await;
            match *last_sync_guard {
                Some(last_sync) => {
                    if let Ok(elapsed) = last_sync.elapsed() {
                        if elapsed > Duration::from_secs(60) {
                            warn!(
                                "Health check: Chain sync is delayed. Last sync was {} seconds ago",
                                elapsed.as_secs()
                            );
                            Some(elapsed)
                        } else {
                            None
                        }
                    } else {
                        warn!("Health check: Unable to determine elapsed time since last sync");
                        Some(Duration::from_secs(u64::MAX))
                    }
                }
                None => {
                    warn!("Health check: Chain sync has not occurred yet");
                    Some(Duration::from_secs(u64::MAX))
                }
            }
        };

        if let Some(elapsed) = sync_status {
            return HttpResponse::ServiceUnavailable().json(json!({
                "status": "error",
                "service": "discovery",
                "message": format!("Chain sync is delayed. Last sync was {} seconds ago", elapsed.as_secs())
            }));
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "ok",
        "service": "discovery",
        "chain_sync_enabled": app_state.chain_sync_enabled
    }))
}

#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    host: &str,
    port: u16,
    node_store: Arc<NodeStore>,
    redis_store: Arc<RedisStore>,
    contracts: Contracts<RootProvider>,
    platform_api_key: String,
    last_chain_sync: Arc<Mutex<Option<SystemTime>>>,
    max_nodes_per_ip: u32,
    chain_sync_enabled: bool,
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
        last_chain_sync,
        max_nodes_per_ip,
        chain_sync_enabled,
    };

    let validator_validator = Arc::new(
        ValidatorState::new(validators)
            .with_redis(redis_store.client.clone())
            .await
            .map_err(|e| {
                std::io::Error::other(format!("Failed to initialize Redis connection pool: {}", e))
            })?,
    );
    let validate_signatures = Arc::new(
        ValidatorState::new(vec![])
            .with_redis(redis_store.client.clone())
            .await
            .map_err(|e| {
                std::io::Error::other(format!("Failed to initialize Redis connection pool: {}", e))
            })?
            .with_validator(move |_| true),
    );
    let api_key_middleware = Arc::new(ApiKeyMiddleware::new(platform_api_key));

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .wrap(Compress::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .app_data(Data::new(app_state.clone()))
            .app_data(web::PayloadConfig::default().limit(2_097_152))
            .route("/health", web::get().to(health_check))
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
