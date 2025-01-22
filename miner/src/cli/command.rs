use crate::api::server::start_server;
use crate::checks::hardware::HardwareChecker;
use crate::checks::software::software_check;
use crate::console::Console;
use crate::docker::DockerService;
use crate::operations::compute_node::ComputeNodeOperations;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::operations::provider::ProviderError;
use crate::operations::provider::ProviderOperations;
use crate::operations::structs::node::NodeConfig;
use crate::services::discovery::DiscoveryService;
use crate::TaskHandles;
use alloy::primitives::U256;
use clap::{Parser, Subcommand};
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
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

        ///  Optional state storage directory
        #[arg(long)]
        state_dir: Option<String>,
    },
    /// Run system checks to verify hardware and software compatibility
    Check {},
}

pub async fn execute_command(
    command: &Commands,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match command {
        Commands::Run {
            private_key_provider,
            private_key_node,
            port,
            external_ip,
            compute_pool_id,
            dry_run: _,
            rpc_url,
            state_dir,
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
            let docker_service = Arc::new(DockerService::new(cancellation_token.clone()));

            let heartbeat_service = HeartbeatService::new(
                Duration::from_secs(10),
                state_dir.clone(),
                cancellation_token.clone(),
                task_handles.clone(),
                node_wallet_instance.clone(),
                docker_service.clone(),
            );

            tokio::spawn(async move {
                if let Err(e) = docker_service.run().await {
                    Console::error(&format!("❌ Docker service failed: {}", e));
                }
            });

            Console::info("═", &"═".repeat(50));
            let pool_id = U256::from(*compute_pool_id as u32);
            let pool_info = match contracts.compute_pool.get_pool_info(pool_id).await {
                Ok(pool) => Arc::new(pool),
                Err(e) => {
                    Console::error(&format!("❌ Failed to get pool info. {}", e));
                    // TODO: Use proper error
                    return Ok(());
                }
            };
            if pool_info.status != PoolStatus::ACTIVE {
                Console::error("❌ Pool is not active.");
                return Ok(());
            }

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

            let mut attempts = 0;
            let max_attempts = 10;
            while attempts < max_attempts {
                let spinner = Console::spinner("Registering provider...");
                if let Err(e) = provider_ops.register_provider().await {
                    spinner.finish_and_clear(); // Finish spinner before logging error
                    if let ProviderError::NotWhitelisted = e {
                        Console::error("❌ Provider not whitelisted, retrying in 15 seconds...");
                        tokio::select! {
                            _ = tokio::time::sleep(tokio::time::Duration::from_secs(15)) => {}
                            _ = cancellation_token.cancelled() => {
                                return Ok(());  // or return an error if you prefer
                            }
                        }
                        attempts += 1;
                        continue; // Retry registration
                    } else {
                        Console::error(&format!("❌ Failed to register provider: {}", e));
                        std::process::exit(1);
                    }
                }
                spinner.finish_and_clear(); // Finish spinner if registration is successful
                break; // Exit loop if registration is successful
            }
            if attempts >= max_attempts {
                Console::error(&format!(
                    "❌ Failed to register provider after {} attempts",
                    attempts
                ));
                std::process::exit(1);
            };

            if let Err(e) = compute_node_ops.add_compute_node().await {
                Console::error(&format!("❌ Failed to add compute node: {}", e));
                std::process::exit(1);
            }

            if let Err(e) = discovery_service.upload_discovery_info(&node_config).await {
                Console::error(&format!("❌ Failed to upload discovery info: {}", e));
                std::process::exit(1);
            }

            Console::success("✅ Discovery info uploaded");

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            Console::info("🌐 Starting endpoint service", "");

            if let Err(err) = {
                let heartbeat_clone = heartbeat_service.unwrap().clone();
                start_server(
                    external_ip,
                    *port,
                    contracts.clone(),
                    node_wallet_instance.clone(),
                    provider_wallet_instance.clone(),
                    heartbeat_clone.clone(),
                    pool_info,
                )
                .await
            } {
                Console::error(&format!("❌ Failed to start server: {}", err));
            }
            Ok(())
        }
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
            Ok(())
        }
    }
}
