use alloy::primitives::{Address, U256};
use clap::Subcommand;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

use crate::config::Config;
use crate::secure_key::get_private_key;

#[derive(Subcommand)]
pub enum PoolCommands {
    /// Create a new compute pool
    Create {
        /// Domain ID for the pool
        #[arg(short = 'd', long, default_value = "0")]
        domain_id: u64,

        /// Compute manager key/address
        #[arg(long)]
        compute_manager_key: String,

        /// Pool name
        #[arg(short = 'n', long)]
        pool_name: String,

        /// Pool data URI
        #[arg(long)]
        pool_data_uri: String,

        /// Compute limit for the pool
        #[arg(long, default_value = "1000")]
        compute_limit: u64,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Start a compute pool
    Start {
        /// Pool ID to start
        #[arg(short = 'p', long, default_value = "0")]
        pool_id: u64,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: PoolCommands, config: &Config) -> Result<()> {
    match command {
        PoolCommands::Create {
            domain_id,
            compute_manager_key,
            pool_name,
            pool_data_uri,
            compute_limit,
            key,
        } => {
            create_pool(
                domain_id,
                compute_manager_key,
                pool_name,
                pool_data_uri,
                compute_limit,
                key,
                config,
            )
            .await
        }
        PoolCommands::Start { pool_id, key } => start_pool(pool_id, key, config).await,
    }
}

async fn create_pool(
    domain_id: u64,
    compute_manager_key: String,
    pool_name: String,
    pool_data_uri: String,
    compute_limit: u64,
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

    let compute_manager_addr = Address::from_str(&compute_manager_key)?;

    println!("Creating compute pool:");
    println!("  Domain ID: {}", domain_id);
    println!("  Pool name: {}", pool_name);
    println!("  Compute manager: {}", compute_manager_key);
    println!("  Data URI: {}", pool_data_uri);
    println!("  Compute limit: {}", compute_limit);

    let tx = contracts
        .compute_pool
        .create_compute_pool(
            U256::from(domain_id),
            compute_manager_addr,
            pool_name,
            pool_data_uri,
            U256::from(compute_limit),
        )
        .await;

    println!("Transaction: {:?}", tx);

    Ok(())
}

async fn start_pool(pool_id: u64, key: Option<String>, config: &Config) -> Result<()> {
    let private_key =
        get_private_key(key.or_else(|| config.get_default_key("pool_owner").cloned()))?;
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

    println!("Starting compute pool ID: {}", pool_id);

    let tx = contracts
        .compute_pool
        .start_compute_pool(U256::from(pool_id))
        .await;
    println!("Transaction: {:?}", tx);

    Ok(())
}
