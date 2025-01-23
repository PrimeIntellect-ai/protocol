use alloy::primitives::Address;
use alloy::primitives::U256;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Domain ID to create the compute pool for
    #[arg(short = 'd', long)]
    domain_id: U256,

    /// Compute manager key address
    #[arg(short = 'm', long)]
    compute_manager_key: String,

    /// Pool name
    #[arg(short = 'n', long)]
    pool_name: String,

    /// Pool data URI
    #[arg(short = 'u', long)]
    pool_data_uri: String,

    /// Private key for transaction signing
    /// The address of this key will be the creator of the compute pool
    /// They are the only one who can actually set the pool to active
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

    let domain_id = args.domain_id;
    let compute_manager_key = Address::from_str(&args.compute_manager_key).unwrap();
    let pool_name = args.pool_name.clone();
    let pool_data_uri = args.pool_data_uri.clone();

    let compute_limit = U256::from(0);

    let tx = contracts
        .compute_pool
        .create_compute_pool(
            domain_id,
            compute_manager_key,
            pool_name,
            pool_data_uri,
            compute_limit,
        )
        .await;
    println!("Transaction: {:?}", tx);

    // TODO: Get this pools id and show pool info
    //let pool_info = contracts.compute_pool.get_pool_info(U256::from(0)).await;
    //println!("Pool info: {:?}", pool_info);

    let tx = contracts
        .compute_pool
        .start_compute_pool(U256::from(0))
        .await;

    Ok(())
}
