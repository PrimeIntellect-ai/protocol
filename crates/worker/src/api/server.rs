use crate::api::routes::challenge::challenge_routes;
use crate::api::routes::invite::invite_routes;
use crate::api::routes::task::task_routes;
use crate::docker::DockerService;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::state::system_state::SystemState;
use actix_web::{middleware, web, web::Data, App, HttpResponse, HttpServer};
use log::error;
use serde_json::json;
use shared::security::auth_signature_middleware::{ValidateSignature, ValidatorState};
use shared::web3::contracts::core::builder::Contracts;
use shared::web3::contracts::structs::compute_pool::PoolInfo;
use shared::web3::wallet::{Wallet, WalletProvider};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub contracts: Contracts<WalletProvider>,
    pub node_wallet: Wallet,
    pub provider_wallet: Wallet,
    pub heartbeat_service: Arc<HeartbeatService>,
    pub docker_service: Arc<DockerService>,
    pub system_state: Arc<SystemState>,
}

#[allow(clippy::too_many_arguments)]
pub async fn start_server(
    host: &str,
    port: u16,
    contracts: Contracts<WalletProvider>,
    node_wallet: Wallet,
    provider_wallet: Wallet,
    heartbeat_service: Arc<HeartbeatService>,
    docker_service: Arc<DockerService>,
    pool_info: Arc<PoolInfo>,
    system_state: Arc<SystemState>,
) -> std::io::Result<()> {
    let app_state = Data::new(AppState {
        contracts: contracts.clone(),
        node_wallet,
        provider_wallet,
        heartbeat_service,
        docker_service,
        system_state,
    });

    let validators = match contracts.prime_network.get_validator_role().await {
        Ok(validators) => validators,
        Err(e) => {
            error!("Failed to get validator role: {}", e);
            std::process::exit(1);
        }
    };

    if validators.is_empty() {
        error!("❌ No validator roles found on contracts - cannot start worker without validators");
        error!("This means the smart contract has no registered validators, which is required for signature validation");
        error!("Please ensure validators are properly registered on the PrimeNetwork contract before starting the worker");
        std::process::exit(1);
    }

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
