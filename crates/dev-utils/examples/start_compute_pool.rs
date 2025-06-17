use alloy::primitives::U256;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Private key for transaction signing
    /// The address of this key will be the creator of the compute pool
    /// They are the only one who can actually set the pool to active
    #[arg(short = 'k', long)]
    key: String,

    /// RPC URL
    #[arg(short = 'r', long)]
    rpc_url: String,

    /// Pool ID
    #[arg(short = 'p', long)]
    pool_id: U256,
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

    let tx = contracts
        .compute_pool
        .start_compute_pool(U256::from(args.pool_id))
        .await;
    println!("Started compute pool with id: {}", args.pool_id);
    println!("Transaction: {:?}", tx);
    Ok(())
}
