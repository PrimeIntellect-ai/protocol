mod api;
mod discovery;
mod events;
mod metrics;
mod models;
mod node;
mod plugins;
mod scheduler;
mod status_update;
mod store;
mod utils;
use crate::api::server::start_server;
use crate::discovery::monitor::DiscoveryMonitor;
use crate::node::invite::NodeInviter;
use crate::scheduler::Scheduler;
use crate::status_update::NodeStatusUpdater;
use crate::store::core::RedisStore;
use crate::store::core::StoreContext;
use crate::utils::loop_heartbeats::LoopHeartbeats;
use alloy::primitives::U256;
use anyhow::Result;
use clap::Parser;
use clap::ValueEnum;
use log::debug;
use log::error;
use log::info;
use log::LevelFilter;
use metrics::webhook_sender::MetricsWebhookSender;
use metrics::MetricsContext;
use plugins::node_groups::NodeGroupConfiguration;
use plugins::node_groups::NodeGroupsPlugin;
use plugins::webhook::WebhookConfig;
use plugins::webhook::WebhookPlugin;
use plugins::SchedulerPlugin;
use plugins::StatusUpdatePlugin;
use shared::utils::google_cloud::GcsStorageProvider;
use shared::web3::contracts::core::builder::ContractBuilder;
use shared::web3::contracts::structs::compute_pool::PoolStatus;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use url::Url;

#[derive(Parser, Clone, Copy, ValueEnum, Debug, PartialEq)]
pub enum ServerMode {
    ApiOnly,
    ProcessorOnly,
    Full,
}

#[derive(Parser)]
struct Args {
    // Server mode
    #[arg(long, default_value = "full")]
    mode: String,

    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    coordinator_key: String,

    /// Compute pool id
    #[arg(long, default_value = "0")]
    compute_pool_id: u32,

    /// Domain id
    #[arg(short = 'd', long, default_value = "0")]
    domain_id: u32,

    /// External ip - advertised to workers
    #[arg(short = 'e', long)]
    host: Option<String>,

    /// Port
    #[arg(short = 'p', long, default_value = "8090")]
    port: u16,

    /// External url - advertised to workers
    #[arg(short = 'u', long)]
    url: Option<String>,

    /// Discovery refresh interval
    #[arg(short = 'i', long, default_value = "10")]
    discovery_refresh_interval: u64,

    /// Redis store url
    #[arg(short = 's', long, default_value = "redis://localhost:6380")]
    redis_store_url: String,

    /// Discovery url
    #[arg(long, default_value = "http://localhost:8089")]
    discovery_url: String,

    /// Admin api key
    #[arg(short = 'a', long, default_value = "admin")]
    admin_api_key: String,

    /// Disable instance ejection from chain
    #[arg(long)]
    disable_ejection: bool,

    /// Hourly s3 upload limit
    #[arg(long, default_value = "2")]
    hourly_s3_upload_limit: i64,

    /// S3 bucket name
    #[arg(long)]
    bucket_name: Option<String>,

    /// Log level
    #[arg(short = 'l', long, default_value = "info")]
    log_level: String,

