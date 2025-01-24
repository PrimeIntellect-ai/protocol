use alloy::primitives::U256;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Minimum stake amount to set
    #[arg(short = 'm', long)]
    min_stake_amount: u64,

    /// Private key for transaction signing
    #[arg(short = 'k', long)]
    key: String,

    /// RPC URL
    #[arg(short = 'r', long)]
    rpc_url: String,
}

async fn set_min_stake_amount(min_stake_amount: u64, key: String, rpc_url: String) -> Result<()> {
    let wallet = Wallet::new(&key, Url::parse(&rpc_url)?).unwrap();

    // Build all contracts
    let contracts = ContractBuilder::new(&wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();

    let min_stake_amount = U256::from(min_stake_amount);

    let tx = contracts
        .prime_network
        .set_stake_minimum(min_stake_amount)
        .await;
    println!("Setting minimum stake amount: {}", min_stake_amount);
    println!("Transaction: {:?}", tx);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    set_min_stake_amount(args.min_stake_amount, args.key, args.rpc_url).await
}
