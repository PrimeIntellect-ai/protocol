use anyhow::Result;
use shared::models::metric::{MetricEntry, MetricKey};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct MetricsStore {
    metrics: RwLock<HashMap<MetricKey, f64>>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            metrics: RwLock::new(HashMap::new()),
        }
    }

    pub async fn update_metric(&self, task_id: String, label: String, value: f64) -> Result<()> {
        if !value.is_finite() {
            anyhow::bail!("Value must be a finite number");
        }
        let mut metrics = self.metrics.write().await;
        let key = MetricKey { task_id, label };
        metrics.insert(key, value);
        Ok(())
    }

    pub async fn get_metrics_for_task(&self, task_id: String) -> Vec<MetricEntry> {
        self.metrics
            .read()
            .await
            .iter()
            .filter(|(k, _)| k.task_id == task_id)
            .map(|(k, &v)| MetricEntry {
                key: k.clone(),
                value: v,
            })
            .collect()
    }

    pub async fn clear_metrics_for_task(&self, task_id: &str) {
        let mut metrics = self.metrics.write().await;
        metrics.retain(|key, _| key.task_id != task_id);
    }

    #[allow(dead_code)]
    pub async fn get_all_metrics(&self) -> HashMap<MetricKey, f64> {
        let metrics = self.metrics.read().await;
        metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_store() -> Result<()> {
        let store = MetricsStore::new();
        store
            .update_metric("task1".to_string(), "progress".to_string(), 1.0)
            .await?;
        store
            .update_metric("task1".to_string(), "cpu_usage".to_string(), 90.0)
            .await?;
        store
            .update_metric("task2".to_string(), "test".to_string(), 2.0)
            .await?;

        let metrics = store.get_metrics_for_task("task1".to_string()).await;
        assert_eq!(metrics.len(), 2);

        let all_metrics = store.get_all_metrics().await;
        assert_eq!(all_metrics.len(), 3);

        // Test invalid value
        let result = store
            .update_metric("task1".to_string(), "invalid".to_string(), f64::INFINITY)
            .await;
        assert!(result.is_err());

        Ok(())
    }
}