    /// Node group management interval
    #[arg(long, default_value = "10")]
    node_group_management_interval: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let log_level = match args.log_level.as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Info, // Default to Info if the level is unrecognized
    };
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp(None)
        .init();

    let server_mode = match args.mode.as_str() {
        "full" => ServerMode::Full,
        "api" => ServerMode::ApiOnly,
        "processor" => ServerMode::ProcessorOnly,
        _ => ServerMode::Full,
    };

    debug!("Log level: {}", log_level);
    debug!("Server mode: {:?}", server_mode);

    let heartbeats = Arc::new(LoopHeartbeats::new(&server_mode));

    let compute_pool_id = args.compute_pool_id;
    let domain_id = args.domain_id;
    let coordinator_key = args.coordinator_key;
    let rpc_url: Url = args.rpc_url.parse().unwrap();

    let mut tasks: JoinSet<Result<()>> = JoinSet::new();

    let coordinator_wallet = Arc::new(Wallet::new(&coordinator_key, rpc_url).unwrap_or_else(
        |err| {
            error!("Error creating wallet: {:?}", err);
            std::process::exit(1);
        },
    ));

    let store = Arc::new(RedisStore::new(&args.redis_store_url));
    let store_context = Arc::new(StoreContext::new(store.clone()));
    let wallet_clone = coordinator_wallet.clone();
    let server_wallet = coordinator_wallet.clone();

    let metrics_store_context = store_context.clone();
    let metrics_context = Arc::new(MetricsContext::new(
        args.compute_pool_id.to_string(),
        metrics_store_context,
    ));

    let metrics_loop_context = metrics_context.clone();
    tasks.spawn(async move {
        loop {
            if let Err(e) = metrics_loop_context.fill_metrics_interval().await {
                error!("Error filling metrics interval: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    let contracts = Arc::new(
        ContractBuilder::new(&coordinator_wallet.clone())
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .build()
            .unwrap(),
    );

    match contracts
        .compute_pool
        .get_pool_info(U256::from(compute_pool_id))
        .await
    {
        Ok(pool) if pool.status == PoolStatus::ACTIVE => Arc::new(pool),
        Ok(_) => {
            info!("Pool is not active. Exiting.");
            return Ok(());
        }
        Err(e) => {
            error!("Failed to get pool info: {}", e);
            return Ok(());
        }
    };
    let group_store_context = store_context.clone();
    let mut scheduler_plugins: Vec<Box<dyn SchedulerPlugin>> = Vec::new();
    let mut status_update_plugins: Vec<Box<dyn StatusUpdatePlugin>> = vec![];
    let mut node_groups_plugin: Option<Arc<NodeGroupsPlugin>> = None;
    let mut webhook_plugins: Vec<WebhookPlugin> = vec![];

    let configs = std::env::var("WEBHOOK_CONFIGS").unwrap_or_default();
    if !configs.is_empty() {
        match serde_json::from_str::<Vec<WebhookConfig>>(&configs) {
            Ok(configs) => {
                for config in configs {
                    let plugin = WebhookPlugin::new(config);
                    let plugin_clone = plugin.clone();
                    webhook_plugins.push(plugin_clone);
                    status_update_plugins.push(Box::new(plugin));
                    info!("Plugin: Webhook plugin initialized");
                }
            }
            Err(e) => {
                error!("Failed to parse webhook configs from environment: {}", e);
            }
        }
    } else {
        info!("No webhook configurations provided");
    }

    let webhook_sender_store = store_context.clone();
    let webhook_plugins_clone = webhook_plugins.clone();
    if !webhook_plugins_clone.is_empty() && server_mode != ServerMode::ApiOnly {
        tasks.spawn(async move {
            let mut webhook_sender = MetricsWebhookSender::new(
                webhook_sender_store.clone(),
                webhook_plugins_clone.clone(),
                compute_pool_id,
            );
            if let Err(e) = webhook_sender.run().await {
                error!("Error running webhook sender: {}", e);
            }
            Ok(())
        });
    }

    // Load node group configurations from environment variable
    let node_group_configs = std::env::var("NODE_GROUP_CONFIGS").unwrap_or_default();
    if !node_group_configs.is_empty() {
        match serde_json::from_str::<Vec<NodeGroupConfiguration>>(&node_group_configs) {
            Ok(configs) if !configs.is_empty() => {
                let node_groups_heartbeats = heartbeats.clone();
                let group_plugin = NodeGroupsPlugin::new(
                    configs,
                    store.clone(),
                    group_store_context.clone(),
                    Some(node_groups_heartbeats.clone()),
                    Some(webhook_plugins.clone()),
                );
                let status_group_plugin = group_plugin.clone();
                let group_plugin_for_server = group_plugin.clone();
                node_groups_plugin = Some(Arc::new(group_plugin_for_server));
                scheduler_plugins.push(Box::new(group_plugin));
                status_update_plugins.push(Box::new(status_group_plugin));
                info!("Plugin: Node group plugin initialized");
            }
            Ok(_) => {
                info!(
                    "No node group configurations provided in environment, skipping plugin setup"
                );
            }
            Err(e) => {
                error!(
                    "Failed to parse node group configurations from environment: {}",
                    e
                );
                std::process::exit(1);
            }
        }
    }

    let scheduler = Scheduler::new(store_context.clone(), scheduler_plugins);

    // Only spawn processor tasks if in ProcessorOnly or Full mode
    if matches!(server_mode, ServerMode::ProcessorOnly | ServerMode::Full) {
        if let Some(group_plugin) = node_groups_plugin.clone() {
            tasks.spawn(async move {
                group_plugin
                    .run_group_management_loop(args.node_group_management_interval)
                    .await
            });
        }

        let discovery_store_context = store_context.clone();
        let discovery_heartbeats = heartbeats.clone();
        tasks.spawn(async move {
            let monitor = DiscoveryMonitor::new(
                wallet_clone.as_ref(),
                compute_pool_id,
                args.discovery_refresh_interval,
                args.discovery_url,
                discovery_store_context.clone(),
                discovery_heartbeats.clone(),
            );
            monitor.run().await
        });

        let inviter_store_context = store_context.clone();
        let inviter_heartbeats = heartbeats.clone();
        tasks.spawn(async move {
            let inviter = NodeInviter::new(
                coordinator_wallet.as_ref(),
                compute_pool_id,
                domain_id,
                args.host.as_deref(),
                Some(&args.port),
                args.url.as_deref(),
                inviter_store_context.clone(),
                inviter_heartbeats.clone(),
            );
            inviter.run().await
        });

        let status_update_store_context = store_context.clone();
        let status_update_heartbeats = heartbeats.clone();
        let status_update_contracts = contracts.clone();
        let status_update_metrics = metrics_context.clone();

        tasks.spawn(async move {
            let status_updater = NodeStatusUpdater::new(
                status_update_store_context.clone(),
                15,
                None,
                status_update_contracts.clone(),
                compute_pool_id,
                args.disable_ejection,
                status_update_heartbeats.clone(),
                status_update_plugins,
                status_update_metrics,
            );
            status_updater.run().await
        });
    }

    let port = args.port;
    let server_store_context = store_context.clone();

    let s3_credentials = std::env::var("S3_CREDENTIALS").ok();

    let gcs_storage = GcsStorageProvider::new(&args.bucket_name.unwrap(), &s3_credentials.unwrap())
        .await
        .unwrap();
    let storage_provider = Arc::new(gcs_storage);

    // Always start server regardless of mode
    tokio::select! {
        res = start_server(
            "0.0.0.0",
            port,
            server_store_context.clone(),
            server_wallet,
            args.admin_api_key,
            storage_provider,
            heartbeats.clone(),
            store.clone(),
            args.hourly_s3_upload_limit,
            Some(contracts.clone()),
            compute_pool_id,
            server_mode,
            scheduler,
            node_groups_plugin,
            metrics_context,
        ) => {
            if let Err(e) = res {
                error!("Server error: {}", e);
            }
        }
        Some(res) = tasks.join_next() => {
            if let Err(e) = res? {
                error!("Task error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            error!("Shutdown signal received");
        }
    }

    tasks.shutdown().await;
    Ok(())
}
