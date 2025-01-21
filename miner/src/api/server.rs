use crate::api::routes::invite::invite_routes;
use crate::api::routes::task::task_routes;
use crate::operations::heartbeat::service::HeartbeatService;
use actix_web::{middleware, web::Data, App, HttpServer};
use shared::security::auth_signature_middleware::ValidateSignature;
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::contracts::structs::compute_pool::PoolInfo;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
#[derive(Clone)]
pub struct AppState {
    pub contracts: Arc<Contracts>,
    pub node_wallet: Arc<Wallet>,
    pub provider_wallet: Arc<Wallet>,
    pub heartbeat_service: Arc<HeartbeatService>,
}

pub async fn start_server(
    host: &str,
    port: u16,
    contracts: Arc<Contracts>,
    node_wallet: Arc<Wallet>,
    provider_wallet: Arc<Wallet>,
    heartbeat_service: Arc<HeartbeatService>,
    pool_info: Arc<PoolInfo>,
) -> std::io::Result<()> {
    println!("Starting server at http://{}:{}", host, port);

    let allowed_addresses = vec![pool_info.creator, pool_info.compute_manager_key];

    let app_state = Data::new(AppState {
        contracts,
        node_wallet,
        provider_wallet,
        heartbeat_service,
    });

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .wrap(ValidateSignature::new(allowed_addresses.clone()))
            .service(task_routes())
            .service(invite_routes())
    })
    .bind((host, port))?
    .run()
    .await
}
