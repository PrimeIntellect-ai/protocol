use crate::api::server::start_server;
use crate::checks::hardware::HardwareChecker;
use crate::checks::issue::IssueReport;
use crate::checks::software::software_check;
use crate::console::Console;
use crate::docker::taskbridge::TaskBridge;
use crate::docker::DockerService;
use crate::metrics::store::MetricsStore;
use crate::operations::compute_node::ComputeNodeOperations;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::operations::provider::ProviderOperations;
use crate::services::discovery::DiscoveryService;
use crate::state::system_state::SystemState;
use crate::TaskHandles;
use alloy::primitives::U256;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use clap::{Parser, Subcommand};
use log::{error, info};
use shared::models::node::Node;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
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

        /// Private key for the provider (not recommended, use environment variable PRIVATE_KEY_PROVIDER instead)
        #[arg(long)]
        private_key_provider: Option<String>,

        /// Private key for the node (not recommended, use environment variable PRIVATE_KEY_NODE instead)
        #[arg(long)]
        private_key_node: Option<String>,

        /// Auto accept transactions
        #[arg(long, default_value = "false")]
        auto_accept: bool,

        /// Retry count until provider has enough balance to stake (0 for unlimited retries)
        #[arg(long, default_value = "0")]
        funding_retry_count: u32,

        /// Skip system requirement checks (for development/testing)
        #[arg(long, default_value = "false")]
        skip_system_checks: bool,

        /// Silence metrics logging
        #[arg(long, default_value = "false")]
        silence_metrics: bool,

        /// Loki URL
        #[arg(long)]
        loki_url: Option<String>,

        /// Log level
        #[arg(long)]
        log_level: Option<String>,
    },
    Check {},

    /// Generate new wallets for provider and node
    GenerateWallets {},

    /// Generate new wallet for node only
    GenerateNodeWallet {},

    /// Get balance of provider and node
    Balance {
        /// Private key for the provider
        #[arg(long)]
        private_key: Option<String>,

        /// RPC URL
        #[arg(long, default_value = "http://localhost:8545")]
        rpc_url: String,
    },

    /// Sign Message
    SignMessage {
        /// Message to sign
        #[arg(long)]
        message: String,

        /// Private key for the provider
        #[arg(long)]
        private_key_provider: Option<String>,

        /// Private key for the node
        #[arg(long)]
        private_key_node: Option<String>,
    },
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
            private_key_provider,
            private_key_node,
            auto_accept,
            funding_retry_count,
            skip_system_checks,
            silence_metrics,
            loki_url: _,
            log_level: _,
        } => {
            if *disable_state_storing && *auto_recover {
                Console::user_error(
                    "Cannot disable state storing and enable auto recover at the same time.",
                );
                std::process::exit(1);
            }
            let state = Arc::new(SystemState::new(
                state_dir_overwrite.clone(),
                *disable_state_storing,
            ));

            let private_key_provider = if let Some(key) = private_key_provider {
                Console::warning("Using private key from command line is not recommended. Consider using PRIVATE_KEY_PROVIDER environment variable instead.");
                key.clone()
            } else {
                std::env::var("PRIVATE_KEY_PROVIDER").expect("PRIVATE_KEY_PROVIDER must be set")
            };

            let private_key_node = if let Some(key) = private_key_node {
                Console::warning("Using private key from command line is not recommended. Consider using PRIVATE_KEY_NODE environment variable instead.");
                key.clone()
            } else {
                std::env::var("PRIVATE_KEY_NODE").expect("PRIVATE_KEY_NODE must be set")
            };

            let mut recover_last_state = *auto_recover;
            let version = env!("CARGO_PKG_VERSION");
            Console::section("🚀 PRIME WORKER INITIALIZATION - beta");
            Console::info("Version", version);
            /*
             Initialize Wallet instances
            */
            let provider_wallet_instance = Arc::new(
                match Wallet::new(&private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        error!("Failed to create wallet: {}", err);
                        std::process::exit(1);
                    }
                },
            );

            let node_wallet_instance = Arc::new(
                match Wallet::new(&private_key_node, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        error!("❌ Failed to create wallet: {}", err);
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

            let provider_ops = ProviderOperations::new(
                provider_wallet_instance.clone(),
                contracts.clone(),
                *auto_accept,
            );

            let provider_ops_cancellation = cancellation_token.clone();

            let compute_node_ops = ComputeNodeOperations::new(
                &provider_wallet_instance,
                &node_wallet_instance,
                contracts.clone(),
                state.clone(),
            );

            let discovery_service =
                DiscoveryService::new(&node_wallet_instance, discovery_url.clone(), None);
            let pool_id = U256::from(*compute_pool_id as u32);

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
                        error!("Failed to get pool info: {}", e);
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
            let issue_tracker = Arc::new(RwLock::new(IssueReport::new()));
            let mut hardware_check = HardwareChecker::new(Some(issue_tracker.clone()));
            let node_config = hardware_check.check_hardware(node_config).await.unwrap();
            if let Err(err) = software_check::run_software_check(Some(issue_tracker.clone())).await
            {
                Console::user_error(&format!("❌ Software check failed: {}", err));
                std::process::exit(1);
            }

            let issues = issue_tracker.read().await;
            issues.print_issues();
            if issues.has_critical_issues() {
                if !*skip_system_checks {
                    Console::user_error("❌ Critical issues found. Exiting.");
                    std::process::exit(1);
                } else {
                    Console::warning("Critical issues found. Ignoring and continuing.");
                }
            }

            let has_gpu = match node_config.compute_specs {
                Some(ref specs) => specs.gpu.is_some(),
                None => {
                    Console::warning("Compute specs are not available, assuming no GPU.");
                    false
                }
            };

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
                *silence_metrics,
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
                node_wallet_instance
                    .wallet
                    .default_signer()
                    .address()
                    .to_string(),
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

            let gpu_count: u32 = match &node_config.compute_specs {
                Some(specs) => specs
                    .gpu
                    .as_ref()
                    .map(|gpu| gpu.count.unwrap_or(0))
                    .unwrap_or(0),
                None => 0,
            };
            let compute_units = U256::from(std::cmp::max(1, gpu_count * 1000));

            Console::section("Syncing with Network");

            // Check if provider exists first
            let provider_exists = match provider_ops.check_provider_exists().await {
                Ok(exists) => exists,
                Err(e) => {
                    error!("❌ Failed to check if provider exists: {}", e);
                    std::process::exit(1);
                }
            };

            let stake_manager = match contracts.stake_manager.as_ref() {
                Some(stake_manager) => stake_manager,
                None => {
                    error!("❌ Stake manager not initialized");
                    std::process::exit(1);
                }
            };

            Console::title("Provider Status");
            let is_whitelisted = match provider_ops.check_provider_whitelisted().await {
                Ok(is_whitelisted) => is_whitelisted,
                Err(e) => {
                    error!("Failed to check provider whitelist status: {}", e);
                    std::process::exit(1);
                }
            };

            if provider_exists && is_whitelisted {
                Console::success("Provider is registered and whitelisted");
            } else {
                let required_stake = match stake_manager
                    .calculate_stake(compute_units, U256::from(0))
                    .await
                {
                    Ok(stake) => stake,
                    Err(e) => {
                        error!("❌ Failed to calculate required stake: {}", e);
                        std::process::exit(1);
                    }
                };
                Console::info(
                    "Required stake",
                    &format!("{}", required_stake / U256::from(10u128.pow(18))),
                );

                if let Err(e) = provider_ops
                    .retry_register_provider(
                        required_stake,
                        *funding_retry_count,
                        cancellation_token.clone(),
                    )
                    .await
                {
                    error!("❌ Failed to register provider: {}", e);
                    std::process::exit(1);
                }
            }

            let compute_node_exists = match compute_node_ops.check_compute_node_exists().await {
                Ok(exists) => exists,
                Err(e) => {
                    error!("❌ Failed to check if compute node exists: {}", e);
                    std::process::exit(1);
                }
            };

            let provider_total_compute = match contracts
                .compute_registry
                .get_provider_total_compute(
                    provider_wallet_instance.wallet.default_signer().address(),
                )
                .await
            {
                Ok(compute) => compute,
                Err(e) => {
                    error!("❌ Failed to get provider total compute: {}", e);
                    std::process::exit(1);
                }
            };

            let provider_stake = stake_manager
                .get_stake(provider_wallet_instance.wallet.default_signer().address())
                .await
                .unwrap_or_default();

            // If we are already registered we do not need additionally compute units
            let compute_units = match compute_node_exists {
                true => U256::from(0),
                false => compute_units,
            };

            let required_stake = match stake_manager
                .calculate_stake(compute_units, provider_total_compute)
                .await
            {
                Ok(stake) => stake,
                Err(e) => {
                    error!("❌ Failed to calculate required stake: {}", e);
                    std::process::exit(1);
                }
            };

            if required_stake > provider_stake {
                Console::info(
                    "Provider stake is less than required stake",
                    &format!(
                        "Required: {} tokens, Current: {} tokens",
                        required_stake / U256::from(10u128.pow(18)),
                        provider_stake / U256::from(10u128.pow(18))
                    ),
                );

                match provider_ops
                    .increase_stake(required_stake - provider_stake)
                    .await
                {
                    Ok(_) => {
                        Console::success("Successfully increased stake");
                    }
                    Err(e) => {
                        error!("❌ Failed to increase stake: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            Console::title("Compute Node Status");
            if compute_node_exists {
                // TODO: What if we have two nodes?
                Console::success("Compute node is registered");
                recover_last_state = true;
            } else {
                match compute_node_ops.add_compute_node(compute_units).await {
                    Ok(added_node) => {
                        if added_node {
                            // If we are adding a new compute node we wait for a proper
                            // invite and do not recover from previous state
                            recover_last_state = false;
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to add compute node: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            let mut attempts = 0;
            let max_attempts = 100;
            while attempts < max_attempts {
                match discovery_service.upload_discovery_info(&node_config).await {
                    Ok(_) => break,
                    Err(e) => {
                        attempts += 1;
                        error!(
                            "Attempt {}: ❌ Failed to upload discovery info: {}",
                            attempts, e
                        );
                        if attempts >= max_attempts {
                            std::process::exit(1);
                        }
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            }

            Console::success("Discovery info uploaded");

            Console::section("Starting Worker with Task Bridge");

            // Start monitoring compute node status on chain
            provider_ops.start_monitoring(provider_ops_cancellation);
            if let Err(err) = compute_node_ops.start_monitoring(cancellation_token.clone()) {
                error!("❌ Failed to start node monitoring: {}", err);
                std::process::exit(1);
            }

            // 6. Start HTTP Server to receive challenges and invites to join cluster
            Console::info(
                "🌐 Starting endpoint service and waiting for sync with orchestrator",
                "",
            );

            if let Err(err) = {
                let heartbeat_clone = heartbeat_service.unwrap().clone();
                if recover_last_state {
                    info!("Recovering from previous state: {}", recover_last_state);
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
                )
                .await
            } {
                error!("❌ Failed to start server: {}", err);
            }
            Ok(())
        }
        Commands::Check {} => {
            Console::section("🔍 PRIME WORKER SYSTEM CHECK");
            let issues = Arc::new(RwLock::new(IssueReport::new()));

            // Run hardware checks
            let mut hardware_checker = HardwareChecker::new(Some(issues.clone()));
            let node_config = Node {
                id: String::new(),
                ip_address: String::new(),
                port: 0,
                compute_specs: None,
                provider_address: String::new(),
                compute_pool_id: 0,
            };

            if let Err(err) = hardware_checker.check_hardware(node_config).await {
                Console::user_error(&format!("❌ Hardware check failed: {}", err));
                std::process::exit(1);
            }

            if let Err(err) = software_check::run_software_check(Some(issues.clone())).await {
                Console::user_error(&format!("❌ Software check failed: {}", err));
                std::process::exit(1);
            }

            let issues = issues.read().await;
            issues.print_issues();

            if issues.has_critical_issues() {
                Console::user_error("❌ Critical issues found. Exiting.");
                std::process::exit(1);
            }

            Ok(())
        }
        Commands::GenerateWallets {} => {
            let provider_signer = PrivateKeySigner::random();
            let node_signer = PrivateKeySigner::random();

            let provider_key = hex::encode(provider_signer.credential().to_bytes());
            let node_key = hex::encode(node_signer.credential().to_bytes());

            println!("Provider wallet:");
            println!("  Address: {}", provider_signer.address());
            println!("  Private key: {}", provider_key);
            println!("\nNode wallet:");
            println!("  Address: {}", node_signer.address());
            println!("  Private key: {}", node_key);
            println!("\nTo set environment variables in your current shell session:");
            println!("export PRIVATE_KEY_PROVIDER={}", provider_key);
            println!("export PRIVATE_KEY_NODE={}", node_key);

            Ok(())
        }

        Commands::GenerateNodeWallet {} => {
            let node_signer = PrivateKeySigner::random();
            let node_key = hex::encode(node_signer.credential().to_bytes());

            println!("Node wallet:");
            println!("  Address: {}", node_signer.address());
            println!("  Private key: {}", node_key);
            println!("\nTo set environment variable in your current shell session:");
            println!("export PRIVATE_KEY_NODE={}", node_key);

            Ok(())
        }

        Commands::Balance {
            private_key,
            rpc_url,
        } => {
            let private_key = if let Some(key) = private_key {
                key.clone()
            } else {
                std::env::var("PRIVATE_KEY_PROVIDER").expect("PRIVATE_KEY_PROVIDER must be set")
            };

            let provider_wallet = Wallet::new(&private_key, Url::parse(rpc_url).unwrap()).unwrap();

            let contracts = Arc::new(
                ContractBuilder::new(&provider_wallet)
                    .with_compute_registry()
                    .with_ai_token()
                    .with_prime_network()
                    .with_compute_pool()
                    .build()
                    .unwrap(),
            );

            let provider_balance = contracts
                .ai_token
                .balance_of(provider_wallet.wallet.default_signer().address())
                .await
                .unwrap();

            let format_balance = format!("{}", provider_balance / U256::from(10u128.pow(18)));

            println!("Provider balance: {}", format_balance);
            Ok(())
        }
        Commands::SignMessage {
            message,
            private_key_provider,
            private_key_node,
        } => {
            let private_key_provider = if let Some(key) = private_key_provider {
                key.clone()
            } else {
                std::env::var("PRIVATE_KEY_PROVIDER").expect("PRIVATE_KEY_PROVIDER must be set")
            };

            let private_key_node = if let Some(key) = private_key_node {
                key.clone()
            } else {
                std::env::var("PRIVATE_KEY_NODE").expect("PRIVATE_KEY_NODE must be set")
            };

            let provider_wallet = Wallet::new(
                &private_key_provider,
                Url::parse("http://localhost:8545").unwrap(),
            )
            .unwrap();
            let node_wallet = Wallet::new(
                &private_key_node,
                Url::parse("http://localhost:8545").unwrap(),
            )
            .unwrap();

            let message_hash = provider_wallet.signer.sign_message(message.as_bytes());
            let node_signature = node_wallet.signer.sign_message(message.as_bytes());

            let provider_signature = message_hash.await?;
            let node_signature = node_signature.await?;
            let combined_signature =
                [provider_signature.as_bytes(), node_signature.as_bytes()].concat();

            println!("\nSignature: {}", hex::encode(combined_signature));

            Ok(())
        }
    }
}
