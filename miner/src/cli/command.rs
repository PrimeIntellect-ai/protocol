use crate::api::server::start_server;
use crate::checks::hardware::HardwareChecker;
use crate::checks::software::software_check;
use crate::console::Console;
use crate::docker::taskbridge::TaskBridge;
use crate::docker::DockerService;
use crate::metrics::store::MetricsStore;
use crate::operations::compute_node::ComputeNodeOperations;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::operations::provider::ProviderError;
use crate::operations::provider::ProviderOperations;
use crate::services::discovery::DiscoveryService;
use crate::TaskHandles;
use alloy::primitives::U256;
use clap::{Parser, Subcommand};
use log::debug;
use shared::models::node::Node;
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

        /// Optional state storage directory overwrite
        #[arg(long)]
        state_dir_overwrite: Option<String>,

        /// Disable state storing
        #[arg(long, default_value = "false")]
        disable_state_storing: bool,

        /// Auto recover from previous state
        #[arg(long, default_value = "true")]
        auto_recover: bool,

        /// Discovery service URL
        #[arg(long)]
        discovery_url: Option<String>,

        // Amount of stake to use when provider is newly registered
        #[arg(long, default_value = "10")]
        provider_stake: i32,
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
            provider_stake,
            discovery_url,
            state_dir_overwrite,
            disable_state_storing,
            auto_recover,
        } => {
            if *disable_state_storing && *auto_recover {
                Console::error(
                    "❌ Cannot disable state storing and enable auto recover at the same time.",
                );
                std::process::exit(1);
            }

            let mut recover_last_state = *auto_recover;
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

            let discovery_service =
                DiscoveryService::new(&node_wallet_instance, discovery_url.clone(), None);
            let pool_id = U256::from(*compute_pool_id as u32);

            let pool_info = loop {
                match contracts.compute_pool.get_pool_info(pool_id).await {
                    Ok(pool) if pool.status == PoolStatus::ACTIVE => break Arc::new(pool),
                    Ok(_) => {
                        Console::error("❌ Pool is not active yet. Checking again in 15 seconds.");
                        tokio::select! {
                            _ = tokio::time::sleep(tokio::time::Duration::from_secs(15)) => {},
                            _ = cancellation_token.cancelled() => return Ok(()),
                        }
                    }
                    Err(e) => {
                        Console::error(&format!("❌ Failed to get pool info. {}", e));
                        return Ok(());
                    }
                }
            };

            let node_config = Node {
                id: node_wallet_instance
                    .wallet
                    .default_signer()
                    .address()
                    .to_string(),
                ip_address: external_ip.to_string(),
                port: *port,
                provider_address: provider_wallet_instance
                    .wallet
                    .default_signer()
                    .address()
                    .to_string(),
                compute_specs: None,
                compute_pool_id: *compute_pool_id as u32,
            };
            let hardware_check = HardwareChecker::new();
            let node_config = hardware_check.enrich_node_config(node_config).unwrap();

            // TODO: Move to proper check
            let _ = software_check::run_software_check();
            let has_gpu = match node_config.compute_specs {
                Some(ref specs) => specs.gpu.is_some(),
                None => {
                    Console::warning("Compute specs are not available, assuming no GPU.");
                    false
                }
            };

            let metrics_store = Arc::new(MetricsStore::new());
            let heartbeat_metrics_clone = metrics_store.clone();
            let task_bridge = Arc::new(TaskBridge::new(None, metrics_store));

            let docker_storage_path = match node_config.clone().compute_specs {
                Some(specs) => specs.storage_path.clone(),
                None => None,
            };

            let docker_service = Arc::new(DockerService::new(
                cancellation_token.clone(),
                has_gpu,
                task_bridge.socket_path.clone(),
                docker_storage_path,
            ));

            let bridge_cancellation_token = cancellation_token.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = bridge_cancellation_token.cancelled() => {
                    }
                    _ = task_bridge.run() => {
                    }
                }
            });
            let heartbeat_service = HeartbeatService::new(
                Duration::from_secs(10),
                state_dir_overwrite.clone(),
                *disable_state_storing,
                cancellation_token.clone(),
                task_handles.clone(),
                node_wallet_instance.clone(),
                docker_service.clone(),
                heartbeat_metrics_clone.clone(),
            );

            let mut attempts = 0;
            let max_attempts = 100;
            let stake = U256::from(*provider_stake);
            while attempts < max_attempts {
                let spinner = Console::spinner("Registering provider...");
                if let Err(e) = provider_ops.register_provider(stake).await {
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

            match compute_node_ops.add_compute_node().await {
                Ok(added_node) => {
                    if added_node {
                        // If we are adding a new compute node we wait for a proper
                        // invite and do not recover from previous state
                        recover_last_state = false;
                    }
                }
                Err(e) => {
                    Console::error(&format!("❌ Failed to add compute node: {}", e));
                    std::process::exit(1);
                }
            }

            if let Err(e) = discovery_service.upload_discovery_info(&node_config).await {
                Console::error(&format!("❌ Failed to upload discovery info: {}", e));
                std::process::exit(1);
            }

            Console::success("✅ Discovery info uploaded");

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            Console::info(
                "🌐 Starting endpoint service and waiting for sync with orchestrator",
                "",
            );

            if let Err(err) = {
                let heartbeat_clone = heartbeat_service.unwrap().clone();
                debug!("Recovering from previous state: {}", recover_last_state);
                if recover_last_state {
                    heartbeat_clone
                        .activate_heartbeat_if_endpoint_exists()
                        .await;
                }

                start_server(
                    "0.0.0.0",
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
            let node_config = Node {
                id: String::new(),
                ip_address: String::new(),
                port: 0,
                compute_specs: None,
                provider_address: String::new(),
                compute_pool_id: 0,
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
