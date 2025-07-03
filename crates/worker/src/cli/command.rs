use crate::checks::hardware::HardwareChecker;
use crate::checks::issue::IssueReport;
use crate::checks::software::SoftwareChecker;
use crate::checks::stun::StunCheck;
use crate::console::Console;
use crate::docker::taskbridge::TaskBridge;
use crate::docker::DockerService;
use crate::metrics::store::MetricsStore;
use crate::operations::compute_node::ComputeNodeOperations;
use crate::operations::heartbeat::service::HeartbeatService;
use crate::operations::provider::ProviderOperations;
use crate::p2p::P2PContext;
use crate::p2p::P2PService;
use crate::services::discovery::DiscoveryService;
use crate::services::discovery_updater::DiscoveryUpdater;
use crate::state::system_state::SystemState;
use crate::TaskHandles;
use alloy::primitives::utils::format_ether;
use alloy::primitives::U256;
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::Signer;
use clap::{Parser, Subcommand};
use log::{error, info};
use shared::models::node::ComputeRequirements;
use shared::models::node::Node;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::Wallet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use url::Url;

const APP_VERSION: &str = match option_env!("WORKER_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(author, version = APP_VERSION, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Run {
        /// RPC URL
        #[arg(long, default_value = option_env!("WORKER_RPC_URL").unwrap_or("http://localhost:8545"))]
        rpc_url: String,

        /// Port number for the worker to listen on - DEPRECATED
        #[arg(long, default_value = "8080")]
        port: u16,

        /// External IP address for the worker to advertise
        #[arg(long)]
        external_ip: Option<String>,

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
        #[arg(long, default_value = "false")]
        no_auto_recover: bool,

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

        /// Loki URL
        #[arg(long)]
        loki_url: Option<String>,

        /// Log level
        #[arg(long)]
        log_level: Option<String>,

        /// Storage path for worker data (overrides automatic selection)
        #[arg(long)]
        storage_path: Option<String>,

        /// Disable host network mode
        #[arg(long, default_value = "false")]
        disable_host_network_mode: bool,
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
        #[arg(long, default_value = option_env!("WORKER_RPC_URL").unwrap_or("http://localhost:8545"))]
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

    /// Deregister worker from compute pool
    Deregister {
        /// Private key for the provider
        #[arg(long)]
        private_key_provider: Option<String>,

        /// Private key for the node
        #[arg(long)]
        private_key_node: Option<String>,

        /// RPC URL
        #[arg(long, default_value = option_env!("WORKER_RPC_URL").unwrap_or("http://localhost:8545"))]
        rpc_url: String,

        /// Compute pool ID
        #[arg(long)]
        compute_pool_id: u64,
    },
}

