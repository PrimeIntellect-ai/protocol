use shared::models::metric::Metric;
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct MetricsStore {
    // taskid -> (label -> metric)
    metrics: RwLock<HashMap<String, HashMap<String, Metric>>>,
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

    pub async fn get_task_metrics(&self, taskid: &str) -> Option<HashMap<String, Metric>> {
        let metrics = self.metrics.read().await;
        metrics.get(taskid).cloned()
    }

    pub async fn get_all_metrics(&self) -> HashMap<String, HashMap<String, Metric>> {
        let metrics = self.metrics.read().await;
        metrics.clone()
    }
}
