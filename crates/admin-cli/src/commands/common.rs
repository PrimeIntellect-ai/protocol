use eyre::Result;
use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
use shared::web3::wallet::{Wallet, WalletProvider};
use url::Url;

use crate::config::Config;
use crate::secure_key::get_private_key;

pub async fn create_wallet_and_contracts(
    key: Option<String>,
    config: &Config,
) -> Result<(Wallet, Contracts<WalletProvider>)> {
    let private_key = get_private_key(key)?;
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
        .with_domain_registry()
        .build()?;

    Ok((wallet, contracts))
}

pub async fn create_wallet_and_contracts_with_default_key(
    role: &str,
    config: &Config,
) -> Result<(Wallet, Contracts<WalletProvider>)> {
    let default_key = config.get_default_key(role).cloned();
    create_wallet_and_contracts(default_key, config).await
}
