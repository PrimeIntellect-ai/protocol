mod api;
mod chainsync;
mod store;
use crate::api::server::start_server;
use crate::chainsync::ChainSync;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use anyhow::Result;
use clap::Parser;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,
}

// TODO: Align node model
// TODO: Get status from chain
// TODO: Add proper validation
// TODO: Chain interaction
// TODO: Align other services reg. api
// TODO: Register miner API call model should be adjusted

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let redis_store = RedisStore::new("redis://localhost:6379");
    let node_store = Arc::new(NodeStore::new(redis_store));
    // TODO: Find a way to read from chain without address
    let wallet = Arc::new(
        Wallet::new(
            "0x0000000000000000000000000000000000000000",
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
            .build()
            .unwrap(),
    );

    let cancellation_token = CancellationToken::new();
    let node_store_clone = node_store.clone();
    let chain_sync = ChainSync::new(
        node_store_clone,
        cancellation_token.clone(),
        Duration::from_secs(10),
        contracts,
    );
    chain_sync.run().await?;

    if let Err(err) = start_server("0.0.0.0", 8089, node_store).await {
        println!("❌ Failed to start server: {}", err);
    }

    cancellation_token.cancel();

    Ok(())
}
