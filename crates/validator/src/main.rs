use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use alloy::primitives::utils::Unit;
use alloy::primitives::U256;
use clap::Parser;
use log::LevelFilter;
use log::{error, info};
use serde_json::json;
use shared::models::api::ApiResponse;
use shared::security::api_key_middleware::ApiKeyMiddleware;
use shared::utils::google_cloud::GcsStorageProvider;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::signal::unix::{signal, SignalKind};
use tokio_util::sync::CancellationToken;
use url::Url;

use validator::{
    export_metrics, HardwareValidator, InvalidationType, MetricsContext, P2PService, RedisStore,
    SyntheticDataValidator, Validator,
};

#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    validator_key: String,

    // /// Discovery URLs (comma-separated)
    // #[arg(long, default_value = "http://localhost:8089", value_delimiter = ',')]
    // discovery_urls: Vec<String>,
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

    /// Libp2p port
    #[arg(long, default_value = "4003")]
    libp2p_port: u16,

    /// Comma-separated list of libp2p bootnode multiaddresses
    /// Example: `/ip4/104.131.131.82/tcp/4001/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ,/ip4/104.131.131.82/udp/4001/quic-v1/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ`
    #[arg(long, default_value = "")]
    bootnodes: String,
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

    let private_key_validator = args.validator_key;
    let rpc_url: Url = args.rpc_url.parse().unwrap();

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

    let keypair = p2p::Keypair::generate_ed25519();
    let bootnodes: Vec<p2p::Multiaddr> = args
        .bootnodes
        .split(',')
        .filter_map(|addr| match addr.to_string().try_into() {
            Ok(multiaddr) => Some(multiaddr),
            Err(e) => {
                error!("Invalid bootnode address '{addr}': {e}");
                None
            }
        })
        .collect();
    if bootnodes.is_empty() {
        error!("No valid bootnodes provided. Please provide at least one valid bootnode address.");
        std::process::exit(1);
    }

    let (p2p_service, hardware_challenge_tx, kademlia_action_tx) = {
        match P2PService::new(
            keypair,
            args.libp2p_port,
            bootnodes,
            cancellation_token.clone(),
            validator_wallet.clone(),
        ) {
            Ok(res) => {
                info!("p2p service initialized successfully");
                res
            }
            Err(e) => {
                error!("failed to initialize p2p service: {e}");
                std::process::exit(1);
            }
        }
    };

    tokio::task::spawn(p2p_service.run());

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

    let hardware_validator = HardwareValidator::new(contracts.clone(), hardware_challenge_tx);

    let synthetic_validator = if let Some(pool_id) = args.pool_id.clone() {
        let penalty = U256::from(args.validator_penalty) * Unit::ETHER.wei();
        match contracts.synthetic_data_validator.clone() {
            Some(validator) => {
                info!(
                    "Synthetic validator has penalty: {} ({})",
                    penalty, args.validator_penalty
                );

                let Ok(toploc_configs) = std::env::var("TOPLOC_CONFIGS") else {
                    error!("Toploc configs are required but not provided in environment");
                    std::process::exit(1);
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

                match (args.bucket_name.as_ref(), s3_credentials) {
                    (Some(bucket_name), Some(s3_credentials))
                        if !bucket_name.is_empty() && !s3_credentials.is_empty() =>
                    {
                        let gcs_storage = GcsStorageProvider::new(bucket_name, &s3_credentials)
                            .await
                            .unwrap_or_else(|_| panic!("Failed to create GCS storage provider"));
                        let storage_provider = Arc::new(gcs_storage);

                        Some(SyntheticDataValidator::new(
                            pool_id,
                            validator,
                            contracts.prime_network.clone(),
                            configs,
                            penalty,
                            storage_provider,
                            redis_store,
                            cancellation_token.clone(),
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
                    _ => {
                        info!("Bucket name or S3 credentials not provided, skipping synthetic data validation");
                        None
                    }
                }
            }
            None => {
                error!("Synthetic data validator not found");
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    let (validator, validator_health) = match Validator::new(
        cancellation_token.clone(),
        validator_wallet.provider(),
        contracts,
        hardware_validator,
        synthetic_validator.clone(),
        kademlia_action_tx,
        args.disable_hardware_validation,
        metrics_ctx,
    ) {
        Ok(v) => v,
        Err(e) => {
            error!("Failed to create validator: {e}");
            std::process::exit(1);
        }
    };

    // Start HTTP server with access to the validator
    tokio::spawn(async move {
        let key = std::env::var("VALIDATOR_API_KEY").unwrap_or_default();
        let api_key_middleware = Arc::new(ApiKeyMiddleware::new(key));

        if let Err(e) = HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new((
                    synthetic_validator.clone(),
                    validator_health.clone(),
                )))
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
                        match export_metrics() {
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

    tokio::task::spawn(validator.run());

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let mut sigquit = signal(SignalKind::quit())?;

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
    cancellation_token.cancel();

    // TODO: handle spawn handles here

    Ok(())
}

async fn health_check(
    _: HttpRequest,
    state: web::Data<
        Option<(
            SyntheticDataValidator<shared::web3::wallet::WalletProvider>,
            Arc<tokio::sync::Mutex<validator::ValidatorHealth>>,
        )>,
    >,
) -> impl Responder {
    // Maximum allowed time between validation loops (2 minutes)
    const MAX_VALIDATION_INTERVAL_SECS: u64 = 120;

    let Some(state) = state.get_ref() else {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "message": "Validator not initialized"
        }));
    };

    let validator_health = state.1.lock().await;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if validator_health.last_validation_timestamp() == 0 {
        // Validation hasn't run yet, but we're still starting up
        return HttpResponse::Ok().json(json!({
            "status": "starting",
            "message": "Validation loop hasn't started yet"
        }));
    }

    let elapsed = now - validator_health.last_validation_timestamp();

    if elapsed > MAX_VALIDATION_INTERVAL_SECS {
        return HttpResponse::ServiceUnavailable().json(json!({
        "status": "error",
        "message": format!("Validation loop hasn't run in {} seconds (max allowed: {})", elapsed, MAX_VALIDATION_INTERVAL_SECS),
        "last_loop_duration_ms": validator_health.last_loop_duration_ms(),
    }));
    }

    HttpResponse::Ok().json(json!({
        "status": "ok",
        "last_validation_seconds_ago": elapsed,
        "last_loop_duration_ms": validator_health.last_loop_duration_ms(),
    }))
}

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

#[cfg(test)]
mod tests {
    use actix_web::{test, App};
    use actix_web::{
        web::{self, post},
        HttpResponse, Scope,
    };
    use p2p::{calc_matrix, ChallengeRequest, ChallengeResponse, FixedF64};

    async fn handle_challenge(challenge: web::Json<ChallengeRequest>) -> HttpResponse {
        let result = calc_matrix(&challenge);
        HttpResponse::Ok().json(result)
    }

    fn challenge_routes() -> Scope {
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
