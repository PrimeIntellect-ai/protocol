use crate::api::routes::groups::groups_routes;
use crate::api::routes::nodes::nodes_routes;
use crate::api::routes::storage::storage_routes;
use crate::api::routes::task::tasks_routes;
use crate::api::routes::{heartbeat::heartbeat_routes, metrics::metrics_routes};
use crate::metrics::MetricsContext;
use crate::models::node::NodeStatus;
use crate::p2p::client::P2PClient;
use crate::plugins::node_groups::NodeGroupsPlugin;
use crate::scheduler::Scheduler;
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
use shared::utils::StorageProvider;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::wallet::WalletProvider;
use std::sync::Arc;

pub struct AppState {
    pub store_context: Arc<StoreContext>,
    pub storage_provider: Arc<dyn StorageProvider>,
    pub heartbeats: Arc<LoopHeartbeats>,
    pub redis_store: Arc<RedisStore>,
    pub hourly_upload_limit: i64,
    pub contracts: Option<Contracts<WalletProvider>>,
    pub pool_id: u32,
    pub scheduler: Scheduler,
    pub node_groups_plugin: Option<Arc<NodeGroupsPlugin>>,
    pub metrics: Arc<MetricsContext>,
    pub p2p_client: Arc<P2PClient>,
}

#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    host: &str,
    port: u16,
    store_context: Arc<StoreContext>,
    admin_api_key: String,
    storage_provider: Arc<dyn StorageProvider>,
    heartbeats: Arc<LoopHeartbeats>,
    redis_store: Arc<RedisStore>,
    hourly_upload_limit: i64,
    contracts: Option<Contracts<WalletProvider>>,
    pool_id: u32,
    server_mode: ServerMode,
    scheduler: Scheduler,
    node_groups_plugin: Option<Arc<NodeGroupsPlugin>>,
    metrics: Arc<MetricsContext>,
    p2p_client: Arc<P2PClient>,
) -> Result<(), Error> {
    info!("Starting server at http://{}:{}", host, port);
    let app_state = Data::new(AppState {
        store_context,
        storage_provider,
        heartbeats,
        redis_store,
        hourly_upload_limit,
        contracts,
        pool_id,
        scheduler,
        node_groups_plugin,
        metrics,
        p2p_client,
    });
    let node_store = app_state.store_context.node_store.clone();
    let node_store_clone = node_store.clone();
    let validator_state = Arc::new(ValidatorState::new(vec![]).with_async_validator(
        move |address| {
            let address = *address;
            let node_store = node_store_clone.clone();
            Box::pin(async move {
                match node_store.get_node(&address).await {
                    Ok(Some(node)) => node.status != NodeStatus::Ejected,
                    _ => false,
                }
            })
        },
    ));

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
                    let health_status = data
                        .heartbeats
                        .health_status(data.node_groups_plugin.is_some());
                    if health_status.healthy {
                        HttpResponse::Ok().json(health_status)
                    } else {
                        HttpResponse::InternalServerError().json(health_status)
                    }
                },
            )))
            .service(metrics_routes().wrap(api_key_middleware.clone()));

        if !matches!(server_mode, ServerMode::ProcessorOnly) {
            app = app
                .service(heartbeat_routes().wrap(ValidateSignature::new(validator_state.clone())))
                .service(storage_routes().wrap(ValidateSignature::new(validator_state.clone())))
                .service(nodes_routes().wrap(api_key_middleware.clone()))
                .service(tasks_routes().wrap(api_key_middleware.clone()))
                .service(groups_routes().wrap(api_key_middleware.clone()))
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
