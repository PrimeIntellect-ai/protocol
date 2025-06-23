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
use crate::utils::loop_heartbeats::{HealthStatus, LoopHeartbeats};
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
use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_swagger_ui::SwaggerUi;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "ApiKeyAuth",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("Authorization"))),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Orchestrator API",
        description = "Prime Intellect Orchestrator API",
        version = "1.0.0"
    ),
    paths(
        crate::api::routes::task::get_all_tasks,
        crate::api::routes::task::create_task,
        crate::api::routes::task::delete_task,
        crate::api::routes::task::delete_all_tasks,
        crate::api::routes::nodes::get_nodes,
        crate::api::routes::nodes::restart_node_task,
        crate::api::routes::nodes::get_node_logs,
        crate::api::routes::nodes::get_node_metrics,
        crate::api::routes::nodes::ban_node,
        crate::api::routes::metrics::get_metrics,
        crate::api::routes::metrics::get_all_metrics,
        crate::api::routes::metrics::get_prometheus_metrics,
        crate::api::routes::metrics::create_metric,
        crate::api::routes::metrics::delete_metric,
        crate::api::routes::groups::get_groups,
        crate::api::routes::groups::get_configurations,
        crate::api::routes::groups::delete_group,
        crate::api::routes::groups::get_group_logs,
        crate::api::routes::groups::force_regroup,
    ),
    security(
        ("ApiKeyAuth" = [])
    ),
    modifiers(&SecurityAddon),
    components(
        schemas(
            shared::models::api::ApiResponse<serde_json::Value>,
            shared::models::task::Task,
            shared::models::task::TaskRequest,
            shared::models::node::Node,
            crate::models::node::NodeStatus,
            crate::models::node::OrchestratorNode,
            shared::models::metric::MetricEntry,
            shared::models::metric::MetricKey,
        )
    ),
    tags(
        (name = "tasks", description = "Task management endpoints"),
        (name = "nodes", description = "Node management endpoints"),
        (name = "metrics", description = "Metrics collection endpoints"),
        (name = "groups", description = "Node groups management endpoints"),
    )
)]
struct ApiDoc;

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Service is healthy", body = HealthStatus),
        (status = 500, description = "Service is unhealthy", body = HealthStatus)
    ),
    tag = "health"
)]
async fn health_check(data: web::Data<AppState>) -> HttpResponse {
    let health_status = data
        .heartbeats
        .health_status(data.node_groups_plugin.is_some());
    if health_status.healthy {
        HttpResponse::Ok().json(health_status)
    } else {
        HttpResponse::InternalServerError().json(health_status)
    }
}

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
    info!("Starting server at http://{host}:{port}");
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
    let validator_state = Arc::new(
        ValidatorState::new(vec![])
            .with_redis(app_state.redis_store.client.clone())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to initialize Redis connection pool: {}", e))?
            .with_async_validator(move |address| {
                let address = *address;
                let node_store = node_store_clone.clone();
                Box::pin(async move {
                    match node_store.get_node(&address).await {
                        Ok(Some(node)) => node.status != NodeStatus::Ejected,
                        _ => false,
                    }
                })
            }),
    );

    let api_key_middleware = Arc::new(ApiKeyMiddleware::new(admin_api_key));
    HttpServer::new(move || {
        let mut app = App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .wrap(Compress::default())
            .wrap(NormalizePath::new(TrailingSlash::Trim))
            .app_data(web::PayloadConfig::default().limit(2_097_152))
            .service(web::resource("/health").route(web::get().to(health_check)))
            .service(
                web::resource("/api-docs/openapi.json")
                    .route(web::get().to(|| async { HttpResponse::Ok().json(ApiDoc::openapi()) })),
            )
            .service(metrics_routes().wrap(api_key_middleware.clone()));

        if !matches!(server_mode, ServerMode::ProcessorOnly) {
            app = app
                .service(heartbeat_routes().wrap(ValidateSignature::new(validator_state.clone())))
                .service(storage_routes().wrap(ValidateSignature::new(validator_state.clone())))
                .service(nodes_routes().wrap(api_key_middleware.clone()))
                .service(tasks_routes().wrap(api_key_middleware.clone()))
                .service(groups_routes().wrap(api_key_middleware.clone()))
                .service(
                    SwaggerUi::new("/swagger-ui/{_:.*}")
                        .url("/api-docs/openapi.json", ApiDoc::openapi()),
                )
                .service(web::redirect("/docs", "/swagger-ui/index.html"))
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
