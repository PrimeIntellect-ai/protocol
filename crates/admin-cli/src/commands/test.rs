use alloy::primitives::utils::Unit;
use alloy::primitives::{Address, U256};
use clap::Subcommand;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use tokio::time::{sleep, Duration};
use url::Url;

use crate::config::Config;
use crate::secure_key::get_private_key;

#[derive(Subcommand)]
pub enum TestCommands {
    /// Test concurrent blockchain calls
    ConcurrentCalls {
        /// Address to test with
        #[arg(short = 'a', long)]
        address: String,

        /// Amount for test transactions
        #[arg(short = 'm', long, default_value = "1")]
        amount: u64,

        /// Number of concurrent calls
        #[arg(short = 'c', long, default_value = "5")]
        concurrency: usize,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: TestCommands, config: &Config) -> Result<()> {
    match command {
        TestCommands::ConcurrentCalls {
            address,
            amount,
            concurrency,
            key,
        } => test_concurrent_calls(address, amount, concurrency, key, config).await,
    }
}

async fn test_concurrent_calls(
    address: String,
    amount: u64,
    concurrency: usize,
    key: Option<String>,
    config: &Config,
) -> Result<()> {
    let private_key =
        get_private_key(key.or_else(|| config.get_default_key("federator").cloned()))?;
    let rpc_url = config.get_rpc_url()?;

    let wallet = Wallet::new(
        &private_key,
        Url::parse(&rpc_url).map_err(|e| eyre::eyre!("URL parse error: {}", e))?,
    )
    .map_err(|e| eyre::eyre!("Wallet creation error: {}", e))?;
    let contracts = ContractBuilder::new(wallet.provider())
        .with_compute_registry()
        .with_ai_token()
        .with_prime_network()
        .with_compute_pool()
        .build()?;

    let target_address = Address::from_str(&address)?;
    let mint_amount = U256::from(amount) * Unit::ETHER.wei();

    println!(
        "Testing {} concurrent calls to mint {} tokens to {}",
        concurrency, amount, address
    );
    println!("Starting concurrent operations...");

    let mut handles = vec![];

    for i in 0..concurrency {
        let contracts_clone = contracts.clone();
        let target_address_clone = target_address;
        let mint_amount_clone = mint_amount;

        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            println!("  Call {} starting...", i + 1);

            let result = contracts_clone
                .ai_token
                .mint(target_address_clone, mint_amount_clone)
                .await;
            let duration = start.elapsed();

            match result {
                Ok(tx) => {
                    println!("  Call {} completed in {:?}: {:?}", i + 1, duration, tx);
                    Ok(())
                }
                Err(e) => {
                    println!("  Call {} failed after {:?}: {}", i + 1, duration, e);
                    Err(eyre::eyre!("Transaction failed: {}", e))
                }
            }
        });

        handles.push(handle);

        sleep(Duration::from_millis(100)).await;
    }

    let mut successful = 0;
    let mut failed = 0;

    for (i, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(())) => {
                successful += 1;
            }
            Ok(Err(_)) | Err(_) => {
                failed += 1;
                println!("  Handle {} failed to complete", i + 1);
            }
        }
    }

    println!("\nConcurrent call test completed:");
    println!("  Successful: {}", successful);
    println!("  Failed: {}", failed);
    println!("  Total: {}", successful + failed);

    let final_balance = contracts.ai_token.balance_of(target_address).await;
    match final_balance {
        Ok(balance) => {
            println!("Final balance: {} AI tokens", balance / Unit::ETHER.wei());
        }
        Err(e) => {
            println!("Failed to get final balance: {}", e);
        }
    }

    Ok(())
}
