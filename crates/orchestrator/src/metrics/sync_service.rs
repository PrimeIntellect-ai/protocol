use crate::metrics::MetricsContext;
use crate::plugins::node_groups::NodeGroupsPlugin;
use crate::store::core::StoreContext;
use crate::ServerMode;
use log::{debug, error, info};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct MetricsSyncService {
    store_context: Arc<StoreContext>,
    metrics_context: Arc<MetricsContext>,
    server_mode: ServerMode,
    sync_interval: Duration,
    node_groups_plugin: Option<Arc<NodeGroupsPlugin>>,
}

impl MetricsSyncService {
    pub fn new(
        store_context: Arc<StoreContext>,
        metrics_context: Arc<MetricsContext>,
        server_mode: ServerMode,
        sync_interval_seconds: u64,
        node_groups_plugin: Option<Arc<NodeGroupsPlugin>>,
    ) -> Self {
        Self {
            store_context,
            metrics_context,
            server_mode,
            sync_interval: Duration::from_secs(sync_interval_seconds),
            node_groups_plugin,
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
            if let Err(e) = self.sync_orchestrator_statistics().await {
                error!("Error syncing orchestrator statistics: {}", e);
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

        // Get all tasks to map task_id to task_name
        let tasks = self
            .store_context
            .task_store
            .get_all_tasks()
            .await
            .unwrap_or_else(|e| {
                error!("Failed to get tasks for task name mapping: {}", e);
                Vec::new()
            });
        let task_name_map: HashMap<String, String> = tasks
            .into_iter()
            .map(|task| (task.id.to_string(), task.name.clone()))
            .collect();

        // Clear existing Prometheus metrics
        self.metrics_context.clear_compute_task_metrics();

        // Rebuild metrics from Redis data
        let mut total_metrics = 0;
        for (task_id, task_metrics) in redis_metrics {
            let task_name = task_name_map.get(&task_id).cloned().unwrap_or_else(|| {
                debug!("No task name found for task_id: {}", task_id);
                "unknown".to_string()
            });

            for (label, node_metrics) in task_metrics {
                for (node_address, value) in node_metrics {
                    self.metrics_context.record_compute_task_gauge(
                        &node_address,
                        &task_id,
                        &task_name,
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

    pub async fn sync_orchestrator_statistics(&self) -> anyhow::Result<()> {
        debug!("Syncing orchestrator statistics to Prometheus");

        // Clear existing orchestrator statistics
        self.metrics_context.clear_orchestrator_statistics();

        // Get nodes once and reuse for multiple statistics
        let nodes = match self.store_context.node_store.get_nodes().await {
            Ok(nodes) => nodes,
            Err(e) => {
                error!("Failed to get nodes for statistics: {}", e);
                Vec::new()
            }
        };

        // Sync nodes count by status
        let mut status_counts: HashMap<String, i32> = HashMap::new();
        for node in &nodes {
            let status = format!("{:?}", node.status).to_lowercase();
            *status_counts.entry(status).or_insert(0) += 1;
        }
        for (status, count) in status_counts {
            self.metrics_context.set_nodes_count(&status, count as f64);
        }
        debug!("Synced node statistics");

        // Sync total tasks count (simple count, not by state)
        if let Ok(tasks) = self.store_context.task_store.get_all_tasks().await {
            let total_tasks = tasks.len() as f64;
            self.metrics_context.set_tasks_count(total_tasks);
            debug!("Synced task statistics: {} total tasks", total_tasks);

            // Sync nodes per task based on node assignments
            // Create task name mapping
            let task_name_map: HashMap<String, String> = tasks
                .into_iter()
                .map(|task| (task.id.to_string(), task.name.clone()))
                .collect();

            // Count nodes per task
            let mut task_node_counts: HashMap<String, i32> = HashMap::new();
            for node in &nodes {
                if let Some(task_id) = &node.task_id {
                    *task_node_counts.entry(task_id.clone()).or_insert(0) += 1;
                }
            }

            // Set metrics for each task with active nodes
            for (task_id, count) in task_node_counts {
                let task_name = task_name_map.get(&task_id).cloned().unwrap_or_else(|| {
                    debug!("No task name found for task_id: {}", task_id);
                    "unknown".to_string()
                });
                self.metrics_context
                    .set_nodes_per_task(&task_id, &task_name, count as f64);
            }
            debug!("Synced nodes per task statistics");
        } else {
            error!("Failed to get tasks for statistics");
        }

        // Sync groups count by configuration name
        if let Some(node_groups_plugin) = &self.node_groups_plugin {
            if let Ok(groups) = node_groups_plugin.get_all_groups().await {
                let mut config_counts: HashMap<String, i32> = HashMap::new();
                for group in groups {
                    *config_counts.entry(group.configuration_name).or_insert(0) += 1;
                }
                for (config_name, count) in config_counts {
                    self.metrics_context
                        .set_groups_count(&config_name, count as f64);
                }
                debug!("Synced group statistics");
            } else {
                error!("Failed to get groups for statistics");
            }
        } else {
            debug!("Node groups plugin not available, skipping groups metrics");
        }

        debug!("Completed syncing orchestrator statistics");
        Ok(())
    }
}
