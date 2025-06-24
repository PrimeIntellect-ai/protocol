use alloy::primitives::utils::Unit;
use alloy::primitives::{Address, U256};
use alloy::providers::Provider;
use clap::Subcommand;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

use crate::config::Config;
use crate::secure_key::get_private_key;

#[derive(Subcommand)]
pub enum TokenCommands {
    /// Mint AI tokens to an address
    Mint {
        /// Address to mint tokens to
        #[arg(short = 'a', long)]
        address: String,

        /// Amount to mint (in ETH units)
        #[arg(short = 'm', long, default_value = "30000")]
        amount: u64,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Transfer ETH between accounts
    TransferEth {
        /// Address to transfer ETH to
        #[arg(short = 'a', long)]
        address: String,

        /// Amount to transfer (in ETH units, supports decimals)
        #[arg(short = 'm', long)]
        amount: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Set minimum stake amount
    SetMinStake {
        /// Minimum stake amount
        #[arg(long)]
        min_stake_amount: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: TokenCommands, config: &Config) -> Result<()> {
    match command {
        TokenCommands::Mint {
            address,
            amount,
            key,
        } => mint_tokens(address, amount, key, config).await,
        TokenCommands::TransferEth {
            address,
            amount,
            key,
        } => transfer_eth(address, amount, key, config).await,
        TokenCommands::SetMinStake {
            min_stake_amount,
            key,
        } => set_min_stake(min_stake_amount, key, config).await,
    }
}

async fn mint_tokens(
    address: String,
    amount: u64,
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

    let address = Address::from_str(&address)?;
    let amount = U256::from(amount) * Unit::ETHER.wei();

    println!(
        "Minting {} AI tokens to address: {}",
        amount / Unit::ETHER.wei(),
        address
    );

    let tx = contracts.ai_token.mint(address, amount).await;
    println!("Transaction: {:?}", tx);

    let balance = contracts
        .ai_token
        .balance_of(address)
        .await
        .map_err(|e| eyre::eyre!("Failed to get balance: {}", e))?;
    println!("New balance: {} AI tokens", balance / Unit::ETHER.wei());

    Ok(())
}

async fn transfer_eth(
    address: String,
    amount: String,
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

    let to_address = Address::from_str(&address)?;
    let amount_f64: f64 = amount
        .parse()
        .map_err(|_| eyre::eyre!("Invalid amount format"))?;
    let amount_wei = U256::from((amount_f64 * 1e18) as u128);

    println!("Transferring {} ETH to address: {}", amount_f64, to_address);

    let provider = wallet.provider();
    let tx_request = alloy::rpc::types::TransactionRequest::default()
        .to(to_address)
        .value(amount_wei)
        .from(wallet.address());

    let tx = provider.send_transaction(tx_request).await?;
    println!("Transaction sent: {:?}", tx.tx_hash());

    let receipt = tx.get_receipt().await?;
    println!("Transaction confirmed in block: {:?}", receipt.block_number);

    Ok(())
}

async fn set_min_stake(
    min_stake_amount: String,
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

    let amount_f64: f64 = min_stake_amount
        .parse()
        .map_err(|_| eyre::eyre!("Invalid stake amount format"))?;
    let amount_wei = U256::from((amount_f64 * 1e18) as u128);

    println!("Setting minimum stake amount to: {} tokens", amount_f64);

    let tx = contracts.prime_network.set_stake_minimum(amount_wei).await;
    println!("Transaction: {:?}", tx);

    Ok(())
}
