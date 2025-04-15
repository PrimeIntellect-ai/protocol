mod api;
mod discovery;
mod models;
mod node;
mod store;
mod utils;
use crate::api::server::start_server;
use crate::discovery::monitor::DiscoveryMonitor;
use crate::node::invite::NodeInviter;
use crate::node::status_update::NodeStatusUpdater;
use crate::store::core::RedisStore;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use anyhow::Result;
use clap::Parser;
use log::debug;
use log::error;
use log::LevelFilter;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::task::JoinSet;
use url::Url;
#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    coordinator_key: String,

    /// Compute pool id
    #[arg(long, default_value = "0")]
    compute_pool_id: u32,

    /// Domain id
    #[arg(short = 'd', long, default_value = "0")]
    domain_id: u32,

    /// External ip - advertised to workers
    #[arg(short = 'e', long)]
    host: Option<String>,

    /// Port
    #[arg(short = 'p', long, default_value = "8090")]
    port: u16,

    /// External url - advertised to workers
    #[arg(short = 'u', long)]
    url: Option<String>,

    /// Discovery refresh interval
    #[arg(short = 'i', long, default_value = "10")]
    discovery_refresh_interval: u64,

    /// Redis store url
    #[arg(short = 's', long, default_value = "redis://localhost:6380")]
    redis_store_url: String,

    /// Discovery url
    #[arg(long, default_value = "http://localhost:8089")]
    discovery_url: String,

    /// Admin api key
    #[arg(short = 'a', long, default_value = "admin")]
    admin_api_key: String,

    /// Disable instance ejection from chain
    #[arg(long)]
    disable_ejection: bool,

    /// S3 credentials
    #[arg(long)]
    s3_credentials: Option<String>,

    /// Hourly s3 upload limit
    #[arg(long, default_value = "2")]
    hourly_s3_upload_limit: i64,

    /// S3 bucket name
    #[arg(long)]
    bucket_name: Option<String>,

    /// Log level
    #[arg(short = 'l', long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let log_level = match args.log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info, // Default to Info if the level is unrecognized
    };
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp(None)
        .init();
    debug!("Log level: {}", log_level);

    let heartbeats = Arc::new(LoopHeartbeats::new());

    let compute_pool_id = args.compute_pool_id;
    let domain_id = args.domain_id;
    let coordinator_key = args.coordinator_key;
    let rpc_url: Url = args.rpc_url.parse().unwrap();

    let mut tasks: JoinSet<Result<()>> = JoinSet::new();

    let coordinator_wallet = Arc::new(Wallet::new(&coordinator_key, rpc_url).unwrap_or_else(
        |err| {
            error!("Error creating wallet: {:?}", err);
            std::process::exit(1);
        },
    ));

    let store = Arc::new(RedisStore::new(&args.redis_store_url));
    let store_context = Arc::new(StoreContext::new(store.clone()));
    let wallet_clone = coordinator_wallet.clone();
    let server_wallet = coordinator_wallet.clone();

    let contracts = Arc::new(
        ContractBuilder::new(&coordinator_wallet.clone())
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .build()
            .unwrap(),
    );

    let discovery_store_context = store_context.clone();
    let discovery_heartbeats = heartbeats.clone();
    tasks.spawn(async move {
        let monitor = DiscoveryMonitor::new(
            wallet_clone.as_ref(),
            compute_pool_id,
            args.discovery_refresh_interval,
            args.discovery_url,
            discovery_store_context.clone(),
            discovery_heartbeats.clone(),
        );
        monitor.run().await
    });

    let port = args.port;

    let inviter_store_context = store_context.clone();
    let inviter_heartbeats = heartbeats.clone();
    tasks.spawn(async move {
        let inviter = NodeInviter::new(
            coordinator_wallet.as_ref(),
            compute_pool_id,
            domain_id,
            args.host.as_deref(),
            Some(&args.port),
            args.url.as_deref(),
            inviter_store_context.clone(),
            inviter_heartbeats.clone(),
        );
        inviter.run().await
    });

    // The node status updater is responsible for checking the heartbeats
    // and calculating the status of the node.
    // It also ejects nodes when they are dead.
    let status_update_store_context = store_context.clone();
    let status_update_heartbeats = heartbeats.clone();
    tasks.spawn(async move {
        let status_updater = NodeStatusUpdater::new(
            status_update_store_context.clone(),
            15,
            None,
            contracts.clone(),
            compute_pool_id,
            args.disable_ejection,
            status_update_heartbeats.clone(),
        );
        status_updater.run().await
    });

    let server_store_context = store_context.clone();
    tokio::select! {
        res = start_server("0.0.0.0", port, server_store_context.clone(), server_wallet, args.admin_api_key, args.s3_credentials, args.bucket_name, heartbeats.clone(), store.clone(), args.hourly_s3_upload_limit) => {
            if let Err(e) = res {
                error!("Server error: {}", e);
            }
        }
        Some(res) = tasks.join_next() => {
            if let Err(e) = res? {
                error!("Task error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            error!("Shutdown signal received");
        }
    }
    tasks.shutdown().await;
    Ok(())
}
