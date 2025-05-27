pub mod store;
pub mod validators;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use alloy::primitives::utils::Unit;
use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use clap::Parser;
use log::{debug, LevelFilter};
use log::{error, info};
use serde_json::json;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::security::request_signer::sign_request;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use store::redis::RedisStore;
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::sync::CancellationToken;
use url::Url;
use validators::hardware::HardwareValidator;
use validators::synthetic_data::SyntheticDataValidator;

// Track the last time the validation loop ran
static LAST_VALIDATION_TIMESTAMP: AtomicI64 = AtomicI64::new(0);
// Maximum allowed time between validation loops (2 minutes)
const MAX_VALIDATION_INTERVAL_SECS: i64 = 120;
// Track the last loop duration in milliseconds
static LAST_LOOP_DURATION_MS: AtomicI64 = AtomicI64::new(0);
async fn health_check() -> impl Responder {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let last_validation = LAST_VALIDATION_TIMESTAMP.load(Ordering::Relaxed);
    let last_duration_ms = LAST_LOOP_DURATION_MS.load(Ordering::Relaxed);

    if last_validation == 0 {
        // Validation hasn't run yet, but we're still starting up
        return HttpResponse::Ok().json(json!({
            "status": "starting",
            "message": "Validation loop hasn't started yet"
        }));
    }

    let elapsed = now - last_validation;

    if elapsed > MAX_VALIDATION_INTERVAL_SECS {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "message": format!("Validation loop hasn't run in {} seconds (max allowed: {})", elapsed, MAX_VALIDATION_INTERVAL_SECS),
            "last_loop_duration_ms": last_duration_ms
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "ok",
        "last_validation_seconds_ago": elapsed,
        "last_loop_duration_ms": last_duration_ms
    }))
}

