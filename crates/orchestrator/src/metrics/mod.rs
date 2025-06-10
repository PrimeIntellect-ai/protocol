use std::{collections::HashMap, sync::Arc};

use prometheus::{GaugeVec, Opts, Registry, TextEncoder};
pub mod webhook_sender;
use anyhow::Result;

use crate::store::core::StoreContext;

pub struct MetricsContext {
    pub compute_task_gauges: GaugeVec,
    pub node_status_gauges: GaugeVec,
    pub task_gauges: GaugeVec,
    pub pool_id: String,
    pub registry: Registry,
    pub store_context: Arc<StoreContext>,
}

impl MetricsContext {
    pub fn new(pool_id: String, store_context: Arc<StoreContext>) -> Self {
        // For current state/rate metrics
        let compute_task_gauges = GaugeVec::new(
            Opts::new("compute_gauges", "Compute task gauge metrics"),
            &["node_address", "task_id", "label", "pool_id"],
        )
        .unwrap();

        // Node status metrics
        let node_status_gauges = GaugeVec::new(
            Opts::new("orchestrator_nodes_by_status", "Number of nodes by status"),
            &["status", "pool_id"],
        )
        .unwrap();

        // Task count metrics
        let task_gauges = GaugeVec::new(
            Opts::new("orchestrator_tasks_total", "Total number of tasks"),
            &["pool_id"],
        )
        .unwrap();

        let registry = Registry::new();
        let _ = registry.register(Box::new(compute_task_gauges.clone()));
        let _ = registry.register(Box::new(node_status_gauges.clone()));
        let _ = registry.register(Box::new(task_gauges.clone()));

        Self {
            compute_task_gauges,
            node_status_gauges,
            task_gauges,
            pool_id,
            registry,
            store_context,
        }
    }

    pub async fn fill_metrics_interval(&self) -> Result<()> {
        // Nodes
        let nodes = self.store_context.node_store.get_nodes().await?;
        let mut status_counts = HashMap::new();
        for node in nodes {
            let status = node.status.to_string();
            *status_counts.entry(status).or_insert(0) += 1;
        }
        for (status, count) in status_counts {
            self.record_nodes_by_status(&status, count as f64);
        }

        // Tasks
        let tasks = self.store_context.task_store.get_all_tasks().await?;
        self.record_task_gauge(tasks.len() as f64);

        Ok(())
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

    pub fn record_nodes_by_status(&self, status: &str, count: f64) {
        self.node_status_gauges
            .with_label_values(&[status, &self.pool_id])
            .set(count);
    }

    pub fn record_task_gauge(&self, count: f64) {
        self.task_gauges
            .with_label_values(&[&self.pool_id])
            .set(count);
    }

    pub fn export_metrics(&self) -> Result<String, prometheus::Error> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder.encode_to_string(&metric_families)
    }
}
