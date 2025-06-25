pub mod metrics;
pub mod p2p;
pub mod store;
pub mod validators;
use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use alloy::primitives::utils::Unit;
use alloy::primitives::{Address, U256};
use anyhow::{Context, Result};
use clap::Parser;
use log::{debug, LevelFilter};
use log::{error, info};
use metrics::MetricsContext;
use p2p::P2PClient;
use serde_json::json;
use shared::models::api::ApiResponse;
use shared::models::node::DiscoveryNode;
use shared::security::api_key_middleware::ApiKeyMiddleware;
use shared::security::request_signer::sign_request_with_nonce;
use shared::utils::google_cloud::GcsStorageProvider;
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

use crate::validators::synthetic_data::types::InvalidationType;
// Track the last time the validation loop ran
static LAST_VALIDATION_TIMESTAMP: AtomicI64 = AtomicI64::new(0);
// Maximum allowed time between validation loops (2 minutes)
const MAX_VALIDATION_INTERVAL_SECS: i64 = 120;
// Track the last loop duration in milliseconds
static LAST_LOOP_DURATION_MS: AtomicI64 = AtomicI64::new(0);

async fn get_rejections(
    req: HttpRequest,
    validator: web::Data<Option<SyntheticDataValidator<shared::web3::wallet::WalletProvider>>>,
) -> impl Responder {
    match validator.as_ref() {
        Some(validator) => {
            // Parse query parameters
            let query = req.query_string();
            let limit = parse_limit_param(query).unwrap_or(100); // Default limit of 100

            let result = if limit > 0 && limit < 1000 {
                // Use the optimized recent rejections method for reasonable limits
                validator.get_recent_rejections(limit as isize).await
            } else {
                // Fallback to all rejections (but warn about potential performance impact)
                if limit >= 1000 {
                    info!("Large limit requested ({limit}), this may impact performance");
                }
                validator.get_all_rejections().await
            };

            match result {
                Ok(rejections) => HttpResponse::Ok().json(ApiResponse {
                    success: true,
                    data: rejections,
                }),
                Err(e) => {
                    error!("Failed to get rejections: {e}");
                    HttpResponse::InternalServerError().json(ApiResponse {
                        success: false,
                        data: format!("Failed to get rejections: {e}"),
                    })
                }
            }
        }
        None => HttpResponse::ServiceUnavailable().json(ApiResponse {
            success: false,
            data: "Synthetic data validator not available",
        }),
    }
}

