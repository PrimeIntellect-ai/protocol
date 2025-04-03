use alloy::primitives::U256; // Already present
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use url::Url;
use hex;
use std::str::FromStr; // Added to bring FromStr into scope

#[derive(Parser)]
struct Args {
    #[arg(long)]
    pool_id: u64,
    #[arg(long)]
    penalty: String,
    #[arg(long)]
    work_key: String,
    #[arg(long)]
    key: String,
    #[arg(long)]
    rpc_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Parse RPC URL with proper error handling
    let rpc_url = Url::parse(&args.rpc_url)
        .map_err(|e| eyre::eyre!("Invalid RPC URL: {}", e))?;

    // Create wallet with error conversion
    let wallet = Wallet::new(&args.key, rpc_url)
        .map_err(|e| eyre::eyre!("Failed to create wallet: {}", e))?;

    // Build the PrimeNetwork contract
    let contracts = ContractBuilder::new(&wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .map_err(|e| eyre::eyre!("Failed to build contracts: {}", e))?;

    // Convert arguments to appropriate types
    let pool_id = U256::from(args.pool_id);
    let penalty = U256::from_str(&args.penalty)
        .map_err(|e| eyre::eyre!("Invalid penalty value: {}", e))?;
    let work_key = hex::decode(args.work_key.trim_start_matches("0x"))
        .map_err(|e| eyre::eyre!("Invalid work key hex: {}", e))?;

    // Validate work_key length
    if work_key.len() != 32 {
        return Err(eyre::eyre!("Work key must be 32 bytes"));
    }

    let data = work_key; // Use the decoded work_key as data

    // Call invalidate_work on the PrimeNetwork contract
    let tx = contracts
        .prime_network
        .invalidate_work(pool_id, penalty, data)
        .await
        .map_err(|e| eyre::eyre!("Failed to invalidate work: {}", e))?;

    println!(
        "Invalidated work in pool {} with penalty {}",
        args.pool_id, args.penalty
    );
    println!("Transaction hash: {:?}", tx);

    Ok(())
}