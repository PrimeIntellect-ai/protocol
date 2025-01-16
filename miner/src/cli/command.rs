use crate::api::server::start_server;
use crate::checks::hardware::HardwareChecker;
use crate::operations::compute_node::ComputeNodeOperations;
use crate::operations::provider::ProviderOperations;
use crate::operations::structs::node::NodeConfig;
use crate::services::discovery::DiscoveryService;
// Import the Wallet struct
use crate::web3::contracts::core::builder::ContractBuilder;
use crate::web3::wallet::Wallet;
use clap::{Parser, Subcommand};
use colored::*;
use url::Url;
use crate::checks::software::software_check; 

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

pub fn execute_command(command: &Commands) {
    match command {
        Commands::Check {
            hardware_only,
            software_only,
        } => {
            println!("\n{}", "🔍 PRIME MINER SYSTEM CHECK".bright_cyan().bold());
            println!("{}", "═".repeat(50).bright_cyan());
            std::process::exit(1);

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
            /*
             Initialize Wallet instances
            */
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

            /*
             Initialize dependencies - services, contracts, operations
            */
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let contracts = ContractBuilder::new(&provider_wallet_instance)
                .with_compute_registry()
                .with_ai_token()
                .with_prime_network()
                .build()
                .unwrap();

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

            println!("{}", "═".repeat(50).bright_cyan());
            // Steps:
            // 1. Ensure we have enough eth in our wallet to register on training run
            // Display the public address of the wallet

            // 2. Run system checks
            let node_config = NodeConfig {
                ip_address: external_ip.to_string(),
                port: *port,
                compute_specs: None,
                compute_pool_id: *compute_pool_id,
            };

            let hardware_check = HardwareChecker::new();
            let node_config = hardware_check.enrich_node_config(node_config).unwrap();

            // TODO: Move to proper check 
            software_check::run_software_check();

            let balance = runtime
                .block_on(provider_wallet_instance.get_balance())
                .unwrap();
            println!("Balance: {:?}", balance);

            if let Err(e) = runtime.block_on(provider_ops.register_provider()) {
                eprintln!("Failed to register provider: {}", e);
                std::process::exit(1);
            }

            if let Err(e) = runtime.block_on(compute_node_ops.add_compute_node()) {
                eprintln!("Failed to add compute node: {}", e);
                std::process::exit(1);
            }

            println!("Uploading discovery info");
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
