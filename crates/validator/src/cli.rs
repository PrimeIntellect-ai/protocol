use std::str::FromStr;
use std::sync::Arc;

use actix_web::{web, App, HttpResponse, HttpServer};
use alloy::primitives::{utils::Unit, U256};
use clap::Parser;
use log::{error, info};
use shared::{
    security::api_key_middleware::ApiKeyMiddleware,
    utils::google_cloud::GcsStorageProvider,
    web3::{contracts::core::builder::ContractBuilder, wallet::Wallet},
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    export_metrics,
    handler::{get_rejections, health_check, State},
    HardwareValidator, InvalidationType, MetricsContext, P2PService, RedisStore,
    SyntheticDataValidator, Validator,
};

#[derive(Parser)]
pub struct Cli {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    pub rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    pub validator_key: String,

    /// Ability to disable hardware validation
    #[arg(long, default_value = "false")]
    pub disable_hardware_validation: bool,

    /// Optional: Pool Id for work validation
    /// If not provided, the validator will not validate work
    #[arg(long, default_value = None)]
    pub pool_id: Option<String>,

    /// Optional: Toploc Grace Interval in seconds between work validation requests
    #[arg(long, default_value = "15")]
    pub toploc_grace_interval: u64,

    /// Optional: interval in minutes of max age of work on chain
    #[arg(long, default_value = "15")]
    pub toploc_work_validation_interval: u64,

    /// Optional: interval in minutes of max age of work on chain
    #[arg(long, default_value = "120")]
    pub toploc_work_validation_unknown_status_expiry_seconds: u64,

    /// Disable toploc ejection
    /// If true, the validator will not invalidate work on toploc
    #[arg(long, default_value = "false")]
    pub disable_toploc_invalidation: bool,

    /// Optional: batch trigger size
    #[arg(long, default_value = "10")]
    pub batch_trigger_size: usize,

    /// Grouping
    #[arg(long, default_value = "false")]
    pub use_grouping: bool,

    /// Grace period in minutes for incomplete groups to recover (0 = disabled)
    #[arg(long, default_value = "0")]
    pub incomplete_group_grace_period_minutes: u64,

    /// Optional: toploc invalidation type
    #[arg(long, default_value = "hard")]
    pub toploc_invalidation_type: InvalidationType,

    /// Optional: work unit invalidation type
    #[arg(long, default_value = "hard")]
    pub work_unit_invalidation_type: InvalidationType,

    /// Optional: Validator penalty in whole tokens
    /// Note: This value will be multiplied by 10^18 (1 token = 10^18 wei)
    #[arg(long, default_value = "200")]
    pub validator_penalty: u64,

    /// Temporary: S3 bucket name
    #[arg(long, default_value = None)]
    pub bucket_name: Option<String>,

    /// Log level
    #[arg(short = 'l', long, default_value = "info")]
    pub log_level: String,

    /// Redis URL
    #[arg(long, default_value = "redis://localhost:6380")]
    pub redis_url: String,

    /// Libp2p port
    #[arg(long, default_value = "4003")]
    pub libp2p_port: u16,

    /// Comma-separated list of libp2p bootnode multiaddresses
    /// Example: `/ip4/104.131.131.82/tcp/4001/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ,/ip4/104.131.131.82/udp/4001/quic-v1/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ`
    #[arg(long, default_value = "")]
    pub bootnodes: String,
}

impl Cli {
    pub async fn run(self, cancellation_token: CancellationToken) -> anyhow::Result<()> {
        let private_key_validator = self.validator_key;
        let rpc_url: Url = self.rpc_url.parse().unwrap();

        let redis_store = RedisStore::new(&self.redis_url);

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
            MetricsContext::new(validator_wallet.address().to_string(), self.pool_id.clone());

        let keypair = p2p::Keypair::generate_ed25519();
        let bootnodes: Vec<p2p::Multiaddr> = self
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
            error!(
                "No valid bootnodes provided. Please provide at least one valid bootnode address."
            );
            std::process::exit(1);
        }

        let (p2p_service, hardware_challenge_tx, kademlia_action_tx) = {
            match P2PService::new(
                keypair,
                self.libp2p_port,
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

        if let Some(pool_id) = self.pool_id.clone() {
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

        let synthetic_validator = if let Some(pool_id) = self.pool_id.clone() {
            let penalty = U256::from(self.validator_penalty) * Unit::ETHER.wei();
            match contracts.synthetic_data_validator.clone() {
                Some(validator) => {
                    info!(
                        "Synthetic validator has penalty: {} ({})",
                        penalty, self.validator_penalty
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

                    match (self.bucket_name.as_ref(), s3_credentials) {
                        (Some(bucket_name), Some(s3_credentials))
                            if !bucket_name.is_empty() && !s3_credentials.is_empty() =>
                        {
                            let gcs_storage = GcsStorageProvider::new(bucket_name, &s3_credentials)
                                .await
                                .unwrap_or_else(|_| {
                                    panic!("Failed to create GCS storage provider")
                                });
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
                                self.toploc_work_validation_interval,
                                self.toploc_work_validation_unknown_status_expiry_seconds,
                                self.toploc_grace_interval,
                                self.batch_trigger_size,
                                self.use_grouping,
                                self.disable_toploc_invalidation,
                                self.incomplete_group_grace_period_minutes,
                                self.toploc_invalidation_type,
                                self.work_unit_invalidation_type,
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
            self.disable_hardware_validation,
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
                    .app_data(web::Data::new(State {
                        synthetic_validator: synthetic_validator.clone(),
                        validator_health: validator_health.clone(),
                    }))
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
        Ok(())
    }
}
