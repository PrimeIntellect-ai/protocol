mod api;
mod discovery;
mod node;
mod store;
mod types;
use crate::api::server::start_server;
use crate::discovery::monitor::DiscoveryMonitor;
use crate::node::invite::NodeInviter;
use crate::node::status_update::NodeStatusUpdater;
use crate::store::redis::RedisStore;
use alloy::primitives::U256;
use anyhow::Result;
use clap::Parser;
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
    let args = Args::parse();
    let compute_pool_id = args.compute_pool_id;
    let domain_id = args.domain_id;
    let coordinator_key = args.coordinator_key;
    let rpc_url: Url = args.rpc_url.parse().unwrap();

    let mut tasks: JoinSet<Result<()>> = JoinSet::new();

    let coordinator_wallet = Arc::new(Wallet::new(&coordinator_key, rpc_url).unwrap_or_else(
        |err| {
            eprintln!("Error creating wallet: {:?}", err);
            std::process::exit(1);
        },
    ));

    let contracts = ContractBuilder::new(&coordinator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()?;

    // TODO: validate that pool exists and we are owner
    // TODO: Move to utils - only here for debug
    let tx = contracts
        .compute_pool
        .start_compute_pool(U256::from(compute_pool_id))
        .await;
    println!("Start pool Tx: {:?}", tx);

    let store = Arc::new(RedisStore::new(&args.redis_store_url));
    let wallet_clone = coordinator_wallet.clone();
    let discovery_store_clone = store.clone();
    let inviter_store_clone = store.clone();
    let status_update_store_clone = store.clone();

    tasks.spawn(async move {
        let monitor = DiscoveryMonitor::new(
            discovery_store_clone,
            wallet_clone.as_ref(),
            compute_pool_id,
            args.discovery_refresh_interval,
            args.discovery_url,
        );
        monitor.run().await
    });

    let host = args.host.clone();
    let port = args.port;

    tasks.spawn(async move {
        let inviter = NodeInviter::new(
            inviter_store_clone,
            coordinator_wallet.as_ref(),
            compute_pool_id,
            domain_id,
            &args.host,
            &args.port,
        );
        inviter.run().await
    });

    tasks.spawn(async move {
        let status_updater = NodeStatusUpdater::new(status_update_store_clone, 15, None);
        status_updater.run().await
    });

    tokio::select! {
        res = start_server(&host, port, store.clone()) => {
            if let Err(e) = res {
                eprintln!("Server error: {}", e);
            }
        }
        Some(res) = tasks.join_next() => {
            if let Err(e) = res? {
                eprintln!("Task error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            println!("Shutdown signal received");
        }
    }
    tasks.shutdown().await;
    Ok(())
}
