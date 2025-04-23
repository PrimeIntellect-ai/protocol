use alloy::primitives::Address;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Provider address
    #[arg(short = 'p', long)]
    provider_address: String,

    /// Node address
    #[arg(short = 'n', long)]
    node_address: String,

    /// Private key for transaction signing
    #[arg(short = 'k', long)]
    key: String,

    /// RPC URL
    #[arg(short = 'r', long)]
    rpc_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let wallet = Wallet::new(&args.key, Url::parse(&args.rpc_url)?).unwrap();

    // Build the contract
    let contracts = ContractBuilder::new(&wallet)
        .with_compute_registry()
        .with_ai_token() // Initialize AI Token
        .with_prime_network() // Initialize Prime Network
        .with_compute_pool()
        .build()
        .unwrap();

    let provider_address = Address::from_str(&args.provider_address).unwrap();
    let node_address = Address::from_str(&args.node_address).unwrap();

    // Get node info
    let (active, validated) = contracts
        .compute_registry
        .get_node(provider_address, node_address)
        .await
        .unwrap();

    let is_node_in_pool = contracts
        .compute_pool
        .is_node_in_pool(0, node_address)
        .await
        .unwrap();

    println!(
        "Node Active: {}, Validated: {}, In Pool: {}",
        active, validated, is_node_in_pool
    );
    Ok(())
}
