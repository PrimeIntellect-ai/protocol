use crate::checks::hardware::HardwareChecker;
use crate::checks::software::software_check;
use crate::console::Console;
use crate::operations::compute_node::ComputeNodeOperations;
use crate::operations::provider::ProviderOperations;
use crate::operations::structs::node::NodeConfig;
use crate::services::discovery::DiscoveryService;
use crate::{api::server::start_server, operations::structs::node};
use alloy::network::NetworkWallet;
use alloy::primitives::Address;
use clap::{Parser, Subcommand};
use colored::*;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
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
    Check {},
}

pub fn execute_command(command: &Commands) {
    match command {
        Commands::Check {} => {
            Console::section("🔍 PRIME MINER SYSTEM CHECK");
            Console::info("═", &"═".repeat(50));

            // Run hardware checks
            let hardware_checker = HardwareChecker::new();
            let node_config = NodeConfig {
                ip_address: String::new(), // Placeholder, adjust as necessary
                port: 0,                   // Placeholder, adjust as necessary
                compute_specs: None,
                provider_address: None,
                compute_pool_id: 0, // Placeholder, adjust as necessary
            };

            match hardware_checker.enrich_node_config(node_config) {
                Ok(_) => {
                    Console::success("✅ Hardware check passed!");
                }
                Err(err) => {
                    Console::error(&format!("❌ Hardware check failed: {}", err));
                    std::process::exit(1);
                }
            }
            let _ = software_check::run_software_check();

            std::process::exit(0);
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
            Console::section("🚀 PRIME MINER INITIALIZATION");
            /*
             Initialize Wallet instances
            */
            let provider_wallet_instance = Arc::new(
                match Wallet::new(private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        Console::error(&format!("❌ Failed to create wallet: {}", err));
                        std::process::exit(1);
                    }
                },
            );

            let node_wallet_instance = Arc::new(
                match Wallet::new(private_key_node, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        Console::error(&format!("❌ Failed to create wallet: {}", err));
                        std::process::exit(1);
                    }
                },
            );

            /*
             Initialize dependencies - services, contracts, operations
            */
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let contracts = Arc::new(
                ContractBuilder::new(&provider_wallet_instance)
                    .with_compute_registry()
                    .with_ai_token()
                    .with_prime_network()
                    .with_compute_pool()
                    .build()
                    .unwrap(),
            );

            let provider_ops = ProviderOperations::new(
                &provider_wallet_instance,
                &contracts.compute_registry,
                &contracts.ai_token,
                &contracts.prime_network,
            );
            let compute_node_ops = ComputeNodeOperations::new(
                &provider_wallet_instance,
                &node_wallet_instance,
                &contracts.compute_registry,
                &contracts.prime_network,
            );

            let discovery_service = DiscoveryService::new(&node_wallet_instance, None, None);

            Console::info("═", &"═".repeat(50));
            // Steps:
            // 1. Ensure we have enough eth in our wallet to register on training run
            // Display the public address of the wallet

            // 2. Run system checks
            let node_config = NodeConfig {
                ip_address: external_ip.to_string(),
                port: *port,
                provider_address: Some(provider_wallet_instance.wallet.default_signer().address()),
                compute_specs: None,
                compute_pool_id: *compute_pool_id,
            };

            let hardware_check = HardwareChecker::new();
            let node_config = hardware_check.enrich_node_config(node_config).unwrap();

            // TODO: Move to proper check
            let _ = software_check::run_software_check();

            let balance = runtime
                .block_on(provider_wallet_instance.get_balance())
                .unwrap();
            Console::info("💰 Balance", &format!("{:?}", balance));

            if let Err(e) = runtime.block_on(provider_ops.register_provider()) {
                Console::error(&format!("❌ Failed to register provider: {}", e));
                std::process::exit(1);
            }

            if let Err(e) = runtime.block_on(compute_node_ops.add_compute_node()) {
                Console::error(&format!("❌ Failed to add compute node: {}", e));
                std::process::exit(1);
            }

            if let Err(e) = runtime.block_on(discovery_service.upload_discovery_info(&node_config))
            {
                Console::error(&format!("❌ Failed to upload discovery info: {}", e));
                std::process::exit(1);
            }

            Console::success("✅ Discovery info uploaded");

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            Console::info("🌐 Starting endpoint service", "");

            if let Err(err) = runtime.block_on(start_server(
                external_ip,
                *port,
                contracts.clone(),
                node_wallet_instance.clone(),
                provider_wallet_instance.clone(),
            )) {
                Console::error(&format!("❌ Failed to start server: {}", err));
                std::process::exit(1);
            }
        }
    }
}
