mod api;
mod chainsync;
mod store;
use crate::api::server::start_server;
use crate::chainsync::ChainSync;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use anyhow::Result;
use clap::Parser;
use log::error;
use log::LevelFilter;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
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

    /// Only one node per IP address
    #[arg(long)]
    only_one_node_per_ip: Option<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .format_timestamp(None)
        .init();

    let args = Args::parse();

    let redis_store = RedisStore::new(&args.redis_url);
    let node_store = Arc::new(NodeStore::new(redis_store));
    // TODO: Find a way to read from chain without address - hardcoded key here
    let wallet = Arc::new(
        Wallet::new(
            "0x4d97cf024fd8b486e5c8079cc478cbe24f01f99f75a0a28ae45088658f2bed5b",
            args.rpc_url.parse().unwrap(),
        )
        .unwrap(),
    );
    let contracts = Arc::new(
        ContractBuilder::new(&wallet)
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .with_stake_manager()
            .build()
            .unwrap(),
    );

    let cancellation_token = CancellationToken::new();
    let node_store_clone = node_store.clone();
    let contracts_clone = contracts.clone();
    let last_chain_sync = Arc::new(Mutex::new(None::<std::time::SystemTime>));
    let heartbeat_server_clone = last_chain_sync.clone();

    let chain_sync = ChainSync::new(
        node_store_clone,
        cancellation_token.clone(),
        Duration::from_secs(10),
        contracts,
        last_chain_sync,
    );
    chain_sync.run().await?;

    if let Err(err) = start_server(
        "0.0.0.0",
        args.port,
        node_store,
        contracts_clone,
        args.platform_api_key,
        heartbeat_server_clone,
        args.only_one_node_per_ip.unwrap_or(false),
    )
    .await
    {
        error!("❌ Failed to start server: {}", err);
    }

    cancellation_token.cancel();

    Ok(())
}
