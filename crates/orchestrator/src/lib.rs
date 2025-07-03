mod api;
mod discovery;
mod events;
mod metrics;
mod models;
mod node;
mod p2p;
mod plugins;
mod scheduler;
mod status_update;
mod store;
mod utils;

pub use api::server::start_server;
pub use discovery::monitor::DiscoveryMonitor;
pub use metrics::sync_service::MetricsSyncService;
pub use metrics::webhook_sender::MetricsWebhookSender;
pub use metrics::MetricsContext;
pub use node::invite::NodeInviter;
pub use p2p::client::P2PClient;
pub use plugins::node_groups::NodeGroupConfiguration;
pub use plugins::node_groups::NodeGroupsPlugin;
pub use plugins::webhook::WebhookConfig;
pub use plugins::webhook::WebhookPlugin;
pub use plugins::SchedulerPlugin;
pub use plugins::StatusUpdatePlugin;
pub use scheduler::Scheduler;
pub use status_update::NodeStatusUpdater;
pub use store::core::RedisStore;
pub use store::core::StoreContext;
pub use utils::loop_heartbeats::LoopHeartbeats;

#[derive(clap::Parser, Clone, Copy, clap::ValueEnum, Debug, PartialEq)]
pub enum ServerMode {
    ApiOnly,
    ProcessorOnly,
    Full,
}
