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
    /// Address to mint tokens to
    #[arg(short = 'a', long)]
    address: String,

    /// Private key for transaction signing
    #[arg(short = 'k', long)]
    key: String,

    /// RPC URL
    #[arg(short = 'r', long)]
    rpc_url: String,

    /// Amount to mint
    #[arg(short = 'm', long, default_value = "1000")]
    amount: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let wallet = Wallet::new(&args.key, Url::parse(&args.rpc_url)?).unwrap();

    // Unfortunately have to build all contracts atm
    let contracts = ContractBuilder::new(&wallet)
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()
        .unwrap();

    let address = Address::from_str(&args.address).unwrap();
    let amount = U256::from(args.amount);
    let tx = contracts.ai_token.mint(address, amount).await;
    println!("Minting to address: {}", args.address);
    println!("Transaction: {:?}", tx);

    let balance = contracts.ai_token.balance_of(address).await;
    println!("Balance: {:?}", balance);
    Ok(())
}
