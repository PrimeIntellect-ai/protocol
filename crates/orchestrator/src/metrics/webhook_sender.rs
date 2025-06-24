use crate::plugins::webhook::WebhookPlugin;
use crate::store::core::StoreContext;
use anyhow::Result;
use log::{error, info};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct MetricsWebhookSender {
    store_context: Arc<StoreContext>,
    webhook_plugins: Vec<WebhookPlugin>,
    last_sent_metrics: HashMap<String, f64>,
    pool_id: u32,
}

impl MetricsWebhookSender {
    pub fn new(
        store_context: Arc<StoreContext>,
        webhook_plugins: Vec<WebhookPlugin>,
        pool_id: u32,
    ) -> Self {
        Self {
            store_context,
            webhook_plugins,
            last_sent_metrics: HashMap::new(),
            pool_id,
        }
    }

    pub fn metrics_changed(
        metrics: &HashMap<String, f64>,
        last_sent_metrics: &HashMap<String, f64>,
    ) -> bool {
        if metrics.len() != last_sent_metrics.len() {
            return true;
        }

        // FP imprecision fix
        const EPSILON: f64 = 1e-10;

        for (key, value) in metrics {
            match last_sent_metrics.get(key) {
                None => return true,
                Some(last_value) if (last_value - value).abs() > EPSILON => return true,
                _ => continue,
            }
        }

        false
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let metrics = match self
                .store_context
                .metrics_store
                .get_aggregate_metrics_for_all_tasks()
                .await
            {
                Ok(metrics) => metrics,
                Err(e) => {
                    error!("Error getting aggregate metrics for all tasks: {e}");
                    continue;
                }
            };

            if Self::metrics_changed(&metrics, &self.last_sent_metrics) {
                info!("Sending {} metrics via webhook", metrics.len());
                for plugin in &self.webhook_plugins {
                    let _ = plugin
                        .send_metrics_updated(self.pool_id, metrics.clone())
                        .await;
                }
                // Update last sent metrics
                self.last_sent_metrics = metrics.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_changed() {
        let mut metrics = HashMap::new();
        metrics.insert("test_metric".to_string(), 1.0);
        metrics.insert("metric_2".to_string(), 2.0);

        let mut last_sent_metrics = HashMap::new();
        last_sent_metrics.insert("metric_2".to_string(), 2.0);
        last_sent_metrics.insert("test_metric".to_string(), 1.0);

        let metrics_changed = MetricsWebhookSender::metrics_changed(&metrics, &last_sent_metrics);
        assert!(!metrics_changed);
    }
}