fn parse_limit_param(query: &str) -> Option<u32> {
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            if key == "limit" {
                return value.parse::<u32>().ok();
            }
        }
    }
    None
}

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

    /// Discovery URLs (comma-separated)
    #[arg(long, default_value = "http://localhost:8089", value_delimiter = ',')]
    discovery_urls: Vec<String>,

    /// Ability to disable hardware validation
    #[arg(long, default_value = "false")]
    disable_hardware_validation: bool,

    /// Optional: Pool Id for work validation
    /// If not provided, the validator will not validate work
    #[arg(long, default_value = None)]
    pool_id: Option<String>,

    /// Optional: Toploc Grace Interval in seconds between work validation requests
    #[arg(long, default_value = "15")]
    toploc_grace_interval: u64,

    /// Optional: interval in minutes of max age of work on chain
    #[arg(long, default_value = "15")]
    toploc_work_validation_interval: u64,

    /// Optional: interval in minutes of max age of work on chain
    #[arg(long, default_value = "120")]
    toploc_work_validation_unknown_status_expiry_seconds: u64,

    /// Disable toploc ejection
    /// If true, the validator will not invalidate work on toploc
    #[arg(long, default_value = "false")]
    disable_toploc_invalidation: bool,

    /// Optional: batch trigger size
    #[arg(long, default_value = "10")]
    batch_trigger_size: usize,

    /// Grouping
    #[arg(long, default_value = "false")]
    use_grouping: bool,

    /// Grace period in minutes for incomplete groups to recover (0 = disabled)
    #[arg(long, default_value = "0")]
    incomplete_group_grace_period_minutes: u64,

    /// Optional: toploc invalidation type
    #[arg(long, default_value = "hard")]
    toploc_invalidation_type: InvalidationType,

    /// Optional: work unit invalidation type
    #[arg(long, default_value = "hard")]
    work_unit_invalidation_type: InvalidationType,

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
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let log_level = match args.log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => anyhow::bail!("invalid log level: {}", args.log_level),
    };
    env_logger::Builder::new()
        .filter_level(log_level)
        .filter_module("iroh", log::LevelFilter::Warn)
        .filter_module("iroh_net", log::LevelFilter::Warn)
        .filter_module("iroh_quinn", log::LevelFilter::Warn)
        .filter_module("iroh_base", log::LevelFilter::Warn)
        .filter_module("tracing::span", log::LevelFilter::Warn)
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
    let discovery_urls = args.discovery_urls;

    let redis_store = RedisStore::new(&args.redis_url);

    let validator_wallet = Wallet::new(&private_key_validator, rpc_url).unwrap_or_else(|err| {
        error!("Error creating wallet: {err:?}");
        std::process::exit(1);
    });

    let mut contract_builder = ContractBuilder::new(validator_wallet.provider())
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .with_domain_registry()
        .with_stake_manager();

    let contracts = contract_builder.build_partial().unwrap();

    let metrics_ctx =
        MetricsContext::new(validator_wallet.address().to_string(), args.pool_id.clone());

    // Initialize P2P client if enabled
    let p2p_client = {
        match P2PClient::new(validator_wallet.clone()).await {
            Ok(client) => {
                info!("P2P client initialized for testing");
                Some(client)
            }
            Err(e) => {
                error!("Failed to initialize P2P client: {e}");
                None
            }
        }
    };

    if let Some(pool_id) = args.pool_id.clone() {
        let pool = match contracts
            .compute_pool
            .get_pool_info(U256::from_str(&pool_id).unwrap())
            .await
        {
            Ok(pool_info) => pool_info,
            Err(e) => {
                error!("Failed to get pool info: {e:?}");
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

    let contracts = contract_builder.build().unwrap();

    let hardware_validator =
        HardwareValidator::new(&validator_wallet, contracts.clone(), p2p_client.as_ref());

    let synthetic_validator = if let Some(pool_id) = args.pool_id.clone() {
        let penalty = U256::from(args.validator_penalty) * Unit::ETHER.wei();
        match contracts.synthetic_data_validator.clone() {
            Some(validator) => {
                info!(
                    "Synthetic validator has penalty: {} ({})",
                    penalty, args.validator_penalty
                );

                let toploc_configs = match std::env::var("TOPLOC_CONFIGS") {
                    Ok(configs) => configs,
                    Err(_) => {
                        error!("Toploc configs are required but not provided in environment");
                        std::process::exit(1);
                    }
                };
                info!("Toploc configs: {toploc_configs}");

                let configs = match serde_json::from_str(&toploc_configs) {
                    Ok(configs) => configs,
                    Err(e) => {
                        error!("Failed to parse toploc configs: {e}");
                        std::process::exit(1);
                    }
                };

                let s3_credentials = std::env::var("S3_CREDENTIALS").ok();
                let gcs_storage =
                    GcsStorageProvider::new(&args.bucket_name.unwrap(), &s3_credentials.unwrap())
                        .await
                        .unwrap();
                let storage_provider = Arc::new(gcs_storage);

                Some(SyntheticDataValidator::new(
                    pool_id,
                    validator,
                    contracts.prime_network.clone(),
                    configs,
                    penalty,
                    storage_provider,
                    redis_store,
                    cancellation_token,
                    args.toploc_work_validation_interval,
                    args.toploc_work_validation_unknown_status_expiry_seconds,
                    args.toploc_grace_interval,
                    args.batch_trigger_size,
                    args.use_grouping,
                    args.disable_toploc_invalidation,
                    args.incomplete_group_grace_period_minutes,
                    args.toploc_invalidation_type,
                    args.work_unit_invalidation_type,
                    Some(metrics_ctx.clone()),
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

    // Start HTTP server with access to the validator
    let validator_for_server = synthetic_validator.clone();
    tokio::spawn(async move {
        let key = std::env::var("VALIDATOR_API_KEY").unwrap_or_default();
        let api_key_middleware = Arc::new(ApiKeyMiddleware::new(key));

        if let Err(e) = HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(validator_for_server.clone()))
                .route("/health", web::get().to(health_check))
                .route(
                    "/rejections",
                    web::get()
                        .to(get_rejections)
                        .wrap(api_key_middleware.clone()),
                )
                .route(
                    "/metrics",
                    web::get().to(|| async {
                        match metrics::export_metrics() {
                            Ok(metrics) => {
                                HttpResponse::Ok().content_type("text/plain").body(metrics)
                            }
                            Err(e) => {
                                error!("Error exporting metrics: {e:?}");
                                HttpResponse::InternalServerError().finish()
                            }
                        }
                    }),
                )
        })
        .bind("0.0.0.0:9879")
        .expect("Failed to bind health check server")
        .run()
        .await
        {
            error!("Actix server error: {e:?}");
        }
    });

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
                error!("Failed to validate work: {e}");
            }
        }

        if !args.disable_hardware_validation {
            async fn _fetch_nodes_from_discovery_url(
                discovery_url: &str,
                validator_wallet: &Wallet,
            ) -> Result<Vec<DiscoveryNode>> {
                let address = validator_wallet
                    .wallet
                    .default_signer()
                    .address()
                    .to_string();

                let discovery_route = "/api/validator";
                let signature = sign_request_with_nonce(discovery_route, validator_wallet, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    "x-address",
                    reqwest::header::HeaderValue::from_str(&address)
                        .context("Failed to create address header")?,
                );
                headers.insert(
                    "x-signature",
                    reqwest::header::HeaderValue::from_str(&signature.signature)
                        .context("Failed to create signature header")?,
                );

                debug!("Fetching nodes from: {discovery_url}{discovery_route}");
                let response = reqwest::Client::new()
                    .get(format!("{discovery_url}{discovery_route}"))
                    .query(&[("nonce", signature.nonce)])
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
                    error!("Failed to fetch nodes from {discovery_url}: {parsed_response:?}");
                    return Ok(vec![]);
                }

                Ok(parsed_response.data)
            }

            let nodes = match async {
                let mut all_nodes = Vec::new();
                let mut any_success = false;

                for discovery_url in &discovery_urls {
                    match _fetch_nodes_from_discovery_url(discovery_url, &validator_wallet).await {
                        Ok(nodes) => {
                            debug!(
                                "Successfully fetched {} nodes from {}",
                                nodes.len(),
                                discovery_url
                            );
                            all_nodes.extend(nodes);
                            any_success = true;
                        }
                        Err(e) => {
                            error!("Failed to fetch nodes from {discovery_url}: {e:#}");
                        }
                    }
                }

                if !any_success {
                    error!("Failed to fetch nodes from all discovery services");
                    return Ok::<Vec<DiscoveryNode>, anyhow::Error>(vec![]);
                }

                // Remove duplicates based on node ID
                let mut unique_nodes = Vec::new();
                let mut seen_ids = std::collections::HashSet::new();
                for node in all_nodes {
                    if seen_ids.insert(node.node.id.clone()) {
                        unique_nodes.push(node);
                    }
                }

                debug!(
                    "Total unique nodes after deduplication: {}",
                    unique_nodes.len()
                );
                Ok(unique_nodes)
            }
            .await
            {
                Ok(n) => n,
                Err(e) => {
                    error!("Error in node fetching loop: {e:#}");
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
                        error!("Failed to parse provider address {provider_address_str}: {e}");
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
                error!("Error validating nodes: {e:#}");
            }
        }

        // Calculate and store loop duration
        let loop_duration = loop_start.elapsed();
        let loop_duration_ms = loop_duration.as_millis() as i64;
        LAST_LOOP_DURATION_MS.store(loop_duration_ms, Ordering::Relaxed);

        metrics_ctx.record_validation_loop_duration(loop_duration.as_secs_f64());
        info!("Validation loop completed in {loop_duration_ms}ms");
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
