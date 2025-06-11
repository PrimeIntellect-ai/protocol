use crate::metrics::MetricsContext;
use crate::store::core::StoreContext;
use crate::ServerMode;
use log::{debug, error, info};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct MetricsSyncService {
    store_context: Arc<StoreContext>,
    metrics_context: Arc<MetricsContext>,
    server_mode: ServerMode,
    sync_interval: Duration,
}

impl MetricsSyncService {
    pub fn new(
        store_context: Arc<StoreContext>,
        metrics_context: Arc<MetricsContext>,
        server_mode: ServerMode,
        sync_interval_seconds: u64,
    ) -> Self {
        Self {
            store_context,
            metrics_context,
            server_mode,
            sync_interval: Duration::from_secs(sync_interval_seconds),
        }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        // Only run the sync service on ProcessorOnly or Full mode instances
        if !matches!(
            self.server_mode,
            ServerMode::ProcessorOnly | ServerMode::Full
        ) {
            debug!("Metrics sync service disabled for ApiOnly mode");
            return Ok(());
        }

        info!(
            "Starting metrics sync service (interval: {:?})",
            self.sync_interval
        );
        let mut interval = interval(self.sync_interval);

        loop {
            interval.tick().await;
            if let Err(e) = self.sync_metrics_from_redis().await {
                error!("Error syncing metrics from Redis: {}", e);
            }
        }
    }

    pub async fn sync_metrics_from_redis(&self) -> anyhow::Result<()> {
        debug!("Syncing metrics from Redis to Prometheus");

        // Get all metrics from Redis
        let redis_metrics = match self.store_context.metrics_store.get_all_metrics().await {
            Ok(metrics) => metrics,
            Err(e) => {
                error!("Failed to get metrics from Redis: {}", e);
                return Err(e);
            }
        };

        // Clear existing Prometheus metrics
        self.metrics_context.clear_all_metrics();

        // Rebuild metrics from Redis data
        let mut total_metrics = 0;
        for (task_id, task_metrics) in redis_metrics {
            for (label, node_metrics) in task_metrics {
                for (node_address, value) in node_metrics {
                    self.metrics_context.record_compute_task_gauge(
                        &node_address,
                        &task_id,
                        &label,
                        value,
                    );
                    total_metrics += 1;
                }
            }
        }

        debug!(
            "Synced {} metric entries from Redis to Prometheus",
            total_metrics
        );
        Ok(())
    }
}
