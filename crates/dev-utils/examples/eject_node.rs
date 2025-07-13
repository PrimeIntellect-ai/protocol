use alloy::primitives::Address;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Private key for transaction signing
    /// The address of this key must be the pool creator or manager
    #[arg(short = 'k', long)]
    key: String,

    /// RPC URL
    #[arg(short = 'r', long)]
    rpc_url: String,

    /// Pool ID
    #[arg(short = 'p', long)]
    pool_id: u32,

    /// Provider address
    #[arg(short = 'a', long)]
    provider_address: String,

    /// Node address to eject
    #[arg(short = 'n', long)]
    node: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let wallet = Wallet::new(&args.key, Url::parse(&args.rpc_url)?).unwrap();

    // Build all contracts
    let contracts = ContractBuilder::new(wallet.provider())
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();

    let node_address = Address::from_str(&args.node).expect("Invalid node address");
    let provider_address =
        Address::from_str(&args.provider_address).expect("Invalid provider address");

    let node_info = contracts
        .compute_registry
        .get_node(provider_address, node_address)
        .await;
    println!("Node info: {node_info:?}");

    let tx = contracts
        .compute_pool
        .eject_node(args.pool_id, node_address)
        .await;
    println!("Ejected node {} from pool {}", args.node, args.pool_id);
    println!("Transaction: {tx:?}");

    let node_info = contracts
        .compute_registry
        .get_node(provider_address, node_address)
        .await;
    println!("Post ejection node info: {node_info:?}");

    Ok(())
}
