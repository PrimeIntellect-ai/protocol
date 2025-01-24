mod api;
mod discovery;
mod models;
mod node;
mod store;
use crate::api::server::start_server;
use crate::discovery::monitor::DiscoveryMonitor;
use crate::node::invite::NodeInviter;
use crate::node::status_update::NodeStatusUpdater;
use crate::store::core::RedisStore;
use crate::store::core::StoreContext;
use anyhow::Result;
use clap::Parser;
use log::error;
use log::LevelFilter;
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

    /// External ip
    #[arg(short = 'e', long, default_value = "0.0.0.0")]
    host: String,

    /// Port
    #[arg(short = 'p', long, default_value = "8090")]
    port: u16,

    /// Discovery refresh interval
    #[arg(short = 'i', long, default_value = "10")]
    discovery_refresh_interval: u64,

    /// Redis store url
    #[arg(short = 's', long, default_value = "redis://localhost:6379")]
    redis_store_url: String,

    /// Discovery url
    #[arg(long, default_value = "http://localhost:8089")]
    discovery_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(None)
        .init();
    let args = Args::parse();
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

    let discovery_store_context = store_context.clone();
    tasks.spawn(async move {
        let monitor = DiscoveryMonitor::new(
            wallet_clone.as_ref(),
            compute_pool_id,
            args.discovery_refresh_interval,
            args.discovery_url,
            discovery_store_context.clone(),
        );
        monitor.run().await
    });

    let host = args.host.clone();
    let port = args.port;

    let inviter_store_context = store_context.clone();
    tasks.spawn(async move {
        let inviter = NodeInviter::new(
            coordinator_wallet.as_ref(),
            compute_pool_id,
            domain_id,
            &args.host,
            &args.port,
            inviter_store_context.clone(),
        );
        inviter.run().await
    });

    let status_update_store_context = store_context.clone();
    tasks.spawn(async move {
        let status_updater = NodeStatusUpdater::new(status_update_store_context.clone(), 15, None);
        status_updater.run().await
    });

    let server_store_context = store_context.clone();
    tokio::select! {
        res = start_server(&host, port, server_store_context.clone()) => {
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
