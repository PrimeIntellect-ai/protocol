use prometheus::{GaugeVec, Opts, Registry, TextEncoder};
pub mod sync_service;
pub mod webhook_sender;

pub struct MetricsContext {
    pub compute_task_gauges: GaugeVec,
    pub pool_id: String,
    pub registry: Registry,
}

impl MetricsContext {
    pub fn new(pool_id: String) -> Self {
        // For current state/rate metrics
        let compute_task_gauges = GaugeVec::new(
            Opts::new("compute_gauges", "Compute task gauge metrics"),
            &["node_address", "task_id", "label", "pool_id"],
        )
        .unwrap();
        let registry = Registry::new();
        let _ = registry.register(Box::new(compute_task_gauges.clone()));

        Self {
            compute_task_gauges,
            pool_id,
            registry,
        }
    }

    pub fn record_compute_task_gauge(
        &self,
        node_address: &str,
        task_id: &str,
        label: &str,
        value: f64,
    ) {
        self.compute_task_gauges
            .with_label_values(&[node_address, task_id, label, &self.pool_id])
            .set(value);
    }

    pub fn remove_compute_task_gauge(&self, node_address: &str, task_id: &str, label: &str) {
        if let Err(e) = self.compute_task_gauges.remove_label_values(&[
            node_address,
            task_id,
            label,
            &self.pool_id,
        ]) {
            println!("Error removing compute task gauge: {}", e);
        }
    }

    pub fn export_metrics(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode_to_string(&metric_families)
    }

    /// Clear all metrics from the registry
    pub fn clear_compute_task_metrics(&self) {
        // Clear all time series from the compute_task_gauges metric family
        // This removes all existing metrics so we can rebuild from Redis
        self.compute_task_gauges.reset();
    }
}
