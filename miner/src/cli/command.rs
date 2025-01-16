use crate::api::server::start_server;
use crate::checks::hardware::run_hardware_check;
use crate::checks::software::run_software_check;
use crate::node::config::NodeConfig;
use crate::services::discovery::DiscoveryService;
// Import the Wallet struct
use crate::web3::contracts::{
    core::builder::ContractBuilder,
    implementations::{
        ai_token_contract::AIToken, compute_registry_contract::ComputeRegistryContract,
        prime_network_contract::PrimeNetworkContract,
    },
};
use crate::web3::wallet::Wallet;
use alloy::{
    network::TransactionBuilder,
    primitives::utils::keccak256 as keccak,
    primitives::{Address, U256},
    providers::Provider,
    signers::Signer,
};
use clap::{Parser, Subcommand};
use colored::*;
use url::Url;
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Run {
        /// Subnet ID to run the miner on
        #[arg(long)]
        subnet_id: String,

        /// Wallet private key (as a hex string)
        #[arg(long)]
        private_key_provider: String,

        /// Wallet private key (as a hex string)
        #[arg(long)]
        private_key_node: String,

        /// RPC URL
        #[arg(long, default_value = "http://localhost:8545")]
        rpc_url: String,

        /// Port number for the miner to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// External IP address for the miner to advertise
        #[arg(long)]
        external_ip: String,

        /// Compute pool ID
        #[arg(long)]
        compute_pool_id: u64,

        /// Dry run the command without starting the miner
        #[arg(long, default_value = "false")]
        dry_run: bool,
    },
    /// Run system checks to verify hardware and software compatibility
    Check {
        /// Run only hardware checks
        #[arg(long, default_value = "false")]
        hardware_only: bool,

        /// Run only software checks  
        #[arg(long, default_value = "false")]
        software_only: bool,
    },
}

fn run_system_checks(hardware_only: bool, software_only: bool) -> Result<(), String> {
    if !software_only {
        println!("\n[SYS] {}", "Running hardware detection...".bright_blue());
        if let Err(err) = run_hardware_check() {
            eprintln!("{}", format!("Hardware check failed: {}", err).red().bold());
            return Err(err.to_string());
        }
    }

    if !hardware_only {
        println!("\n[SYS] {}", "Running software detection...".bright_blue());
        if let Err(err) = run_software_check() {
            eprintln!("{}", format!("Software check failed: {}", err).red().bold());
            return Err(err.to_string());
        }
    }

    Ok(())
}

