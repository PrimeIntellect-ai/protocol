use alloy::eips::BlockId;
use alloy::eips::BlockNumberOrTag;
use alloy::primitives::utils::Unit;
use alloy::primitives::Address;
use alloy::primitives::U256;
use alloy::providers::Provider;
use clap::Parser;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
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
    let gas_price = wallet.provider.get_gas_price().await.unwrap();
    let random = (rand::random::<u8>() % 10) + 1;
    println!("Random: {:?}", random);

    let contracts_one = contracts.clone();
    let wallet_one = wallet.clone();
    tokio::spawn(async move {
        let tx = contracts_one
            .ai_token
            .mint(
                address,
                amount,
                current_nonce,
                Some(gas_price + random as u128),
            )
            .await
            .unwrap();
        println!("Transaction hash: {:?}", tx);

        let wallet_clone = wallet_one.clone();
        tokio::spawn(async move {
            loop {
                let receipt = wallet_clone
                    .provider
                    .get_transaction_receipt(tx)
                    .await
                    .unwrap();
                if receipt.is_some() {
                    println!("Transaction receipt I: {:?}", receipt);
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        });
    });

    let mut current_gas = gas_price;
    let max_retries = 5;
    let mut retry_count = 0;
    let gas_increase_percentage = 0.1;

    let contracts_two = contracts.clone();
    let wallet_clone = wallet.clone();

    while retry_count < max_retries {
        match contracts_two
            .ai_token
            .mint(address, amount, current_nonce, Some(current_gas))
            .await
        {
            Ok(tx_hash) => {
                println!("Transaction two submitted");
                let wallet_clone = wallet_clone.clone();
                tokio::spawn(async move {
                    println!("Transaction two waiting for receipt");
                    loop {
                        let receipt = wallet_clone
                            .provider
                            .get_transaction_receipt(tx_hash)
                            .await
                            .unwrap();
                        if receipt.is_some() {
                            println!("Transaction receipt II: {:?}", receipt);
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                });
                break;
            }
            Err(e) => {
                if e.to_string()
                    .contains("replacement transaction underpriced")
                {
                    println!(
                        "Transaction underpriced, increasing gas price by {}%. Attempt {}/{}",
                        gas_increase_percentage * 100.0,
                        retry_count + 1,
                        max_retries
                    );
                    current_gas = (current_gas as f64 * (1.0 + gas_increase_percentage)) as u128;
                    retry_count += 1;
                } else {
                    println!("Unexpected error: {:?}", e);
                    break;
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    let balance = contracts.ai_token.balance_of(address).await.unwrap();
    println!("Balance: {:?}", balance);
    tokio::time::sleep(tokio::time::Duration::from_secs(40)).await;
    Ok(())
}
