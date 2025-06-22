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
pub enum DomainCommands {
    /// Create a new domain
    Create {
        /// Domain name to create
        #[arg(short = 'd', long)]
        domain_name: String,

        /// Validation logic address
        #[arg(short = 'v', long)]
        validation_logic: Option<String>,

        /// Domain URI
        #[arg(short = 'u', long)]
        domain_uri: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Create a training domain (preset)
    CreateTraining {
        /// Domain name (defaults to "training")
        #[arg(short = 'd', long)]
        domain_name: Option<String>,

        /// Domain URI
        #[arg(short = 'u', long, default_value = "http://default.uri")]
        domain_uri: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
    /// Create a synthetic data domain (preset)
    CreateSynthData {
        /// Domain name (defaults to "synth_data")
        #[arg(short = 'd', long)]
        domain_name: Option<String>,

        /// Domain URI
        #[arg(short = 'u', long, default_value = "http://default.uri")]
        domain_uri: String,

        /// Private key source (env:VAR_NAME, file:/path, or interactive)
        #[arg(short = 'k', long)]
        key: Option<String>,
    },
}

pub async fn handle_command(command: DomainCommands, config: &Config) -> Result<()> {
    match command {
        DomainCommands::Create {
            domain_name,
            validation_logic,
            domain_uri,
            key,
        } => create_domain(domain_name, validation_logic, domain_uri, key, config).await,
        DomainCommands::CreateTraining {
            domain_name,
            domain_uri,
            key,
        } => {
            let name = domain_name.unwrap_or_else(|| "training".to_string());
            let validation_logic = config.get_contract_address("work_validation").cloned();
            create_domain(name, validation_logic, domain_uri, key, config).await
        }
        DomainCommands::CreateSynthData {
            domain_name,
            domain_uri,
            key,
        } => {
            let name = domain_name.unwrap_or_else(|| "synth_data".to_string());
            let validation_logic = config.get_contract_address("work_validation").cloned();
            create_domain(name, validation_logic, domain_uri, key, config).await
        }
    }
}

async fn create_domain(
    domain_name: String,
    validation_logic: Option<String>,
    domain_uri: String,
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
        .with_domain_registry()
        .build()?;

    let validation_logic = validation_logic
        .as_deref()
        .unwrap_or("0x0000000000000000000000000000000000000000");
    let validation_logic_addr = Address::from_str(validation_logic)?;

    println!("Creating domain: {}", domain_name);
    println!("Validation logic: {}", validation_logic);
    println!("Domain URI: {}", domain_uri);

    let tx = contracts
        .prime_network
        .create_domain(
            domain_name.clone(),
            validation_logic_addr,
            domain_uri.clone(),
        )
        .await;

    println!("Transaction: {:?}", tx);

    Ok(())
}
