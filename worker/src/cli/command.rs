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
use crate::state::system_state::SystemState;
use crate::TaskHandles;
use alloy::primitives::U256;
use alloy::signers::local::PrivateKeySigner;
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
        /// RPC URL
        #[arg(long, default_value = "http://localhost:8545")]
        rpc_url: String,

        /// Port number for the worker to listen on
        #[arg(long, default_value = "8080")]
        port: u16,

        /// External IP address for the worker to advertise
        #[arg(long)]
        external_ip: String,

        /// Compute pool ID
        #[arg(long)]
        compute_pool_id: u64,

        /// Dry run the command without starting the worker
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

        #[arg(long, default_value = "0x0000000000000000000000000000000000000000")]
        validator_address: Option<String>,
    },
    Check {},

    /// Generate new wallets for provider and node
    GenerateWallets {},
}

pub async fn execute_command(
    command: &Commands,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match command {
        Commands::Run {
            port,
            external_ip,
            compute_pool_id,
            dry_run: _,
            rpc_url,
            discovery_url,
            state_dir_overwrite,
            disable_state_storing,
            auto_recover,
            validator_address,
        } => {
            if *disable_state_storing && *auto_recover {
                Console::error(
                    "❌ Cannot disable state storing and enable auto recover at the same time.",
                );
                std::process::exit(1);
            }

            let private_key_provider =
                std::env::var("PRIVATE_KEY_PROVIDER").expect("PRIVATE_KEY_PROVIDER must be set");
            let private_key_node =
                std::env::var("PRIVATE_KEY_NODE").expect("PRIVATE_KEY_NODE must be set");

            let mut recover_last_state = *auto_recover;
            let version = env!("CARGO_PKG_VERSION");
            Console::section("🚀 PRIME MINER INITIALIZATION");
            Console::info("Version:", version);
            /*
             Initialize Wallet instances
            */
            let provider_wallet_instance = Arc::new(
                match Wallet::new(&private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        Console::error(&format!("❌ Failed to create wallet: {}", err));
                        std::process::exit(1);
                    }
                },
            );

            let node_wallet_instance = Arc::new(
                match Wallet::new(&private_key_node, Url::parse(rpc_url).unwrap()) {
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
                    .with_stake_manager()
                    .build()
                    .unwrap(),
            );

            let provider_ops =
                ProviderOperations::new(provider_wallet_instance.clone(), contracts.clone());

            let provider_ops_cancellation = cancellation_token.clone();

            let compute_node_ops = ComputeNodeOperations::new(
                &provider_wallet_instance,
                &node_wallet_instance,
                contracts.clone(),
            );

            let discovery_service =
                DiscoveryService::new(&node_wallet_instance, discovery_url.clone(), None);
            let pool_id = U256::from(*compute_pool_id as u32);

            Console::progress("Loading pool info");
            println!("Loading pool info {}", pool_id);
            let pool_info = loop {
                match contracts.compute_pool.get_pool_info(pool_id).await {
                    Ok(pool) if pool.status == PoolStatus::ACTIVE => break Arc::new(pool),
                    Ok(_) => {
                        Console::warning("Pool is not active yet. Checking again in 15 seconds.");
                        tokio::select! {
                            _ = tokio::time::sleep(tokio::time::Duration::from_secs(15)) => {},
                            _ = cancellation_token.cancelled() => return Ok(()),
                        }
                    }
                    Err(e) => {
                        Console::error(&format!("Failed to get pool info: {}", e));
                        return Ok(());
                    }
                }
            };
            println!("Pool info: {:?}", pool_info);

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

            let state = Arc::new(SystemState::new(
                state_dir_overwrite.clone(),
                *disable_state_storing,
            ));
            let metrics_store = Arc::new(MetricsStore::new());
            let heartbeat_metrics_clone = metrics_store.clone();
            let bridge_contracts = contracts.clone();
            let bridge_wallet = node_wallet_instance.clone();

            let docker_storage_path = match node_config.clone().compute_specs {
                Some(specs) => specs.storage_path.clone(),
                None => None,
            };
            let task_bridge = Arc::new(TaskBridge::new(
                None,
                metrics_store,
                Some(bridge_contracts),
                Some(node_config.clone()),
                Some(bridge_wallet),
                docker_storage_path.clone(),
                state.clone(),
            ));

            let system_memory = node_config
                .compute_specs
                .as_ref()
                .map(|specs| specs.ram_mb.unwrap_or(0));

            let docker_service = Arc::new(DockerService::new(
                cancellation_token.clone(),
                has_gpu,
                system_memory,
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
                cancellation_token.clone(),
                task_handles.clone(),
                node_wallet_instance.clone(),
                docker_service.clone(),
                heartbeat_metrics_clone.clone(),
                state,
            );

            let mut attempts = 0;
            let max_attempts = 100;
            let gpu_count: u32 = match &node_config.compute_specs {
                Some(specs) => specs
                    .gpu
                    .as_ref()
                    .map(|gpu| gpu.count.unwrap_or(0))
                    .unwrap_or(0),
                None => 0,
            };

            let compute_units = U256::from(gpu_count * 1000);

            let provider_total_compute = match contracts
                .compute_registry
                .get_provider_total_compute(
                    provider_wallet_instance.wallet.default_signer().address(),
                )
                .await
            {
                Ok(compute) => compute,
                Err(e) => {
                    Console::error(&format!("❌ Failed to get provider total compute: {}", e));
                    std::process::exit(1);
                }
            };
            let stake_manager = match contracts.stake_manager.as_ref() {
                Some(stake_manager) => stake_manager,
                None => {
                    Console::error("❌ Stake manager not initialized");
                    std::process::exit(1);
                }
            };

            let required_stake = match stake_manager
                .calculate_stake(compute_units, provider_total_compute)
                .await
            {
                Ok(stake) => stake,
                Err(e) => {
                    Console::error(&format!("❌ Failed to calculate required stake: {}", e));
                    std::process::exit(1);
                }
            };
            Console::info(
                "Required stake",
                &format!("{}", required_stake / U256::from(10u128.pow(18))),
            );

            // TODO: Currently we do not increase stake when adding more nodes

            while attempts < max_attempts {
                let spinner = Console::spinner("Registering provider...");
                if let Err(e) = provider_ops.register_provider(required_stake).await {
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

            provider_ops.start_monitoring(provider_ops_cancellation);

            let provider_stake = match stake_manager
                .get_stake(provider_wallet_instance.wallet.default_signer().address())
                .await
            {
                Ok(stake) => stake,
                Err(e) => {
                    Console::error(&format!("❌ Failed to get provider stake: {}", e));
                    std::process::exit(1);
                }
            };
            Console::info(
                "Provider stake",
                &format!("{}", provider_stake / U256::from(10u128.pow(18))),
            );

            if provider_stake < required_stake {
                let spinner = Console::spinner("Increasing stake...");
                if let Err(e) = provider_ops
                    .increase_stake(required_stake - provider_stake)
                    .await
                {
                    spinner.finish_and_clear();
                    Console::error(&format!("❌ Failed to increase stake: {}", e));
                    std::process::exit(1);
                }
                spinner.finish_and_clear();
            }

            match compute_node_ops.add_compute_node(compute_units).await {
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

            // Start monitoring compute node status on chain
            compute_node_ops.start_monitoring(cancellation_token.clone());

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
                    docker_service.clone(),
                    pool_info,
                    validator_address.clone().unwrap_or_default(),
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
        Commands::GenerateWallets {} => {
            let provider_signer = PrivateKeySigner::random();
            let node_signer = PrivateKeySigner::random();

            println!("Provider wallet:");
            println!("  Address: {}", provider_signer.address());
            println!(
                "  Private key: {}",
                hex::encode(provider_signer.credential().to_bytes())
            );
            println!("\nNode wallet:");
            println!("  Address: {}", node_signer.address());
            println!(
                "  Private key: {}",
                hex::encode(node_signer.credential().to_bytes())
            );
            Ok(())
        }
    }
}
