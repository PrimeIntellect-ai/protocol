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
pub enum ProviderCommands {
    /// Whitelist a provider
    Whitelist {
        /// Provider address to whitelist
        #[arg(short = 'p', long)]
        provider_address: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: ProviderCommands, config: &Config) -> Result<()> {
    match command {
        ProviderCommands::Whitelist {
            provider_address,
            key,
        } => whitelist_provider(provider_address, key, config).await,
    }
}

async fn whitelist_provider(
    provider_address: String,
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

    let provider_addr = Address::from_str(&provider_address)?;

    println!("Whitelisting provider: {}", provider_address);

    let tx = contracts
        .prime_network
        .whitelist_provider(provider_addr)
        .await;
    println!("Transaction: {:?}", tx);

    Ok(())
}
