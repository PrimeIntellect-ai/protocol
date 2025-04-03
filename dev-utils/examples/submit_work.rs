use alloy::primitives::{Address, U256};
use clap::Parser;
use eyre::Result;
use hex;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Pool ID
    #[arg(short = 'p', long)]
    pool_id: u32,

    /// Node address
    #[arg(short = 'n', long)]
    node: String,

    /// Work key (32-byte hex string)
    #[arg(short = 'w', long)]
    work_key: String,

    /// Private key for transaction signing (provider's private key)
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

    // Build all contracts
    let contracts = ContractBuilder::new(&wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();

    let pool_id = U256::from(args.pool_id);
    let node = Address::from_str(&args.node).expect("Invalid node address");
    let work_key = hex::decode(&args.work_key).expect("Invalid work key hex");

    if work_key.len() != 32 {
        panic!("Work key must be 32 bytes");
    }

    let data = work_key; // 32 bytes, used as workKey in submit_work

    let tx = contracts
        .compute_pool
        .submit_work(pool_id, node, data)
        .await
        .map_err(|e| eyre::eyre!("Failed to submit work: {}", e))?;
    println!(
        "Submitted work for node {} in pool {}",
        args.node, args.pool_id
    );
    println!("Transaction hash: {:?}", tx);

    Ok(())
}
