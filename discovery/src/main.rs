mod api;
mod models;
mod store;
use anyhow::Result;
use clap::Parser;
use crate::store::redis::RedisStore;
use log::LevelFilter;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::task::JoinSet;
use url::Url;
use actix_web::{App, HttpServer};
use crate::api::server::start_server;
use crate::store::node_store::NodeStore;

#[derive(Parser)]
struct Args {
    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,
}

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