pub async fn execute_command(
    command: &Commands,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match command {
        Commands::Run {
            port: _,
            external_ip,
            compute_pool_id,
            dry_run: _,
            rpc_url,
            discovery_url,
            state_dir_overwrite,
            disable_state_storing,
            no_auto_recover,
            private_key_provider,
            private_key_node,
            auto_accept,
            funding_retry_count,
            skip_system_checks,
            loki_url: _,
            log_level: _,
            storage_path,
            disable_host_network_mode,
        } => {
            if *disable_state_storing && !(*no_auto_recover) {
                Console::user_error(
                    "Cannot disable state storing and enable auto recover at the same time. Use --no-auto-recover to disable auto recover.",
                );
                std::process::exit(1);
            }
            let state = Arc::new(SystemState::new(
                state_dir_overwrite.clone(),
                *disable_state_storing,
                Some(compute_pool_id.to_string()),
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

            let mut recover_last_state = !(*no_auto_recover);
            let version = APP_VERSION;
            Console::section("🚀 PRIME WORKER INITIALIZATION - beta");
            Console::info("Version", version);

            /*
             Initialize Wallet instances
            */
            let provider_wallet_instance =
                match Wallet::new(&private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        error!("Failed to create wallet: {err}");
                        std::process::exit(1);
                    }
                };

            let node_wallet_instance =
                match Wallet::new(&private_key_node, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        error!("❌ Failed to create wallet: {err}");
                        std::process::exit(1);
                    }
                };

            /*
             Initialize dependencies - services, contracts, operations
            */
            let contracts = ContractBuilder::new(provider_wallet_instance.provider())
                .with_compute_registry()
                .with_ai_token()
                .with_prime_network()
                .with_compute_pool()
                .with_stake_manager()
                .build()
                .unwrap();

            let provider_ops = ProviderOperations::new(
                provider_wallet_instance.clone(),
                contracts.clone(),
                *auto_accept,
            );

            let provider_ops_cancellation = cancellation_token.clone();

            let compute_node_state = state.clone();
            let compute_node_ops = ComputeNodeOperations::new(
                &provider_wallet_instance,
                &node_wallet_instance,
                contracts.clone(),
                compute_node_state,
            );

            let discovery_urls = vec![discovery_url
                .clone()
                .unwrap_or("http://localhost:8089".to_string())];
            let discovery_service =
                DiscoveryService::new(node_wallet_instance.clone(), discovery_urls, None);
            let discovery_state = state.clone();
            let discovery_updater =
                DiscoveryUpdater::new(discovery_service.clone(), discovery_state.clone());
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
                        error!("Failed to get pool info: {e}");
                        return Ok(());
                    }
                }
            };

            let stun_check = StunCheck::new(Duration::from_secs(5), 0);
            let detected_external_ip = match stun_check.get_public_ip().await {
                Ok(ip) => ip,
                Err(e) => {
                    error!("❌ Failed to get public IP: {e}");
                    std::process::exit(1);
                }
            };

            let node_config = Node {
                id: node_wallet_instance
                    .wallet
                    .default_signer()
                    .address()
                    .to_string(),
                ip_address: external_ip.clone().unwrap_or(detected_external_ip.clone()),
                port: 0,
                provider_address: provider_wallet_instance
                    .wallet
                    .default_signer()
                    .address()
                    .to_string(),
                compute_specs: None,
                compute_pool_id: *compute_pool_id as u32,
                worker_p2p_id: None,
                worker_p2p_addresses: None,
            };

            let issue_tracker = Arc::new(RwLock::new(IssueReport::new()));
            let mut hardware_check = HardwareChecker::new(Some(issue_tracker.clone()));
            let mut node_config = match hardware_check
                .check_hardware(node_config, storage_path.clone())
                .await
            {
                Ok(config) => config,
                Err(e) => {
                    Console::user_error(&format!("❌ Hardware check failed: {e}"));
                    std::process::exit(1);
                }
            };
            let software_checker = SoftwareChecker::new(Some(issue_tracker.clone()));
            if let Err(err) = software_checker.check_software(&node_config).await {
                Console::user_error(&format!("❌ Software check failed: {err}"));
                std::process::exit(1);
            }

            if let Some(external_ip) = external_ip {
                if *external_ip != detected_external_ip {
                    Console::warning(
                        &format!(
                            "Automatically detected external IP {detected_external_ip} does not match the provided external IP {external_ip}"
                        ),
                    );
                }
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
            let required_specs = match ComputeRequirements::from_str(&pool_info.pool_data_uri) {
                Ok(specs) => Some(specs),
                Err(e) => {
                    log::debug!("❌ Could not parse pool compute specs: {e}");
                    None
                }
            };

            // Check if node meets the pool's compute requirements
            if let Some(ref compute_specs) = node_config.compute_specs {
                if let Some(ref required_specs) = required_specs {
                    if !compute_specs.meets(required_specs) {
                        Console::user_error(
                            "❌ Your node does not meet the compute requirements for this pool.",
                        );
                        info!("Required compute requirements:\n{required_specs}");
                        if !*skip_system_checks {
                            std::process::exit(1);
                        } else {
                            Console::warning(
                                "Ignoring compute requirements mismatch and continuing.",
                            );
                        }
                    } else {
                        Console::success(
                            "✅ Your node meets the compute requirements for this pool.",
                        );
                    }
                } else {
                    Console::success("✅ No specific compute requirements for this pool.");
                }
            } else {
                Console::warning("Cannot verify compute requirements: node specs not available.");
                if !*skip_system_checks {
                    std::process::exit(1);
                } else {
                    Console::warning("Ignoring missing compute specs and continuing.");
                }
            }

            let metrics_store = Arc::new(MetricsStore::new());
            let heartbeat_metrics_clone = metrics_store.clone();
            let bridge_contracts = contracts.clone();
            let bridge_wallet = node_wallet_instance.clone();

            let docker_storage_path = node_config
                .compute_specs
                .as_ref()
                .expect("Hardware check should have populated compute_specs")
                .storage_path
                .clone();
            let task_bridge = match TaskBridge::new(
                None,
                metrics_store,
                Some(bridge_contracts),
                Some(node_config.clone()),
                Some(bridge_wallet),
                docker_storage_path.clone(),
                state.clone(),
            ) {
                Ok(bridge) => bridge,
                Err(e) => {
                    error!("❌ Failed to create Task Bridge: {e}");
                    std::process::exit(1);
                }
            };

            let system_memory = node_config
                .compute_specs
                .as_ref()
                .map(|specs| specs.ram_mb.unwrap_or(0));

            let gpu = node_config
                .compute_specs
                .clone()
                .and_then(|specs| specs.gpu.clone());
            let docker_service = Arc::new(DockerService::new(
                cancellation_token.clone(),
                gpu,
                system_memory,
                task_bridge
                    .socket_path
                    .to_str()
                    .expect("path is valid utf-8 string")
                    .to_string(),
                docker_storage_path,
                node_wallet_instance
                    .wallet
                    .default_signer()
                    .address()
                    .to_string(),
                state.get_p2p_seed(),
                *disable_host_network_mode,
            ));

            let bridge_cancellation_token = cancellation_token.clone();
            tokio::spawn(async move {
                let bridge_clone = task_bridge.clone();
                tokio::select! {
                    _ = bridge_cancellation_token.cancelled() => {
                    }
                    _ = bridge_clone.run() => {
                    }
                }
            });
            let heartbeat_state = state.clone();
            let heartbeat_service = HeartbeatService::new(
                Duration::from_secs(10),
                cancellation_token.clone(),
                task_handles.clone(),
                node_wallet_instance.clone(),
                docker_service.clone(),
                heartbeat_metrics_clone.clone(),
                heartbeat_state,
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
                    error!("❌ Failed to check if provider exists: {e}");
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
                    error!("Failed to check provider whitelist status: {e}");
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
                        error!("❌ Failed to calculate required stake: {e}");
                        std::process::exit(1);
                    }
                };
                Console::info("Required stake", &format_ether(required_stake).to_string());

                if let Err(e) = provider_ops
                    .retry_register_provider(
                        required_stake,
                        *funding_retry_count,
                        cancellation_token.clone(),
                    )
                    .await
                {
                    error!("❌ Failed to register provider: {e}");
                    std::process::exit(1);
                }
            }

            let compute_node_exists = match compute_node_ops.check_compute_node_exists().await {
                Ok(exists) => exists,
                Err(e) => {
                    error!("❌ Failed to check if compute node exists: {e}");
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
                    error!("❌ Failed to get provider total compute: {e}");
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
                    error!("❌ Failed to calculate required stake: {e}");
                    std::process::exit(1);
                }
            };

            if required_stake > provider_stake {
                Console::info(
                    "Provider stake is less than required stake",
                    &format!(
                        "Required: {} tokens, Current: {} tokens",
                        format_ether(required_stake),
                        format_ether(provider_stake)
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
                        error!("❌ Failed to increase stake: {e}");
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
                        error!("❌ Failed to add compute node: {e}");
                        std::process::exit(1);
                    }
                }
            }

            // Start P2P service
            Console::title("🔗 Starting P2P Service");
            let heartbeat = match heartbeat_service.clone() {
                Ok(service) => service,
                Err(e) => {
                    error!("❌ Heartbeat service is not available: {e}");
                    std::process::exit(1);
                }
            };

            let p2p_context = P2PContext {
                docker_service: docker_service.clone(),
                heartbeat_service: heartbeat.clone(),
                system_state: state.clone(),
                contracts: contracts.clone(),
                node_wallet: node_wallet_instance.clone(),
                provider_wallet: provider_wallet_instance.clone(),
            };

            let validators = match contracts.prime_network.get_validator_role().await {
                Ok(validators) => validators,
                Err(e) => {
                    error!("Failed to get validator role: {e}");
                    std::process::exit(1);
                }
            };

            if validators.is_empty() {
                error!("❌ No validator roles found on contracts - cannot start worker without validators");
                error!("This means the smart contract has no registered validators, which is required for signature validation");
                error!("Please ensure validators are properly registered on the PrimeNetwork contract before starting the worker");
                std::process::exit(1);
            }

            let mut allowed_addresses = vec![pool_info.creator, pool_info.compute_manager_key];
            allowed_addresses.extend(validators);

            let p2p_service = match P2PService::new(
                state.worker_p2p_seed,
                cancellation_token.clone(),
                Some(p2p_context),
                node_wallet_instance.clone(),
                allowed_addresses,
            )
            .await
            {
                Ok(service) => service,
                Err(e) => {
                    error!("❌ Failed to start P2P service: {e}");
                    std::process::exit(1);
                }
            };

            if let Err(e) = p2p_service.start() {
                error!("❌ Failed to start P2P listener: {e}");
                std::process::exit(1);
            }

            node_config.worker_p2p_id = Some(p2p_service.node_id().to_string());
            node_config.worker_p2p_addresses = Some(
                p2p_service
                    .listening_addresses()
                    .iter()
                    .map(|addr| addr.to_string())
                    .collect(),
            );
            Console::success(&format!(
                "P2P service started with ID: {}",
                p2p_service.node_id()
            ));

            let mut attempts = 0;
            let max_attempts = 100;
            while attempts < max_attempts {
                Console::title("📦 Uploading discovery info");
                match discovery_service.upload_discovery_info(&node_config).await {
                    Ok(_) => break,
                    Err(e) => {
                        attempts += 1;
                        let error_msg = e.to_string();

                        // Check if this is a Cloudflare block
                        if error_msg.contains("403 Forbidden")
                            && (error_msg.contains("Cloudflare")
                                || error_msg.contains("Sorry, you have been blocked")
                                || error_msg.contains("Attention Required!"))
                        {
                            error!(
                                "Attempt {attempts}: ❌ Discovery service blocked by Cloudflare protection. This may indicate:"
                            );
                            error!("  • Your IP address has been flagged by Cloudflare security");
                            error!("  • Too many requests from your location");
                            error!("  • Network configuration issues");
                            error!("  • Discovery service may be under DDoS protection");
                            error!(
                                "Please contact support or try from a different network/IP address"
                            );
                        } else {
                            error!("Attempt {attempts}: ❌ Failed to upload discovery info: {e}");
                        }

                        if attempts >= max_attempts {
                            if error_msg.contains("403 Forbidden")
                                && (error_msg.contains("Cloudflare")
                                    || error_msg.contains("Sorry, you have been blocked"))
                            {
                                error!("❌ Unable to reach discovery service due to Cloudflare blocking after {max_attempts} attempts");
                                error!("This is likely a network/IP issue rather than a worker configuration problem");
                            }
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

            let pool_id = state.compute_pool_id.clone().unwrap_or("0".to_string());
            if let Err(err) = compute_node_ops.start_monitoring(cancellation_token.clone(), pool_id)
            {
                error!("❌ Failed to start node monitoring: {err}");
                std::process::exit(1);
            }

            discovery_updater.start_auto_update(node_config);

            if recover_last_state {
                info!("Recovering from previous state: {recover_last_state}");
                heartbeat.activate_heartbeat_if_endpoint_exists().await;
            }

            // Keep the worker running and listening for P2P connections
            Console::success("Worker is now running and listening for P2P connections...");

            // Wait for cancellation signal to gracefully shutdown
            cancellation_token.cancelled().await;

            Console::info(
                "Shutdown signal received",
                "Gracefully shutting down worker...",
            );

            Ok(())
        }
        Commands::Check {} => {
            Console::section("🔍 PRIME WORKER SYSTEM CHECK");
            let issues = Arc::new(RwLock::new(IssueReport::new()));

            // Run checks
            let mut hardware_checker = HardwareChecker::new(Some(issues.clone()));
            let software_checker = SoftwareChecker::new(Some(issues.clone()));
            let node_config = Node {
                id: String::new(),
                ip_address: String::new(),
                port: 0,
                compute_specs: None,
                provider_address: String::new(),
                compute_pool_id: 0,
                worker_p2p_id: None,
                worker_p2p_addresses: None,
            };

            let node_config = match hardware_checker.check_hardware(node_config, None).await {
                Ok(node_config) => node_config,
                Err(err) => {
                    Console::user_error(&format!("❌ Hardware check failed: {err}"));
                    std::process::exit(1);
                }
            };

            if let Err(err) = software_checker.check_software(&node_config).await {
                Console::user_error(&format!("❌ Software check failed: {err}"));
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
            println!("  Private key: {provider_key}");
            println!("\nNode wallet:");
            println!("  Address: {}", node_signer.address());
            println!("  Private key: {node_key}");
            println!("\nTo set environment variables in your current shell session:");
            println!("export PRIVATE_KEY_PROVIDER={provider_key}");
            println!("export PRIVATE_KEY_NODE={node_key}");

            Ok(())
        }

        Commands::GenerateNodeWallet {} => {
            let node_signer = PrivateKeySigner::random();
            let node_key = hex::encode(node_signer.credential().to_bytes());

            println!("Node wallet:");
            println!("  Address: {}", node_signer.address());
            println!("  Private key: {node_key}");
            println!("\nTo set environment variable in your current shell session:");
            println!("export PRIVATE_KEY_NODE={node_key}");

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

            let contracts = ContractBuilder::new(provider_wallet.provider())
                .with_compute_registry()
                .with_ai_token()
                .with_prime_network()
                .with_compute_pool()
                .build()
                .unwrap();

            let provider_balance = contracts
                .ai_token
                .balance_of(provider_wallet.wallet.default_signer().address())
                .await
                .unwrap();

            let format_balance = format_ether(provider_balance).to_string();

            println!("Provider balance: {format_balance}");
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
        Commands::Deregister {
            private_key_provider,
            private_key_node,
            rpc_url,
            compute_pool_id,
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

            let provider_wallet_instance =
                match Wallet::new(&private_key_provider, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        Console::user_error(&format!("Failed to create wallet: {err}"));
                        std::process::exit(1);
                    }
                };

            let node_wallet_instance =
                match Wallet::new(&private_key_node, Url::parse(rpc_url).unwrap()) {
                    Ok(wallet) => wallet,
                    Err(err) => {
                        Console::user_error(&format!("❌ Failed to create wallet: {err}"));
                        std::process::exit(1);
                    }
                };
            let state = Arc::new(SystemState::new(None, true, None));
            /*
             Initialize dependencies - services, contracts, operations
            */

            let contracts = ContractBuilder::new(provider_wallet_instance.provider())
                .with_compute_registry()
                .with_ai_token()
                .with_prime_network()
                .with_compute_pool()
                .with_stake_manager()
                .build()
                .unwrap();

            let compute_node_ops = ComputeNodeOperations::new(
                &provider_wallet_instance,
                &node_wallet_instance,
                contracts.clone(),
                state.clone(),
            );

            let provider_ops =
                ProviderOperations::new(provider_wallet_instance.clone(), contracts.clone(), false);

            let compute_node_exists = match compute_node_ops.check_compute_node_exists().await {
                Ok(exists) => exists,
                Err(e) => {
                    Console::user_error(&format!("❌ Failed to check if compute node exists: {e}"));
                    std::process::exit(1);
                }
            };

            let pool_id = U256::from(*compute_pool_id as u32);

            if compute_node_exists {
                match contracts
                    .compute_pool
                    .leave_compute_pool(
                        pool_id,
                        provider_wallet_instance.wallet.default_signer().address(),
                        node_wallet_instance.wallet.default_signer().address(),
                    )
                    .await
                {
                    Ok(result) => {
                        Console::success(&format!("Leave compute pool tx: {result:?}"));
                    }
                    Err(e) => {
                        Console::user_error(&format!("❌ Failed to leave compute pool: {e}"));
                        std::process::exit(1);
                    }
                }
                match compute_node_ops.remove_compute_node().await {
                    Ok(_removed_node) => {
                        Console::success("Compute node removed");
                        match provider_ops.reclaim_stake(U256::from(0)).await {
                            Ok(_) => {
                                Console::success("Successfully reclaimed stake");
                            }
                            Err(e) => {
                                Console::user_error(&format!("❌ Failed to reclaim stake: {e}"));
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        Console::user_error(&format!("❌ Failed to remove compute node: {e}"));
                        std::process::exit(1);
                    }
                }
            } else {
                Console::success("Compute node is not registered");
            }

            Ok(())
        }
    }
}