pub fn execute_command(command: &Commands) {
    match command {
        Commands::Check {
            hardware_only,
            software_only,
        } => {
            println!("\n{}", "🔍 PRIME MINER SYSTEM CHECK".bright_cyan().bold());
            println!("{}", "═".repeat(50).bright_cyan());

            if run_system_checks(*hardware_only, *software_only).is_err() {
                std::process::exit(1);
            }

            println!(
                "\n[SYS] {}",
                "✅ System check passed!".bright_green().bold()
            );
        }

        Commands::Run {
            subnet_id: _,
            private_key_provider,
            private_key_node,
            port,
            external_ip,
            compute_pool_id,
            dry_run,
            rpc_url,
        } => {
            println!("\n{}", "🚀 PRIME MINER INITIALIZATION".bright_cyan().bold());
            let runtime = tokio::runtime::Runtime::new().unwrap();

            println!("{}", "═".repeat(50).bright_cyan());
            // Steps:
            // 1. Ensure we have enough eth in our wallet to register on training run
            // Display the public address of the wallet
            let provider_wallet_instance =
                match Wallet::new(private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        eprintln!("Failed to create wallet: {}", err);
                        std::process::exit(1);
                    }
                };

            let node_wallet_instance =
                match Wallet::new(private_key_node, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        eprintln!("Failed to create wallet: {}", err);
                        std::process::exit(1);
                    }
                };

            let balance = runtime
                .block_on(provider_wallet_instance.get_balance())
                .unwrap();
            println!("Balance: {:?}", balance);

            let contracts = ContractBuilder::new(&provider_wallet_instance)
                .with_compute_registry()
                .with_ai_token()
                .with_prime_network()
                .build()
                .unwrap();

            // TODO: Register provider on network
            // TODO: Later we do not have to reregister on network
            pub async fn register_provider(
                wallet: &Wallet,
                prime_network_contract: &PrimeNetworkContract,
                ai_token_contract: &AIToken,
                compute_registry_contract: &ComputeRegistryContract,
            ) -> Result<(), Box<dyn std::error::Error>> {
                // TODO[MOVE]: Move provider registration logic to a separate module for better organization
                let address = wallet.wallet.default_signer().address();
                println!("Address: {:?}", address);
                let balance: U256 = ai_token_contract.balance_of(address).await?;

                // Check if we are already provider
                let provider = compute_registry_contract.get_provider(address).await?;
                println!("Provider address: {:?}", provider.provider_address);
                println!("Is whitelisted: {:?}", provider.is_whitelisted);

                let provider_exists = provider.provider_address != Address::default();

                println!("Provider registered: {}", provider_exists);

                println!("Balance: {} tokens", balance);
                if !provider_exists {
                    let stake: U256 = U256::from(100);
                    let approve_tx = ai_token_contract.approve(stake).await?;
                    println!("Transaction approved: {:?}", approve_tx);

                    let register_tx = prime_network_contract.register_provider(stake).await?;
                    println!("Provider registered: {:?}", register_tx);
                }

                // Get provider details again  - cleanup later
                let provider = compute_registry_contract.get_provider(address).await?;
                println!("Provider address: {:?}", provider.provider_address);
                println!("Is whitelisted: {:?}", provider.is_whitelisted);

                let provider_exists = provider.provider_address != Address::default();
                if !provider_exists {
                    eprintln!("Provider could not be registered.");
                    std::process::exit(1);
                }

                if !provider.is_whitelisted {
                    eprintln!("Provider is not whitelisted. Cannot proceed.");
                    std::process::exit(1);
                }

                Ok(())
            }

            if let Err(e) = runtime.block_on(register_provider(
                &provider_wallet_instance,
                &contracts.prime_network,
                &contracts.ai_token,
                &contracts.compute_registry,
            )) {
                eprintln!("Failed to register provider: {}", e);
                std::process::exit(1);
            }

            pub async fn add_compute_node(
                provider_wallet: &Wallet,
                node_wallet: &Wallet,
                prime_network_contract: &PrimeNetworkContract,
                compute_registry_contract: &ComputeRegistryContract,
            ) -> Result<(), Box<dyn std::error::Error>> {
                let compute_node = compute_registry_contract
                    .get_node(
                        provider_wallet.wallet.default_signer().address(),
                        node_wallet.wallet.default_signer().address(),
                    )
                    .await;

                match compute_node {
                    Ok(()) => {
                        println!("Compute node already exists");
                        return Ok(());
                    }
                    Err(e) => {
                        println!("Compute node does not exist - creating");
                    }
                }

                println!("Adding compute node");
                println!(
                    "Provider wallet: {:?}",
                    provider_wallet.wallet.default_signer().address()
                );

                println!(
                    "Node wallet: {:?}",
                    node_wallet.wallet.default_signer().address()
                );

                let provider_address = provider_wallet.wallet.default_signer().address();
                let node_address = node_wallet.wallet.default_signer().address();
                let digest =
                    keccak([provider_address.as_slice(), node_address.as_slice()].concat());

                let signature = node_wallet
                    .signer
                    .sign_message(digest.as_slice())
                    .await?
                    .as_bytes();
                println!("Signature: {:?}", signature);

                // Create the signature bytes
                let compute_units: U256 = U256::from(10);
                let add_node_tx = prime_network_contract
                    .add_compute_node(node_address, compute_units, signature.to_vec())
                    .await?;
                println!("Add node tx: {:?}", add_node_tx);
                Ok(())
            }

            if let Err(e) = runtime.block_on(add_compute_node(
                &provider_wallet_instance,
                &node_wallet_instance,
                &contracts.prime_network,
                &contracts.compute_registry,
            )) {
                eprintln!("Failed to add compute node: {}", e);
                std::process::exit(1);
            }

            // TODO: We need the system info and should share this with discovery service
            if run_system_checks(false, false).is_err() {
                std::process::exit(1);
            }
            let node_config = NodeConfig {
                ip_address: "127.0.0.1".to_string(),
                port: 8080,
                compute_specs: None,
                compute_pool_id: compute_pool_id.clone(),
            };

            println!("Uploading discovery info");
            let discovery_service = DiscoveryService::new(&node_wallet_instance, None, None);

            if let Err(e) = runtime.block_on(discovery_service.upload_discovery_info(&node_config))
            {
                eprintln!("Failed to upload discovery info: {}", e);
                std::process::exit(1);
            }

            println!("{}", "Discovery info uploaded".bright_green().bold());

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            println!("\n[SRV] {}", "Starting endpoint service".bright_white());

            if let Err(err) = runtime.block_on(start_server(external_ip, *port)) {
                eprintln!(
                    "{}",
                    format!("Failed to start server: {}", err).red().bold()
                );
                std::process::exit(1);
            }
        }
    }
}
