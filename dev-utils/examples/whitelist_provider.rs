use alloy::primitives::Address;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use url::Url;

#[derive(Parser)]
struct Args {
    /// Provider address to whitelist
    #[arg(short = 'a', long)]
    provider_address: String,

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

    let provider_address: Address = args.provider_address.parse()?;

    let tx = contracts
        .prime_network
        .whitelist_provider(provider_address)
        .await;
    println!("Whitelisting provider: {}", args.provider_address);
    

    Ok(())
}
