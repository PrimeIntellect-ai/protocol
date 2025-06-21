mod api;
mod chainsync;
mod location_enrichment;
mod location_service;
mod store;
use crate::api::server::start_server;
use crate::chainsync::ChainSync;
use crate::location_enrichment::LocationEnrichmentService;
use crate::location_service::LocationService;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use alloy::providers::RootProvider;
use anyhow::Result;
use clap::Parser;
use log::LevelFilter;
use log::{error, info};
use shared::web3::contracts::core::builder::ContractBuilder;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ServiceMode {
    Api,
    Processor,
    Full,
}

impl std::str::FromStr for ServiceMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "api" => Ok(ServiceMode::Api),
            "processor" => Ok(ServiceMode::Processor),
            "full" => Ok(ServiceMode::Full),
            _ => Err(format!(
                "Invalid mode: {}. Use 'api', 'processor', or 'full'",
                s
            )),
        }
    }
}

#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,

    /// Platform API key
    #[arg(short = 'p', long, default_value = "prime")]
    platform_api_key: String,

    /// Redis URL
    #[arg(long, default_value = "redis://localhost:6380")]
    redis_url: String,

    /// Port
    #[arg(short = 'P', long, default_value = "8089")]
    port: u16,

    /// Maximum number of nodes allowed per IP address (active state)
    #[arg(long, default_value = "1")]
    max_nodes_per_ip: u32,

    /// Service mode: api, processor, or full
    #[arg(short = 'm', long, default_value = "full")]
    mode: ServiceMode,

    /// Location service URL (e.g., https://ipapi.co). If not provided, location services are disabled.
    #[arg(long)]
    location_service_url: Option<String>,

    /// Location service API key
    #[arg(long)]
    location_service_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(None)
        .init();

    let args = Args::parse();

    let redis_store = Arc::new(RedisStore::new(&args.redis_url));
    let node_store = Arc::new(NodeStore::new(redis_store.as_ref().clone()));
    let endpoint = match args.rpc_url.parse() {
        Ok(url) => url,
        Err(_) => {
            return Err(anyhow::anyhow!("invalid RPC URL: {}", args.rpc_url));
        }
    };

    let provider = RootProvider::new_http(endpoint);
    let contracts = ContractBuilder::new(provider)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .with_stake_manager()
        .build()
        .unwrap();

    let cancellation_token = CancellationToken::new();
    let last_chain_sync = Arc::new(Mutex::new(None::<std::time::SystemTime>));

    info!("Starting discovery service in {:?} mode", args.mode);

    match args.mode {
        ServiceMode::Processor | ServiceMode::Full => {
            let chain_sync = ChainSync::new(
                node_store.clone(),
                cancellation_token.clone(),
                Duration::from_secs(10),
                contracts.clone(),
                last_chain_sync.clone(),
            );
            chain_sync.run().await?;

            // Start location enrichment service if enabled
            if let Some(location_url) = args.location_service_url.clone() {
                let location_service = Arc::new(LocationService::new(
                    Some(location_url),
                    args.location_service_api_key.clone(),
                ));
                let location_enrichment = LocationEnrichmentService::new(
                    node_store.clone(),
                    location_service,
                    &args.redis_url,
                )?;

                info!("Starting location enrichment service");
                tokio::spawn(async move {
                    if let Err(e) = location_enrichment.run(10).await {
                        // Run every 60 seconds
                        error!("Location enrichment service failed: {}", e);
                    }
                });
            }

            if let Err(err) = start_server(
                "0.0.0.0",
                args.port,
                node_store,
                redis_store,
                contracts,
                args.platform_api_key,
                last_chain_sync,
                args.max_nodes_per_ip,
                true,
            )
            .await
            {
                error!("❌ Failed to start server: {}", err);
            }

            tokio::signal::ctrl_c().await?;
            cancellation_token.cancel();
        }
        ServiceMode::Api => {
            if let Err(err) = start_server(
                "0.0.0.0",
                args.port,
                node_store,
                redis_store,
                contracts,
                args.platform_api_key,
                last_chain_sync,
                args.max_nodes_per_ip,
                false,
            )
            .await
            {
                error!("❌ Failed to start server: {}", err);
            }
        }
    }

    Ok(())
}
