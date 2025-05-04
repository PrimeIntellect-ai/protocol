use crate::api::routes::nodes::nodes_routes;
use crate::api::routes::storage::storage_routes;
use crate::api::routes::task::tasks_routes;
use crate::api::routes::{heartbeat::heartbeat_routes, metrics::metrics_routes};
use crate::models::node::NodeStatus;
use crate::store::core::{RedisStore, StoreContext};
use crate::utils::loop_heartbeats::LoopHeartbeats;
use crate::ServerMode;
use actix_web::middleware::{Compress, NormalizePath, TrailingSlash};
use actix_web::{middleware, web::Data, App, HttpServer};
use actix_web::{web, HttpResponse};
use anyhow::Error;
use log::info;
use serde_json::json;
use shared::security::api_key_middleware::ApiKeyMiddleware;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::Wallet;
use std::sync::Arc;

pub struct AppState {
    pub store_context: Arc<StoreContext>,
    pub wallet: Arc<Wallet>,
    pub s3_credentials: Option<String>,
    pub bucket_name: Option<String>,
    pub heartbeats: Arc<LoopHeartbeats>,
    pub redis_store: Arc<RedisStore>,
    pub hourly_upload_limit: i64,
    pub contracts: Option<Arc<Contracts>>,
    pub pool_id: u32,
}

#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    host: &str,
    port: u16,
    store_context: Arc<StoreContext>,
    wallet: Arc<Wallet>,
    admin_api_key: String,
    s3_credentials: Option<String>,
    bucket_name: Option<String>,
    heartbeats: Arc<LoopHeartbeats>,
    redis_store: Arc<RedisStore>,
    hourly_upload_limit: i64,
    contracts: Option<Arc<Contracts>>,
    pool_id: u32,
    server_mode: ServerMode,
) -> Result<(), Error> {
    info!("Starting server at http://{}:{}", host, port);
    let app_state = Data::new(AppState {
        store_context,
        wallet,
        s3_credentials,
        bucket_name,
        heartbeats,
        redis_store,
        hourly_upload_limit,
        contracts,
        pool_id,
    });
    let node_store = app_state.store_context.node_store.clone();
    let node_store_clone = node_store.clone();
    let validator_state = Arc::new(ValidatorState::new(vec![]).with_validator(move |address| {
        if let Some(node) = node_store_clone.get_node(address) {
            node.status != NodeStatus::Ejected
        } else {
            false
        }
    }));

    let api_key_middleware = Arc::new(ApiKeyMiddleware::new(admin_api_key));

    HttpServer::new(move || {
        let mut app = App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .wrap(Compress::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .app_data(web::PayloadConfig::default().limit(2_097_152))
            .service(web::resource("/health").route(web::get().to(
                |data: web::Data<AppState>| async move {
                    let health_status = data.heartbeats.health_status();
                    if health_status.healthy {
                        HttpResponse::Ok().json(health_status)
                    } else {
                        HttpResponse::InternalServerError().json(health_status)
                    }
                },
            )));

        if !matches!(server_mode, ServerMode::ProcessorOnly) {
            app = app
                .service(heartbeat_routes().wrap(ValidateSignature::new(validator_state.clone())))
                .service(storage_routes().wrap(ValidateSignature::new(validator_state.clone())))
                .service(nodes_routes().wrap(api_key_middleware.clone()))
                .service(tasks_routes().wrap(api_key_middleware.clone()))
                .service(metrics_routes().wrap(api_key_middleware.clone()))
                .default_service(web::route().to(|| async {
                    HttpResponse::NotFound().json(json!({
                        "success": false,
                        "error": "Resource not found"
                    }))
                }));
        }

        app
    })
    .bind((host, port))?
    .run()
    .await?;
    Ok(())
}
