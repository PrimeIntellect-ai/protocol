use alloy::primitives::Address;
use clap::Subcommand;
use eyre::Result;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use url::Url;

use crate::config::Config;
use crate::secure_key::get_private_key;

#[derive(Subcommand)]
pub enum NodeCommands {
    /// Get information about a node
    Info {
        /// Provider address
        #[arg(short = 'p', long)]
        provider_address: String,

        /// Node address
        #[arg(short = 'n', long)]
        node_address: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Eject a node from a pool
    Eject {
        /// Pool ID to eject from
        #[arg(long)]
        pool_id: u64,

        /// Provider address
        #[arg(short = 'p', long)]
        provider_address: String,

        /// Node address to eject
        #[arg(short = 'n', long)]
        node: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: NodeCommands, config: &Config) -> Result<()> {
    match command {
        NodeCommands::Info {
            provider_address,
            node_address,
            key,
        } => get_node_info(provider_address, node_address, key, config).await,
        NodeCommands::Eject {
            pool_id,
            provider_address,
            node,
            key,
        } => eject_node(pool_id, provider_address, node, key, config).await,
    }
}

async fn get_node_info(
    provider_address: String,
    node_address: String,
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

    let provider_addr = Address::from_str(&provider_address)?;
    let node_addr = Address::from_str(&node_address)?;

    println!("Getting node information:");
    println!("  Provider: {}", provider_address);
    println!("  Node: {}", node_address);

    let node_info = contracts
        .compute_registry
        .get_node(provider_addr, node_addr)
        .await;
    println!("Node info: {:?}", node_info);

    Ok(())
}

async fn eject_node(
    pool_id: u64,
    provider_address: String,
    node: String,
    key: Option<String>,
    config: &Config,
) -> Result<()> {
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

    let _provider_addr = Address::from_str(&provider_address)?;
    let node_addr = Address::from_str(&node)?;

    println!("Ejecting node from pool:");
    println!("  Pool ID: {}", pool_id);
    println!("  Provider: {}", provider_address);
    println!("  Node: {}", node);

    let tx = contracts
        .compute_pool
        .eject_node(pool_id as u32, node_addr)
        .await;
    println!("Transaction: {:?}", tx);

    Ok(())
}
