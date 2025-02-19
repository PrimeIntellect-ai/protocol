use crate::api::routes::challenge::challenge_routes;
use crate::api::routes::invite::invite_routes;
use crate::api::routes::task::task_routes;
use crate::docker::DockerService;
use crate::operations::heartbeat::service::HeartbeatService;
use actix_web::{middleware, web::Data, App, HttpServer};
use alloy::primitives::Address;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::contracts::structs::compute_pool::PoolInfo;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub contracts: Arc<Contracts>,
    pub node_wallet: Arc<Wallet>,
    pub provider_wallet: Arc<Wallet>,
    pub heartbeat_service: Arc<HeartbeatService>,
    pub docker_service: Arc<DockerService>,
}

#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    host: &str,
    port: u16,
    contracts: Arc<Contracts>,
    node_wallet: Arc<Wallet>,
    provider_wallet: Arc<Wallet>,
    heartbeat_service: Arc<HeartbeatService>,
    docker_service: Arc<DockerService>,
    pool_info: Arc<PoolInfo>,
    validator_address: String,
) -> std::io::Result<()> {
    println!("Starting server at http://{}:{}", host, port);

    let app_state = Data::new(AppState {
        contracts,
        node_wallet,
        provider_wallet,
        heartbeat_service,
        docker_service,
    });

    let validator = Address::from_str(validator_address.as_str()).unwrap();

    let allowed_addresses = vec![pool_info.creator, pool_info.compute_manager_key, validator];
    let validator_state = Arc::new(ValidatorState::new(allowed_addresses));

    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(middleware::Logger::default())
            .wrap(ValidateSignature::new(validator_state.clone()))
            .service(invite_routes())
            .service(task_routes())
            .service(challenge_routes())
    })
    .bind((host, port))?
    .run()
    .await
}
