use crate::plugins::webhook::WebhookPlugin;
use crate::store::core::StoreContext;
use anyhow::Result;
use log::info;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

pub struct MetricsWebhookSender {
    store_context: Arc<StoreContext>,
    webhook_plugins: Vec<WebhookPlugin>,
    last_sent_metrics: HashMap<String, f64>,
}

impl MetricsWebhookSender {
    pub fn new(store_context: Arc<StoreContext>, webhook_plugins: Vec<WebhookPlugin>) -> Self {
        Self {
            store_context,
            webhook_plugins,
            last_sent_metrics: HashMap::new(),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut interval = interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let metrics = self
                .store_context
                .metrics_store
                .get_aggregate_metrics_for_all_tasks();

            if metrics != self.last_sent_metrics {
                info!("Sending {} metrics via webhook", metrics.len());
                for plugin in &self.webhook_plugins {
                    let _ = plugin.send_metrics_updated(metrics.clone()).await;
                }
                // Update last sent metrics
                self.last_sent_metrics = metrics.clone();
            }
        }
    }
}
