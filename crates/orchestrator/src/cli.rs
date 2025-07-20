use std::sync::Arc;

use alloy::providers::Provider;
use anyhow::Result;
use clap::Parser;
use futures::FutureExt;
use log::{debug, error, info};
use shared::{
    utils::google_cloud::GcsStorageProvider,
    web3::{contracts::core::builder::ContractBuilder, wallet::Wallet},
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::{
    start_server, DiscoveryMonitor, LoopHeartbeats, MetricsContext, MetricsSyncService,
    MetricsWebhookSender, NodeGroupConfiguration, NodeGroupsPlugin, NodeInviter, NodeStatusUpdater,
    P2PService, RedisStore, Scheduler, SchedulerPlugin, ServerMode, StatusUpdatePlugin,
    StoreContext, WebhookConfig, WebhookPlugin,
};

#[derive(Parser)]
pub struct Cli {
    // Server mode
    #[arg(long, default_value = "full")]
    // TODO: directly parse into `ServerMode`
    pub mode: String,

    /// RPC URL
    #[arg(short = 'r', long, default_value = "http://localhost:8545")]
    pub rpc_url: String,

    /// Owner key
    #[arg(short = 'k', long)]
    pub coordinator_key: String,

    /// Compute pool id
    #[arg(long, default_value = "0")]
    pub compute_pool_id: u32,

    /// Domain id
    #[arg(short = 'd', long, default_value = "0")]
    pub domain_id: u32,

    /// External ip - advertised to workers
    #[arg(short = 'e', long)]
    pub host: Option<String>,

    /// Port
    #[arg(short = 'p', long, default_value = "8090")]
    pub port: u16,

    /// External url - advertised to workers
    #[arg(short = 'u', long)]
    pub url: Option<String>,

    /// Discovery refresh interval
    #[arg(short = 'i', long, default_value = "10")]
    pub discovery_refresh_interval: u64,

    /// Redis store url
    #[arg(short = 's', long, default_value = "redis://localhost:6380")]
    pub redis_store_url: String,

    /// Admin api key
    #[arg(short = 'a', long, default_value = "admin")]
    pub admin_api_key: String,

    /// Disable instance ejection from chain
    #[arg(long)]
    pub disable_ejection: bool,

    /// Hourly s3 upload limit
    #[arg(long, default_value = "2")]
    pub hourly_s3_upload_limit: i64,

    /// S3 bucket name
    #[arg(long)]
    pub bucket_name: Option<String>,

    /// Log level
    #[arg(short = 'l', long, default_value = "info")]
    pub log_level: String,

    /// Node group management interval
    #[arg(long, default_value = "10")]
    pub node_group_management_interval: u64,

    /// Max healthy nodes with same endpoint
    #[arg(long, default_value = "1")]
    pub max_healthy_nodes_with_same_endpoint: u32,

    /// Libp2p port
    #[arg(long, default_value = "4004")]
    pub libp2p_port: u16,

    /// Comma-separated list of libp2p bootnode multiaddresses
    /// Example: `/ip4/104.131.131.82/tcp/4001/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ,/ip4/104.131.131.82/udp/4001/quic-v1/p2p/QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ`
    #[arg(long, default_value = "")]
    pub bootnodes: String,

    /// Location service URL (e.g., https://ipapi.co). If not provided, location services are disabled.
    #[arg(long)]
    pub location_service_url: Option<String>,

    /// Location service API key
    #[arg(long)]
    pub location_service_api_key: Option<String>,
}

impl Cli {
    pub async fn run(self, cancellation_token: CancellationToken) -> anyhow::Result<()> {
        let server_mode = match self.mode.as_str() {
            "api" => ServerMode::ApiOnly,
            "processor" => ServerMode::ProcessorOnly,
            "full" => ServerMode::Full,
            _ => anyhow::bail!("invalid server mode: {}", self.mode),
        };

        debug!("Server mode: {server_mode:?}");

        let metrics_context = Arc::new(MetricsContext::new(self.compute_pool_id.to_string()));

        let heartbeats = Arc::new(LoopHeartbeats::new(&server_mode));

        let compute_pool_id = self.compute_pool_id;
        let domain_id = self.domain_id;
        let coordinator_key = self.coordinator_key;
        let rpc_url: Url = self.rpc_url.parse().unwrap();

        let mut tasks: JoinSet<Result<()>> = JoinSet::new();

        let wallet = Wallet::new(&coordinator_key, rpc_url).unwrap_or_else(|err| {
            error!("Error creating wallet: {err:?}");
            std::process::exit(1);
        });

        let store = Arc::new(RedisStore::new(&self.redis_store_url));
        let store_context = Arc::new(StoreContext::new(store.clone()));

        let keypair = p2p::Keypair::generate_ed25519();
        let bootnodes: Vec<p2p::Multiaddr> = self
            .bootnodes
            .split(',')
            .filter_map(|addr| match addr.to_string().try_into() {
                Ok(multiaddr) => Some(multiaddr),
                Err(e) => {
                    error!("Invalid bootnode address '{addr}': {e}");
                    None
                }
            })
            .collect();
        if bootnodes.is_empty() {
            error!(
                "No valid bootnodes provided. Please provide at least one valid bootnode address."
            );
            std::process::exit(1);
        }

        let (p2p_service, invite_tx, get_task_logs_tx, restart_task_tx, kademlia_action_tx) = {
            match P2PService::new(
                keypair,
                self.libp2p_port,
                bootnodes,
                cancellation_token.clone(),
                wallet.clone(),
            ) {
                Ok(res) => {
                    info!("p2p service initialized successfully");
                    res
                }
                Err(e) => {
                    error!("failed to initialize p2p service: {e}");
                    std::process::exit(1);
                }
            }
        };

        tokio::task::spawn(p2p_service.run());

        let contracts = ContractBuilder::new(wallet.provider().root().clone())
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .build()
            .unwrap();

        let contracts_with_wallet = ContractBuilder::new(wallet.provider())
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .build()
            .unwrap();

        let group_store_context = store_context.clone();
        let mut scheduler_plugins: Vec<SchedulerPlugin> = Vec::new();
        let mut status_update_plugins: Vec<StatusUpdatePlugin> = vec![];
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
                        status_update_plugins.push(plugin.into());
                        info!("Plugin: Webhook plugin initialized");
                    }
                }
                Err(e) => {
                    error!("Failed to parse webhook configs from environment: {e}");
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
                    error!("Error running webhook sender: {e}");
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

                    let group_plugin = Arc::new(NodeGroupsPlugin::new(
                        configs,
                        store.clone(),
                        group_store_context.clone(),
                        Some(node_groups_heartbeats.clone()),
                        Some(webhook_plugins.clone()),
                    ));

                    // Register the plugin as a task observer
                    group_store_context
                        .task_store
                        .add_observer(group_plugin.clone())
                        .await;

                    let status_group_plugin = group_plugin.clone();
                    let group_plugin_for_server = group_plugin.clone();

                    node_groups_plugin = Some(group_plugin_for_server);
                    scheduler_plugins.push(group_plugin.into());
                    status_update_plugins.push(status_group_plugin.into());
                    info!("Plugin: Node group plugin initialized");
                }
                Ok(_) => {
                    info!(
                        "No node group configurations provided in environment, skipping plugin setup"
                    );
                }
                Err(e) => {
                    error!("Failed to parse node group configurations from environment: {e}");
                    std::process::exit(1);
                }
            }
        }

        let scheduler = Scheduler::new(store_context.clone(), scheduler_plugins);

        // Only spawn processor tasks if in ProcessorOnly or Full mode
        if matches!(server_mode, ServerMode::ProcessorOnly | ServerMode::Full) {
            // Start metrics sync service to centralize metrics from Redis to Prometheus
            let metrics_sync_store_context = store_context.clone();
            let metrics_sync_context = metrics_context.clone();
            let metrics_sync_node_groups = node_groups_plugin.clone();
            tasks.spawn(async move {
                let sync_service = MetricsSyncService::new(
                    metrics_sync_store_context,
                    metrics_sync_context,
                    server_mode,
                    10,
                    metrics_sync_node_groups,
                );
                sync_service.run().await
            });

            if let Some(group_plugin) = node_groups_plugin.clone() {
                tasks.spawn(async move {
                    group_plugin
                        .run_group_management_loop(self.node_group_management_interval)
                        .await
                });
            }

            // Create status_update_plugins for discovery monitor
            let mut discovery_status_update_plugins: Vec<StatusUpdatePlugin> = vec![];

            // Add webhook plugins to discovery status update plugins
            for plugin in &webhook_plugins {
                discovery_status_update_plugins.push(plugin.into());
            }

            // Add node groups plugin if available
            if let Some(group_plugin) = node_groups_plugin.clone() {
                discovery_status_update_plugins.push(group_plugin.into());
            }

            let discovery_store_context = store_context.clone();
            let discovery_heartbeats = heartbeats.clone();
            let monitor = match DiscoveryMonitor::new(
                compute_pool_id,
                self.discovery_refresh_interval,
                discovery_store_context.clone(),
                discovery_heartbeats.clone(),
                discovery_status_update_plugins,
                kademlia_action_tx,
                wallet.provider().root().clone(),
                contracts.clone(),
                self.location_service_url,
                self.location_service_api_key,
            ) {
                Ok(monitor) => {
                    info!("Discovery monitor initialized successfully");
                    monitor
                }
                Err(e) => {
                    error!("Failed to initialize discovery monitor: {e}");
                    std::process::exit(1);
                }
            };

            tasks.spawn(monitor.run(cancellation_token.clone()).map(|_| Ok(())));

            let inviter_store_context = store_context.clone();
            let inviter_heartbeats = heartbeats.clone();
            let wallet = wallet.clone();
            let inviter = match NodeInviter::new(
                wallet,
                compute_pool_id,
                domain_id,
                self.host.as_deref(),
                Some(&self.port),
                self.url.as_deref(),
                inviter_store_context.clone(),
                inviter_heartbeats.clone(),
                invite_tx,
            ) {
                Ok(inviter) => {
                    info!("Node inviter initialized successfully");
                    inviter
                }
                Err(e) => {
                    error!("Failed to initialize node inviter: {e}");
                    std::process::exit(1);
                }
            };

            tasks.spawn(async move { inviter.run().await });

            // Create status_update_plugins for status updater
            let mut status_updater_plugins: Vec<StatusUpdatePlugin> = vec![];

            // Add webhook plugins to status updater plugins
            for plugin in &webhook_plugins {
                status_updater_plugins.push(plugin.into());
            }

            // Add node groups plugin if available
            if let Some(group_plugin) = node_groups_plugin.clone() {
                status_updater_plugins.push(group_plugin.into());
            }

            let status_update_store_context = store_context.clone();
            let status_update_heartbeats = heartbeats.clone();
            let status_update_metrics = metrics_context.clone();
            tasks.spawn({
                let contracts = contracts_with_wallet.clone();
                async move {
                    let status_updater = NodeStatusUpdater::new(
                        status_update_store_context.clone(),
                        15,
                        None,
                        contracts,
                        compute_pool_id,
                        self.disable_ejection,
                        status_update_heartbeats.clone(),
                        status_updater_plugins,
                        status_update_metrics,
                    );
                    status_updater.run().await
                }
            });
        }

        let port = self.port;
        let server_store_context = store_context.clone();
        let s3_credentials = std::env::var("S3_CREDENTIALS").ok();
        let storage_provider: Option<Arc<dyn shared::utils::StorageProvider>> =
            match (self.bucket_name.as_ref(), s3_credentials) {
                (Some(bucket_name), Some(s3_credentials))
                    if !bucket_name.is_empty() && !s3_credentials.is_empty() =>
                {
                    let gcs_storage = GcsStorageProvider::new(bucket_name, &s3_credentials)
                        .await
                        .unwrap_or_else(|_| panic!("Failed to create GCS storage provider"));
                    Some(Arc::new(gcs_storage) as Arc<dyn shared::utils::StorageProvider>)
                }
                _ => {
                    info!("Bucket name or S3 credentials not provided, storage provider disabled");
                    None
                }
            };

        // Always start server regardless of mode
        tokio::select! {
            res = start_server(
                "0.0.0.0",
                port,
                server_store_context.clone(),
                self.admin_api_key,
                storage_provider,
                heartbeats.clone(),
                store.clone(),
                self.hourly_s3_upload_limit,
                Some(contracts_with_wallet),
                compute_pool_id,
                server_mode,
                scheduler,
                node_groups_plugin,
                metrics_context,
                get_task_logs_tx,
                restart_task_tx,
            ) => {
                if let Err(e) = res {
                    error!("Server error: {e}");
                }
            }
            Some(res) = tasks.join_next() => {
                if let Err(e) = res? {
                    error!("Task error: {e}");
                }
            }
            _ = cancellation_token.cancelled() => {
                error!("Shutdown signal received");
            }
        }

        tasks.shutdown().await;
        Ok(())
    }
}
