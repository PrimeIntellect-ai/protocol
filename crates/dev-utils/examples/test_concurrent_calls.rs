use alloy::eips::BlockId;
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::utils::Unit;
use alloy::primitives::Address;
use alloy::primitives::U256;
use alloy::providers::Provider;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::contracts::helpers::utils::retry_call;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::Arc;
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
    #[arg(short = 'm', long, default_value = "30000")]
    amount: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let wallet = Arc::new(Wallet::new(&args.key, Url::parse(&args.rpc_url)?).unwrap());

    let price = wallet.provider.get_gas_price().await?;
    println!("Gas price: {:?}", price);

    let current_nonce = wallet
        .provider
        .get_transaction_count(wallet.address())
        .await?;
    let pending_nonce = wallet
        .provider
        .get_transaction_count(wallet.address())
        .block_id(BlockId::Number(BlockNumberOrTag::Pending))
        .await?;

    println!("Pending nonce: {:?}", pending_nonce);
    println!("Current nonce: {:?}", current_nonce);

    // Unfortunately have to build all contracts atm
    let contracts = Arc::new(
        ContractBuilder::new(&wallet)
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .build()
            .unwrap(),
    );

    let address = Address::from_str(&args.address).unwrap();
    let amount = U256::from(args.amount) * Unit::ETHER.wei();
    let random = (rand::random::<u8>() % 10) + 1;
    println!("Random: {:?}", random);

    let contracts_one = contracts.clone();
    let wallet_one = wallet.clone();
    tokio::spawn(async move {
        let mint_call = contracts_one
            .ai_token
            .build_mint_call(address, amount)
            .unwrap();

        let tx = retry_call(mint_call, 5, wallet_one.provider.clone(), None)
            .await
            .unwrap();
        println!("Transaction hash I: {:?}", tx);
    });

    let contracts_two = contracts.clone();
    let wallet_two = wallet.clone();
    tokio::spawn(async move {
        let mint_call_two = contracts_two
            .ai_token
            .build_mint_call(address, amount)
            .unwrap();
        let tx = retry_call(mint_call_two, 5, wallet_two.provider.clone(), None)
            .await
            .unwrap();
        println!("Transaction hash II: {:?}", tx);
    });

    let balance = contracts.ai_token.balance_of(address).await.unwrap();
    println!("Balance: {:?}", balance);
    tokio::time::sleep(tokio::time::Duration::from_secs(40)).await;
    Ok(())
}
