use alloy::providers::Provider;
use clap::Subcommand;
use eyre::Result;
use rand::RngCore;
use shared::web3::wallet::Wallet;
use url::Url;

use crate::config::Config;

#[derive(Subcommand)]
pub enum WalletCommands {
    /// Generate a new wallet
    Generate {
        /// Output format (json, env, pretty)
        #[arg(short = 'f', long, default_value = "pretty")]
        format: String,

        /// Save to file
        #[arg(short = 'o', long)]
        output: Option<String>,
    },
    /// Get wallet address from private key
    Address {
        /// Private key (32 bytes hex)
        #[arg(short = 'k', long)]
        private_key: String,
    },
    /// Check wallet balance
    Balance {
        /// Private key or address
        #[arg(short = 'k', long)]
        key_or_address: String,

        /// RPC URL (optional, uses config default)
        #[arg(short = 'r', long)]
        rpc_url: Option<String>,
    },
}

pub async fn handle_command(command: WalletCommands, config: &Config) -> Result<()> {
    match command {
        WalletCommands::Generate { format, output } => generate_wallet(format, output).await,
        WalletCommands::Address { private_key } => get_address(private_key).await,
        WalletCommands::Balance {
            key_or_address,
            rpc_url,
        } => check_balance(key_or_address, rpc_url, config).await,
    }
}

async fn generate_wallet(format: String, output: Option<String>) -> Result<()> {
    let mut rng = rand::rng();
    let mut private_key_bytes = [0u8; 32];
    rng.fill_bytes(&mut private_key_bytes);

    let private_key_hex = hex::encode(private_key_bytes);

    let dummy_url =
        Url::parse("http://localhost:8545").map_err(|e| eyre::eyre!("URL parse error: {}", e))?;
    let wallet = Wallet::new(&private_key_hex, dummy_url)
        .map_err(|e| eyre::eyre!("Wallet creation error: {}", e))?;
    let address = wallet.address();

    let output_content = match format.as_str() {
        "json" => serde_json::json!({
            "private_key": private_key_hex,
            "address": format!("{:?}", address)
        })
        .to_string(),
        "env" => {
            format!("PRIVATE_KEY={}\nADDRESS={:?}", private_key_hex, address)
        }
        "pretty" => {
            format!(
                "Generated new wallet:\n  Private Key: {}\n  Address: {:?}",
                private_key_hex, address
            )
        }
        _ => {
            format!(
                "Generated new wallet:\n  Private Key: {}\n  Address: {:?}",
                private_key_hex, address
            )
        }
    };

    if let Some(file_path) = output {
        std::fs::write(&file_path, &output_content)?;
        println!("Wallet saved to: {}", file_path);
    } else {
        println!("{}", output_content);
    }

    Ok(())
}

async fn get_address(private_key: String) -> Result<()> {
    let dummy_url =
        Url::parse("http://localhost:8545").map_err(|e| eyre::eyre!("URL parse error: {}", e))?;
    let wallet = Wallet::new(&private_key, dummy_url)
        .map_err(|e| eyre::eyre!("Wallet creation error: {}", e))?;
    let address = wallet.address();

    println!("Address: {:?}", address);
    Ok(())
}

async fn check_balance(
    key_or_address: String,
    rpc_url: Option<String>,
    config: &Config,
) -> Result<()> {
    let rpc_url = rpc_url
        .or_else(|| config.get_rpc_url().ok())
        .unwrap_or_else(|| "http://localhost:8545".to_string());
    let url = Url::parse(&rpc_url).map_err(|e| eyre::eyre!("URL parse error: {}", e))?;

    let address = if key_or_address.len() == 64
        || key_or_address.starts_with("0x") && key_or_address.len() == 66
    {
        let wallet = Wallet::new(&key_or_address, url.clone())
            .map_err(|e| eyre::eyre!("Wallet creation error: {}", e))?;
        wallet.address()
    } else {
        key_or_address
            .parse()
            .map_err(|_| eyre::eyre!("Invalid address or private key format"))?
    };

    let provider = alloy::providers::ProviderBuilder::new().connect_http(url);
    let balance = provider.get_balance(address).await?;
    let balance_eth = balance.to::<u128>() as f64 / 1e18;

    println!("Address: {:?}", address);
    println!("Balance: {} ETH", balance_eth);

    Ok(())
}
