use std::time::Duration;

use anyhow::Error;
use serde_json::json;

use crate::{
    models::node::{NodeStatus, OrchestratorNode},
    prelude::Plugin,
};

use super::StatusUpdatePlugin;
use log::{debug, error};

pub struct WebhookPlugin {
    webhook_url: String,
}

impl WebhookPlugin {
    pub fn new(webhook_url: String) -> Self {
        Self { webhook_url }
    }
}

impl Plugin for WebhookPlugin {}

#[async_trait::async_trait]
impl StatusUpdatePlugin for WebhookPlugin {
    async fn handle_status_change(
        &self,
        node: &OrchestratorNode,
        old_status: &NodeStatus,
    ) -> Result<(), Error> {
        if *old_status == node.status
            || node.status == NodeStatus::Unhealthy
            || node.status == NodeStatus::Discovered
        {
            return Ok(());
        }

        let payload = json!({
            "node_address": node.address.to_string(),
            "ip_address": node.ip_address,
            "port": node.port,
            "old_status": old_status.to_string(),
            "new_status": node.status.to_string(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        let webhook_url = self.webhook_url.clone();
        let client = reqwest::Client::new();
        tokio::spawn(async move {
            if let Err(e) = client
                .post(&webhook_url)
                .json(&payload)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                error!("Failed to send webhook to {}: {}", webhook_url, e);
            } else {
                debug!("Webhook to {} triggered successfully", webhook_url);
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;

    use std::str::FromStr;

    fn create_test_node(status: NodeStatus) -> OrchestratorNode {
        OrchestratorNode {
            address: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status,
            task_id: None,
            task_state: None,
            version: None,
            last_status_change: None,
        }
    }

    #[tokio::test]
    async fn test_webhook_sends_on_status_change() {
        let plugin = WebhookPlugin {
            webhook_url: "https://example.com/webhook".to_string(),
        };

        let node = create_test_node(NodeStatus::Dead);
        let result = plugin
            .handle_status_change(&node, &NodeStatus::Healthy)
            .await;

        assert!(result.is_ok());
    }
}
