use shared::models::metric::{Metric, MetricsMap};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct MetricsStore {
    // taskid -> (label -> metric)
    metrics: RwLock<MetricsMap>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            metrics: RwLock::new(HashMap::new()),
        }
    }

    pub async fn update_metric(&self, metric: Metric) {
        let mut metrics = self.metrics.write().await;
        metrics
            .entry(metric.taskid.clone())
            .or_insert_with(HashMap::new)
            .insert(metric.label.clone(), metric);
    }

    pub async fn get_all_metrics(&self) -> MetricsMap {
        let metrics = self.metrics.read().await;
        metrics.clone()
    }
}
