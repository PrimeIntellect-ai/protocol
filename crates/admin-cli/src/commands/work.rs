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
pub enum WorkCommands {
    /// Submit work to a compute pool
    Submit {
        /// Pool ID to submit work to
        #[arg(short = 'p', long)]
        pool_id: u64,

        /// Node address
        #[arg(short = 'n', long)]
        node: String,

        /// Work key identifier
        #[arg(short = 'w', long)]
        work_key: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Invalidate submitted work with penalty
    Invalidate {
        /// Pool ID containing the work
        #[arg(short = 'p', long)]
        pool_id: u64,

        /// Penalty amount
        #[arg(long)]
        penalty: String,

        /// Work key identifier
        #[arg(short = 'w', long)]
        work_key: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: WorkCommands, config: &Config) -> Result<()> {
    match command {
        WorkCommands::Submit {
            pool_id,
            node,
            work_key,
            key,
        } => submit_work(pool_id, node, work_key, key, config).await,
        WorkCommands::Invalidate {
            pool_id,
            penalty,
            work_key,
            key,
        } => invalidate_work(pool_id, penalty, work_key, key, config).await,
    }
}

async fn submit_work(
    pool_id: u64,
    node: String,
    work_key: String,
    key: Option<String>,
    config: &Config,
) -> Result<()> {
    let private_key = get_private_key(key.or_else(|| config.get_default_key("provider").cloned()))?;
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

    let node_addr = Address::from_str(&node)?;

    println!("Submitting work:");
    println!("  Pool ID: {}", pool_id);
    println!("  Node: {}", node);
    println!("  Work key: {}", work_key);

    let work_data = work_key.as_bytes().to_vec();
    let work_units = U256::from(1u64); // Default to 1 work unit

    let call_builder = contracts
        .compute_pool
        .build_work_submission_call(U256::from(pool_id), node_addr, work_data, work_units)
        .await
        .map_err(|e| eyre::eyre!("Failed to build work submission call: {}", e))?;

    let tx = call_builder.send().await;
    println!("Transaction: {:?}", tx);

    Ok(())
}

async fn invalidate_work(
    pool_id: u64,
    penalty: String,
    work_key: String,
    key: Option<String>,
    config: &Config,
) -> Result<()> {
    let private_key =
        get_private_key(key.or_else(|| config.get_default_key("validator").cloned()))?;
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

    let penalty_f64: f64 = penalty
        .parse()
        .map_err(|_| eyre::eyre!("Invalid penalty amount format"))?;
    let penalty_wei = U256::from((penalty_f64 * 1e18) as u128);

    println!("Invalidating work:");
    println!("  Pool ID: {}", pool_id);
    println!("  Work key: {}", work_key);
    println!("  Penalty: {} tokens", penalty_f64);

    let work_data = work_key.as_bytes().to_vec();

    let tx = contracts
        .prime_network
        .invalidate_work(U256::from(pool_id), penalty_wei, work_data)
        .await;
    println!("Transaction: {:?}", tx);

    Ok(())
}
