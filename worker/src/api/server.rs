use crate::api::routes::challenge::challenge_routes;
use crate::api::routes::invite::invite_routes;
use crate::api::routes::task::task_routes;
use crate::docker::DockerService;
use crate::operations::heartbeat::service::HeartbeatService;
use actix_web::{middleware, web::Data, App, HttpServer};
use log::error;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
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
) -> std::io::Result<()> {
    let app_state = Data::new(AppState {
        contracts: contracts.clone(),
        node_wallet,
        provider_wallet,
        heartbeat_service,
        docker_service,
    });

    let validators = match contracts.prime_network.get_validator_role().await {
        Ok(validators) => validators,
        Err(e) => {
            error!("Failed to get validator role: {}", e);
            std::process::exit(1);
        }
    };

    let mut allowed_addresses = vec![pool_info.creator, pool_info.compute_manager_key];
    allowed_addresses.extend(validators);
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