#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    validator_key: String,

    /// Discovery url
    #[arg(long, default_value = "http://localhost:8089")]
    discovery_url: String,

    /// Ability to disable hardware validation
    #[arg(long, default_value = "false")]
    disable_hardware_validation: bool,

    /// Optional: Pool Id for work validation
    /// If not provided, the validator will not validate work
    #[arg(long, default_value = None)]
    pool_id: Option<String>,

    /// Optional: Toploc configurations as JSON array string
    /// Example: [{"server_url": "http://example.com", "auth_token": "token123"}]
    #[arg(long, default_value = None)]
    toploc_configs: Option<String>,

    /// Optional: Toploc Grace Interval in seconds between work validation requests
    #[arg(long, default_value = "15")]
    toploc_grace_interval: u64,

    /// Optional: interval in minutes of max age of work on chain
    #[arg(long, default_value = "15")]
    toploc_work_validation_interval: u64,

    /// Optional: interval in minutes of max age of work on chain
    #[arg(long, default_value = "120")]
    toploc_work_validation_unknown_status_expiry_seconds: u64,

    /// Optional: Validator penalty in whole tokens
    /// Note: This value will be multiplied by 10^18 (1 token = 10^18 wei)
    #[arg(long, default_value = "200")]
    validator_penalty: u64,

    /// Temporary: S3 bucket name
    #[arg(long, default_value = None)]
    bucket_name: Option<String>,

    /// Log level
    #[arg(short = 'l', long, default_value = "info")]
    log_level: String,

    /// Redis URL
    #[arg(long, default_value = "redis://localhost:6380")]
    redis_url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let log_level = match args.log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info,
    };
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp(None)
        .init();

    let cancellation_token = CancellationToken::new();
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let mut sigquit = signal(SignalKind::quit())?;
    let signal_token = cancellation_token.clone();
    let cancellation_token_clone = cancellation_token.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = sigterm.recv() => {
                log::info!("Received termination signal");
            }
            _ = sigint.recv() => {
                log::info!("Received interrupt signal");
            }
            _ = sighup.recv() => {
                log::info!("Received hangup signal");
            }
            _ = sigquit.recv() => {
                log::info!("Received quit signal");
            }
        }
        signal_token.cancel();
    });

    let private_key_validator = args.validator_key;
    let rpc_url: Url = args.rpc_url.parse().unwrap();
    let discovery_url = args.discovery_url;

    let redis_store = RedisStore::new(&args.redis_url);

    let validator_wallet = Wallet::new(&private_key_validator, rpc_url).unwrap_or_else(|err| {
        error!("Error creating wallet: {:?}", err);
        std::process::exit(1);
    });

    tokio::spawn(async {
        if let Err(e) = HttpServer::new(|| App::new().route("/health", web::get().to(health_check)))
            .bind("0.0.0.0:9879")
            .expect("Failed to bind health check server")
            .run()
            .await
        {
            error!("Actix server error: {:?}", e);
        }
    });

    let mut contract_builder = ContractBuilder::new(&validator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .with_domain_registry()
        .with_stake_manager();

    let contracts = contract_builder.build_partial().unwrap();

    if let Some(pool_id) = args.pool_id.clone() {
        let pool = match contracts
            .compute_pool
            .get_pool_info(U256::from_str(&pool_id).unwrap())
            .await
        {
            Ok(pool_info) => pool_info,
            Err(e) => {
                error!("Failed to get pool info: {:?}", e);
                std::process::exit(1);
            }
        };
        let domain_id: u32 = pool.domain_id.try_into().unwrap();
        let domain = contracts
            .domain_registry
            .as_ref()
            .unwrap()
            .get_domain(domain_id)
            .await
            .unwrap();
        contract_builder =
            contract_builder.with_synthetic_data_validator(Some(domain.validation_logic));
    }

    let contracts = Arc::new(contract_builder.build().unwrap());

    let hardware_validator = HardwareValidator::new(&validator_wallet, contracts.clone());

    let synthetic_validator = if let Some(pool_id) = args.pool_id.clone() {
        let penalty = U256::from(args.validator_penalty) * Unit::ETHER.wei();
        match contracts.synthetic_data_validator.clone() {
            Some(validator) => {
                info!(
                    "Synthetic validator has penalty: {} ({})",
                    penalty, args.validator_penalty
                );

                if args.toploc_configs.is_none() {
                    error!("Toploc configs are required for synthetic validator");
                    std::process::exit(1);
                }
                let s3_credentials = std::env::var("S3_CREDENTIALS").ok();

                let toploc_configs = match args.toploc_configs.as_ref() {
                    Some(configs) => configs,
                    None => {
                        error!("Toploc configs are required but not provided");
                        std::process::exit(1);
                    }
                };

                let configs = match serde_json::from_str(toploc_configs) {
                    Ok(configs) => configs,
                    Err(e) => {
                        error!("Failed to parse toploc configs: {}", e);
                        std::process::exit(1);
                    }
                };

                Some(SyntheticDataValidator::new(
                    pool_id,
                    validator,
                    contracts.prime_network.clone(),
                    configs,
                    penalty,
                    s3_credentials,
                    args.bucket_name,
                    redis_store,
                    cancellation_token,
                    args.toploc_work_validation_interval,
                    args.toploc_work_validation_unknown_status_expiry_seconds,
                    args.toploc_grace_interval,
                ))
            }
            None => {
                error!("Synthetic data validator not found");
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    loop {
        if cancellation_token_clone.is_cancelled() {
            log::info!("Validation loop is stopping due to cancellation signal");
            break;
        }

        // Start timing the loop
        let loop_start = Instant::now();

        // Update the last validation timestamp
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        LAST_VALIDATION_TIMESTAMP.store(now, Ordering::Relaxed);

        if let Some(validator) = synthetic_validator.clone() {
            if let Err(e) = validator.validate_work().await {
                error!("Failed to validate work: {}", e);
            }
        }

        if !args.disable_hardware_validation {
            async fn _generate_signature(wallet: &Wallet, message: &str) -> Result<String> {
                let signature = sign_request(message, wallet, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(signature)
            }

            let nodes = match async {
                let discovery_route = "/api/validator";
                let address = validator_wallet
                    .wallet
                    .default_signer()
                    .address()
                    .to_string();

                let signature = _generate_signature(&validator_wallet, discovery_route)
                    .await
                    .context("Failed to generate signature")?;

                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    "x-address",
                    reqwest::header::HeaderValue::from_str(&address)
                        .context("Failed to create address header")?,
                );
                headers.insert(
                    "x-signature",
                    reqwest::header::HeaderValue::from_str(&signature)
                        .context("Failed to create signature header")?,
                );

                debug!("Fetching nodes from: {}{}", discovery_url, discovery_route);
                let response = reqwest::Client::new()
                    .get(format!("{}{}", discovery_url, discovery_route))
                    .headers(headers)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await
                    .context("Failed to fetch nodes")?;

                let response_text = response
                    .text()
                    .await
                    .context("Failed to get response text")?;

                let parsed_response: ApiResponse<Vec<DiscoveryNode>> =
                    serde_json::from_str(&response_text).context("Failed to parse response")?;

                if !parsed_response.success {
                    error!("Failed to fetch nodes: {:?}", parsed_response);
                    return Ok::<Vec<DiscoveryNode>, anyhow::Error>(vec![]);
                }

                Ok(parsed_response.data)
            }
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    error!("Error in node fetching loop: {:#}", e);
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    continue;
                }
            };

            // Ensure nodes have enough stake
            let mut nodes_with_enough_stake = Vec::new();
            let stake_manager = match contracts.stake_manager.as_ref() {
                Some(manager) => manager,
                None => {
                    error!("Stake manager contract not initialized");
                    continue;
                }
            };

            let mut provider_stake_cache: std::collections::HashMap<String, (U256, U256)> =
                std::collections::HashMap::new();
            for node in nodes {
                let provider_address_str = &node.node.provider_address;
                let provider_address = match Address::from_str(provider_address_str) {
                    Ok(address) => address,
                    Err(e) => {
                        error!(
                            "Failed to parse provider address {}: {}",
                            provider_address_str, e
                        );
                        continue;
                    }
                };

                let (stake, required_stake) =
                    if let Some(&cached_info) = provider_stake_cache.get(provider_address_str) {
                        cached_info
                    } else {
                        let stake = stake_manager
                            .get_stake(provider_address)
                            .await
                            .unwrap_or_default();
                        let total_compute = contracts
                            .compute_registry
                            .get_provider_total_compute(provider_address)
                            .await
                            .unwrap_or_default();
                        let required_stake = stake_manager
                            .calculate_stake(U256::from(0), total_compute)
                            .await
                            .unwrap_or_default();

                        provider_stake_cache
                            .insert(provider_address_str.clone(), (stake, required_stake));
                        (stake, required_stake)
                    };

                if stake >= required_stake {
                    nodes_with_enough_stake.push(node);
                } else {
                    info!(
                        "Node {} has insufficient stake: {} (required: {})",
                        node.node.id,
                        stake / Unit::ETHER.wei(),
                        required_stake / Unit::ETHER.wei()
                    );
                }
            }

            if let Err(e) = hardware_validator
                .validate_nodes(nodes_with_enough_stake)
                .await
            {
                error!("Error validating nodes: {:#}", e);
            }
        }

        // Calculate and store loop duration
        let loop_duration = loop_start.elapsed();
        let loop_duration_ms = loop_duration.as_millis() as i64;
        LAST_LOOP_DURATION_MS.store(loop_duration_ms, Ordering::Relaxed);

        info!("Validation loop completed in {}ms", loop_duration_ms);
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use actix_web::{test, App};
    use actix_web::{
        web::{self, post},
        HttpResponse, Scope,
    };
    use shared::models::challenge::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64};

    pub async fn handle_challenge(challenge: web::Json<ChallengeRequest>) -> HttpResponse {
        let result = calc_matrix(&challenge);
        HttpResponse::Ok().json(result)
    }

    pub fn challenge_routes() -> Scope {
        web::scope("/challenge")
            .route("", post().to(handle_challenge))
            .route("/", post().to(handle_challenge))
    }

    #[actix_web::test]
    async fn test_challenge_route() {
        let app = test::init_service(App::new().service(challenge_routes())).await;

        let vec_a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let vec_b = [9.0, 8.0, 7.0, 6.0, 5.0, 4.0, 3.0, 2.0, 1.0];

        // convert vectors to FixedF64
        let data_a: Vec<FixedF64> = vec_a.iter().map(|x| FixedF64(*x)).collect();
        let data_b: Vec<FixedF64> = vec_b.iter().map(|x| FixedF64(*x)).collect();

        let challenge_request = ChallengeRequest {
            rows_a: 3,
            cols_a: 3,
            data_a,
            rows_b: 3,
            cols_b: 3,
            data_b,
            timestamp: None,
        };

        let req = test::TestRequest::post()
            .uri("/challenge")
            .set_json(&challenge_request)
            .to_request();

        let resp: ChallengeResponse = test::call_and_read_body_json(&app, req).await;
        let expected_response = calc_matrix(&challenge_request);

        assert_eq!(resp.result, expected_response.result);
    }
}
