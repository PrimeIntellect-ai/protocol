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
    /// Domain name to create
    #[arg(short = 'd', long)]
    domain_name: String,

    /// Validation logic address
    #[arg(short = 'v', long, default_value = "0x0000000000000000000000000000000000000000")]
    validation_logic: String,

    /// Domain URI
    #[arg(short = 'u', long)]
    domain_uri: String,

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

    let domain_name = args.domain_name.clone();
    let validation_logic = Address::from_str(&args.validation_logic).unwrap();
    let domain_uri = args.domain_uri.clone();

    let tx = contracts.prime_network.create_domain(domain_name, validation_logic, domain_uri).await;
    println!("Creating domain: {}", args.domain_name);
    println!("Transaction: {:?}", tx);

    // TODO: Get domain info from contract

    Ok(())
}
