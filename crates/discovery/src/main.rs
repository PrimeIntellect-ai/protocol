mod api;
mod chainsync;
mod store;
use crate::api::server::start_server;
use crate::chainsync::ChainSync;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use alloy::providers::RootProvider;
use anyhow::Result;
use clap::Parser;
use log::error;
use log::LevelFilter;
use shared::web3::contracts::core::builder::ContractBuilder;
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
    let chain_sync = ChainSync::new(
        node_store.clone(),
        cancellation_token.clone(),
        Duration::from_secs(10),
        contracts.clone(),
        last_chain_sync.clone(),
    );
    chain_sync.run().await?;

    if let Err(err) = start_server(
        "0.0.0.0",
        args.port,
        node_store,
        contracts,
        args.platform_api_key,
        last_chain_sync,
        args.only_one_node_per_ip.unwrap_or(false),
    )
    .await
    {
        error!("❌ Failed to start server: {}", err);
    }

    cancellation_token.cancel();

    Ok(())
}
