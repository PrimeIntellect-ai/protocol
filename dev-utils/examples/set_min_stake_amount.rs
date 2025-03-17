use alloy::primitives::utils::Unit;
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
    min_stake_amount: f64,

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

    // Build all contracts
    let contracts = ContractBuilder::new(&wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();

    let min_stake_amount = U256::from(args.min_stake_amount) * Unit::ETHER.wei();
    println!("Min stake amount: {}", min_stake_amount);

    let tx = contracts
        .prime_network
        .set_stake_minimum(min_stake_amount)
        .await;
    println!("Transaction: {:?}", tx);

    Ok(())
}
