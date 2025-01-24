mod api;
mod store;
use crate::api::server::start_server;
use crate::store::node_store::NodeStore;
use crate::store::redis::RedisStore;
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;

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
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let redis_store = RedisStore::new("redis://localhost:6379");
    let node_store = Arc::new(NodeStore::new(redis_store));

    if let Err(err) = start_server("0.0.0.0", 8089, node_store).await {
        println!("❌ Failed to start server: {}", err);
    }

    Ok(())
}
