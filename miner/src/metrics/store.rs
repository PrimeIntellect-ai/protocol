use shared::models::metric::{Metric, MetricKey, MetricsMap};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct MetricsStore {
    metrics: RwLock<MetricsMap>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            metrics: RwLock::new(HashMap::new()),
        }
    }

    pub async fn update_metric(&self, task_id: String, label: String, value: f64) {
        let mut metrics = self.metrics.write().await;
        let key = MetricKey { task_id, label };
        metrics.insert(key, Metric { value });
    }

    pub async fn get_metrics_for_task(&self, task_id: String) -> MetricsMap {
        let metrics = self.metrics.read().await;
        metrics
            .iter()
            .filter(|(key, _)| key.task_id == task_id)
            .map(|(key, metric)| (key.clone(), metric.clone()))
            .collect()
    }

    #[allow(dead_code)]
    pub async fn get_all_metrics(&self) -> MetricsMap {
        let metrics = self.metrics.read().await;
        metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_store() {
        let store = MetricsStore::new();
        store
            .update_metric("task1".to_string(), "progress".to_string(), 1.0)
            .await;
        store
            .update_metric("task1".to_string(), "cpu_usage".to_string(), 90.0)
            .await;
        store
            .update_metric("task2".to_string(), "test".to_string(), 2.0)
            .await;

        let metrics = store.get_metrics_for_task("task1".to_string()).await;
        assert_eq!(metrics.len(), 2);

        let all_metrics = store.get_all_metrics().await;
        assert_eq!(all_metrics.len(), 3);
    }
}
