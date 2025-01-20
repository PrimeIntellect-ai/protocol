mod api;
mod discovery;
mod node;
mod store;
mod types;
use crate::api::server::start_server;
use crate::discovery::monitor::DiscoveryMonitor;
use crate::node::invite::NodeInviter;
use crate::store::redis::RedisStore;
use alloy::primitives::U256;
use anyhow::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::env;
use std::sync::Arc;
use tokio::task::JoinSet;
use url::Url;

#[tokio::main]
async fn main() -> Result<()> {
    let compute_pool_id = 0;
    let domain_id = 0;
    let coordinator_key = env::var("POOL_OWNER_PRIVATE_KEY").unwrap();
    let rpc_url = "http://localhost:8545";

    let mut tasks: JoinSet<Result<()>> = JoinSet::new();

    let coordinator_wallet = Arc::new(
        Wallet::new(&coordinator_key, Url::parse(rpc_url).unwrap()).unwrap_or_else(|err| {
            eprintln!("Error creating wallet: {:?}", err);
            std::process::exit(1);
        }),
    );

    let contracts = ContractBuilder::new(&coordinator_wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()?;

    // TODO: Move to utils - only here for debug
    let tx = contracts
        .compute_pool
        .start_compute_pool(U256::from(compute_pool_id))
        .await;
    println!("Start pool Tx: {:?}", tx);

    let store = RedisStore::new("redis://localhost:6379");
    let store_clone = store.clone();
    let wallet_clone = coordinator_wallet.clone();

    tasks.spawn(async move {
        let monitor = DiscoveryMonitor::new(store_clone, wallet_clone.as_ref(), compute_pool_id);
        monitor.run().await
    });

    tasks.spawn(async move {
        let inviter = NodeInviter::new(
            store.clone(),
            coordinator_wallet.as_ref(),
            compute_pool_id,
            domain_id,
        );
        inviter.run().await
    });

    tokio::select! {
        res = start_server("0.0.0.0", 8090) => {
            if let Err(e) = res {
                eprintln!("Server error: {}", e);
            }
        }
        Some(res) = tasks.join_next() => {
            if let Err(e) = res? {
                eprintln!("Task error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {  // Fix the asterisks here
            println!("Shutdown signal received");
        }
    }
    tasks.shutdown().await;
    Ok(())

    // Wait for Ctrl+C
    // First core requirement
    // The orchestrator automatically sources nodes from chain that are interested in joining - continiously queries discovery service (simple db that gives us the ip)
    // Get list of nodes from discovery service
    // Create invite for nodes to join - signing invite here
    // send invites to nodes

    // Have nodes send heartbeats to orchestrator - keep state of connected and healthy nodes
    // Have a simple api to accept task and forward to nodes

    // Need to store the current state of connected nodes
    // Nodes check in with their heartrates and current metrics - might eventually move this out to diff service
    // This service has an api to accept tasks from the core developer - these tasks are distributed to the nodes
}
