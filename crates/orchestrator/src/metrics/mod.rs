use prometheus::{CounterVec, GaugeVec, HistogramOpts, HistogramVec, Opts, Registry, TextEncoder};
pub(crate) mod sync_service;
pub(crate) mod webhook_sender;

pub struct MetricsContext {
    pub compute_task_gauges: GaugeVec,
    pub task_info: GaugeVec,
    pub pool_id: String,
    pub registry: Registry,
    pub file_upload_requests_total: CounterVec,
    pub nodes_total: GaugeVec,
    pub tasks_total: GaugeVec,
    pub groups_total: GaugeVec,
    pub heartbeat_requests_total: CounterVec,
    pub nodes_per_task: GaugeVec,
    pub task_state: GaugeVec,
    pub status_update_execution_time: HistogramVec,
}

impl MetricsContext {
    pub fn new(pool_id: String) -> Self {
        // For current state/rate metrics
        let compute_task_gauges = GaugeVec::new(
            Opts::new("compute_gauges", "Compute task gauge metrics"),
            &[
                "node_address",
                "task_id",
                "task_name",
                "label",
                "pool_id",
                "group_id",
                "group_config_name",
            ],
        )
        .unwrap();

        let task_info = GaugeVec::new(
            Opts::new("task_info", "Task information with metadata"),
            &["task_id", "task_name", "pool_id", "metadata"],
        )
        .unwrap();

        // New metrics for orchestrator statistics
        let file_upload_requests_total = CounterVec::new(
            Opts::new(
                "orchestrator_file_upload_requests_total",
                "Total number of file upload requests",
            ),
            &["task_id", "task_name", "node_address", "pool_id"],
        )
        .unwrap();

        let nodes_total = GaugeVec::new(
            Opts::new(
                "orchestrator_nodes_total",
                "Total number of nodes by status",
            ),
            &["status", "pool_id"],
        )
        .unwrap();

        let tasks_total = GaugeVec::new(
            Opts::new("orchestrator_tasks_total", "Total number of tasks"),
            &["pool_id"],
        )
        .unwrap();

        let groups_total = GaugeVec::new(
            Opts::new(
                "orchestrator_groups_total",
                "Total number of node groups by configuration",
            ),
            &["configuration_name", "pool_id"],
        )
        .unwrap();

        let heartbeat_requests_total = CounterVec::new(
            Opts::new(
                "orchestrator_heartbeat_requests_total",
                "Total number of heartbeat requests per node",
            ),
            &["node_address", "pool_id"],
        )
        .unwrap();

        let nodes_per_task = GaugeVec::new(
            Opts::new(
                "orchestrator_nodes_per_task",
                "Number of nodes actively working on each task",
            ),
            &["task_id", "task_name", "pool_id"],
        )
        .unwrap();
        let task_state = GaugeVec::new(
            Opts::new(
                "orchestrator_task_state",
                "Task state reported from nodes (1 for active state, 0 for inactive)",
            ),
            &["node_address", "task_id", "task_state", "pool_id"],
        )
        .unwrap();

        let status_update_execution_time = HistogramVec::new(
            HistogramOpts::new(
                "orchestrator_status_update_execution_time_seconds",
                "Duration of status update execution",
            )
            .buckets(vec![
                0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 45.0, 60.0, 90.0,
                120.0,
            ]),
            &["node_address", "pool_id"],
        )
        .unwrap();

        let registry = Registry::new();
        let _ = registry.register(Box::new(compute_task_gauges.clone()));
        let _ = registry.register(Box::new(task_info.clone()));
        let _ = registry.register(Box::new(file_upload_requests_total.clone()));
        let _ = registry.register(Box::new(nodes_total.clone()));
        let _ = registry.register(Box::new(tasks_total.clone()));
        let _ = registry.register(Box::new(groups_total.clone()));
        let _ = registry.register(Box::new(heartbeat_requests_total.clone()));
        let _ = registry.register(Box::new(nodes_per_task.clone()));
        let _ = registry.register(Box::new(task_state.clone()));
        let _ = registry.register(Box::new(status_update_execution_time.clone()));

        Self {
            compute_task_gauges,
            task_info,
            pool_id,
            registry,
            file_upload_requests_total,
            nodes_total,
            tasks_total,
            groups_total,
            heartbeat_requests_total,
            nodes_per_task,
            task_state,
            status_update_execution_time,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_compute_task_gauge(
        &self,
        node_address: &str,
        task_id: &str,
        task_name: &str,
        label: &str,
        value: f64,
        group_id: Option<&str>,
        group_config_name: Option<&str>,
    ) {
        let group_id_str = group_id.unwrap_or("none");
        let group_config_name_str = group_config_name.unwrap_or("none");
        self.compute_task_gauges
            .with_label_values(&[
                node_address,
                task_id,
                task_name,
                label,
                &self.pool_id,
                group_id_str,
                group_config_name_str,
            ])
            .set(value);
    }

    pub fn increment_file_upload_requests(
        &self,
        task_id: &str,
        task_name: &str,
        node_address: &str,
    ) {
        self.file_upload_requests_total
            .with_label_values(&[task_id, task_name, node_address, &self.pool_id])
            .inc();
    }

    pub fn increment_heartbeat_requests(&self, node_address: &str) {
        self.heartbeat_requests_total
            .with_label_values(&[node_address, &self.pool_id])
            .inc();
    }

    pub fn set_nodes_count(&self, status: &str, count: f64) {
        self.nodes_total
            .with_label_values(&[status, &self.pool_id])
            .set(count);
    }

    pub fn set_tasks_count(&self, count: f64) {
        self.tasks_total
            .with_label_values(&[&self.pool_id])
            .set(count);
    }

    pub fn set_groups_count(&self, configuration_name: &str, count: f64) {
        self.groups_total
            .with_label_values(&[configuration_name, &self.pool_id])
            .set(count);
    }

    pub fn set_nodes_per_task(&self, task_id: &str, task_name: &str, count: f64) {
        self.nodes_per_task
            .with_label_values(&[task_id, task_name, &self.pool_id])
            .set(count);
    }

    pub fn set_task_info(&self, task_id: &str, task_name: &str, metadata: &str) {
        self.task_info
            .with_label_values(&[task_id, task_name, &self.pool_id, metadata])
            .set(1.0);
    }

    pub fn set_task_state(&self, node_address: &str, task_id: &str, task_state: &str) {
        self.task_state
            .with_label_values(&[node_address, task_id, task_state, &self.pool_id])
            .set(1.0);
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

    pub fn record_status_update_execution_time(&self, node_address: &str, duration: f64) {
        self.status_update_execution_time
            .with_label_values(&[node_address, &self.pool_id])
            .observe(duration);
    }

    /// Clear all orchestrator statistics metrics
    pub fn clear_orchestrator_statistics(&self) {
        self.nodes_total.reset();
        self.tasks_total.reset();
        self.groups_total.reset();
        self.nodes_per_task.reset();
        self.task_info.reset();
        self.task_state.reset();
    }
}
