use crate::services::discovery::DiscoveryService;
use crate::state::system_state::SystemState;
use log::{debug, error, info};
use shared::models::node::Node;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

const INITIAL_UPDATE_DELAY: Duration = Duration::from_secs(120);
const UPDATE_INTERVAL: Duration = Duration::from_secs(120);

pub(crate) struct DiscoveryUpdater {
    discovery_service: Arc<DiscoveryService>,
    is_running: Arc<AtomicBool>,
    system_state: Arc<SystemState>,
    cancellation_token: Arc<CancellationToken>,
}

impl DiscoveryUpdater {
    pub(crate) fn new(discovery_service: DiscoveryService, system_state: Arc<SystemState>) -> Self {
        Self {
            discovery_service: Arc::new(discovery_service),
            is_running: Arc::new(AtomicBool::new(false)),
            system_state,
            cancellation_token: Arc::new(CancellationToken::new()),
        }
    }

    pub(crate) fn start_auto_update(&self, node_config: Node) {
        if self.is_running.load(Ordering::SeqCst) {
            debug!("Auto update already running, skipping start");
            return;
        }

        self.is_running.store(true, Ordering::SeqCst);
        let is_running = self.is_running.clone();
        let discovery_service = self.discovery_service.clone();
        let system_state = self.system_state.clone();
        let cancellation_token = self.cancellation_token.clone();

        tokio::spawn(async move {
            debug!("Starting discovery info auto-update task");

            // Initial delay before first update
            tokio::select! {
                _ = sleep(INITIAL_UPDATE_DELAY) => {},
                _ = cancellation_token.cancelled() => {
                    is_running.store(false, Ordering::SeqCst);
                    return;
                }
            }

            while is_running.load(Ordering::SeqCst) {
                // Check if we're in a compute pool by checking the heartbeat endpoint
                let should_update = !system_state.is_running().await;

                if should_update {
                    if let Err(e) = discovery_service.upload_discovery_info(&node_config).await {
                        error!("Failed to update discovery info: {e}");
                    } else {
                        info!("Successfully updated discovery info");
                    }
                }

                // Sleep before next check, but check for cancellation
                tokio::select! {
                    _ = sleep(UPDATE_INTERVAL) => {},
                    _ = cancellation_token.cancelled() => {
                        is_running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }
            debug!("Discovery info auto-update task finished");
        });
    }
}

impl Clone for DiscoveryUpdater {
    fn clone(&self) -> Self {
        Self {
            discovery_service: self.discovery_service.clone(),
            is_running: self.is_running.clone(),
            system_state: self.system_state.clone(),
            cancellation_token: self.cancellation_token.clone(),
        }
    }
}
