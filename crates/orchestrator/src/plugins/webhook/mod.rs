use std::{sync::Arc, time::Duration};

use anyhow::Error;
use serde::{Deserialize, Serialize};

use crate::models::node::{NodeStatus, OrchestratorNode};

use log::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum WebhookEvent {
    #[serde(rename = "node.status_changed")]
    NodeStatusChanged {
        node_address: String,
        ip_address: String,
        port: u16,
        old_status: String,
        new_status: String,
    },
    #[serde(rename = "group.created")]
    GroupCreated {
        group_id: String,
        configuration_name: String,
        nodes: Vec<String>,
    },
    #[serde(rename = "group.destroyed")]
    GroupDestroyed {
        group_id: String,
        configuration_name: String,
        nodes: Vec<String>,
    },
    #[serde(rename = "metrics.updated")]
    MetricsUpdated {
        pool_id: u32,
        #[serde(flatten)]
        metrics: std::collections::HashMap<String, f64>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookPayload {
    #[serde(flatten)]
    pub event: WebhookEvent,
    pub timestamp: String,
}

impl WebhookPayload {
    pub fn new(event: WebhookEvent) -> Self {
        #[cfg(test)]
        let timestamp = "2024-01-01T00:00:00Z".to_string();
        #[cfg(not(test))]
        let timestamp = chrono::Utc::now().to_rfc3339();

        Self { event, timestamp }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub bearer_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebhookPlugin {
    webhook_url: String,
    client: Arc<reqwest::Client>,
}

impl WebhookPlugin {
    const MAX_RETRIES: u32 = 5;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
    const BASE_RETRY_DELAY_MS: u64 = 500;
    const MAX_RETRY_DELAY_MS: u64 = 10000;

    pub fn new(webhook_config: WebhookConfig) -> Self {
        let client = Arc::new(
            reqwest::Client::builder()
                .default_headers(Self::build_headers(&webhook_config.bearer_token))
                .timeout(Self::REQUEST_TIMEOUT)
                .build()
                .expect("Failed to build HTTP client"),
        );

        Self {
            webhook_url: webhook_config.url,
            client,
        }
    }

    fn build_headers(bearer_token: &Option<String>) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(token) = bearer_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                    .expect("Invalid bearer token"),
            );
        }
        headers
    }

    fn calculate_retry_delay(attempt: u32) -> Duration {
        let delay_ms = std::cmp::min(
            Self::BASE_RETRY_DELAY_MS * (2_u64.pow(attempt)),
            Self::MAX_RETRY_DELAY_MS,
        );
        Duration::from_millis(delay_ms)
    }

    fn get_event_name(event: &WebhookEvent) -> &'static str {
        match event {
            WebhookEvent::NodeStatusChanged { .. } => "node.status_changed",
            WebhookEvent::GroupCreated { .. } => "group.created",
            WebhookEvent::GroupDestroyed { .. } => "group.destroyed",
            WebhookEvent::MetricsUpdated { .. } => "metrics.updated",
        }
    }

    async fn send_webhook_request(&self, payload: &WebhookPayload) -> Result<(), Error> {
        let start_time = std::time::Instant::now();

        let response = self
            .client
            .post(&self.webhook_url)
            .json(payload)
            .send()
            .await?;

        let duration = start_time.elapsed();
        let status = response.status();

        if response.status().is_success() {
            info!(
                "Webhook '{}' sent successfully to {} (HTTP {}, took {:?})",
                Self::get_event_name(&payload.event),
                self.webhook_url,
                status,
                duration
            );
            Ok(())
        } else {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read error response".to_string());
            Err(anyhow::anyhow!(
                "HTTP {} after {:?}: {}",
                status,
                duration,
                error_text
            ))
        }
    }

    async fn send_with_retry(&self, payload: WebhookPayload) -> Result<(), Error> {
        let event_name = Self::get_event_name(&payload.event);
        let mut last_error = None;

        info!(
            "Sending webhook '{}' to {} (max {} retries)",
            event_name,
            self.webhook_url,
            Self::MAX_RETRIES
        );

        for attempt in 0..=Self::MAX_RETRIES {
            match self.send_webhook_request(&payload).await {
                Ok(()) => {
                    if attempt > 0 {
                        info!(
                            "Webhook '{}' succeeded on attempt {} after retries",
                            event_name,
                            attempt + 1
                        );
                    }
                    return Ok(());
                }
                Err(e) => {
                    if attempt < Self::MAX_RETRIES {
                        let delay = Self::calculate_retry_delay(attempt);
                        warn!(
                            "Webhook '{}' attempt {} failed: {}, retrying in {:?}",
                            event_name,
                            attempt + 1,
                            e,
                            delay
                        );
                        tokio::time::sleep(delay).await;
                    }
                    last_error = Some(e);
                }
            }
        }

        let error = last_error.unwrap_or_else(|| anyhow::anyhow!("Unknown error"));
        error!(
            "Failed to send webhook '{}' to {} after {} attempts: {}",
            event_name,
            self.webhook_url,
            Self::MAX_RETRIES + 1,
            error
        );
        Err(error)
    }

    async fn send_event(&self, event: WebhookEvent) -> Result<(), Error> {
        let payload = WebhookPayload::new(event);

        #[cfg(not(test))]
        {
            let webhook_url = self.webhook_url.clone();
            let client = self.client.clone();
            tokio::spawn(async move {
                let plugin = WebhookPlugin {
                    webhook_url,
                    client,
                };
                if let Err(e) = plugin.send_with_retry(payload).await {
                    // Error already logged in send_with_retry
                    let _ = e;
                }
            });
            Ok(())
        }

        #[cfg(test)]
        {
            self.send_with_retry(payload).await
        }
    }

    pub async fn send_node_status_changed(
        &self,
        node: &OrchestratorNode,
        old_status: &NodeStatus,
    ) -> Result<(), Error> {
        let event = WebhookEvent::NodeStatusChanged {
            node_address: node.address.to_string(),
            ip_address: node.ip_address.clone(),
            port: node.port,
            old_status: old_status.to_string(),
            new_status: node.status.to_string(),
        };

        self.send_event(event).await
    }

    pub async fn send_group_created(
        &self,
        group_id: String,
        configuration_name: String,
        nodes: Vec<String>,
    ) -> Result<(), Error> {
        let event = WebhookEvent::GroupCreated {
            group_id,
            configuration_name,
            nodes,
        };

        self.send_event(event).await
    }

    pub async fn send_group_destroyed(
        &self,
        group_id: String,
        configuration_name: String,
        nodes: Vec<String>,
    ) -> Result<(), Error> {
        let event = WebhookEvent::GroupDestroyed {
            group_id,
            configuration_name,
            nodes,
        };

        self.send_event(event).await
    }

    pub async fn send_metrics_updated(
        &self,
        pool_id: u32,
        metrics: std::collections::HashMap<String, f64>,
    ) -> Result<(), Error> {
        let event = WebhookEvent::MetricsUpdated { pool_id, metrics };

        self.send_event(event).await
    }

    pub(crate) async fn handle_status_change(
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

        if let Err(e) = self.send_node_status_changed(node, old_status).await {
            error!("Failed to send webhook to {}: {}", self.webhook_url, e);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;
    use anyhow::Result;
    use mockito::Server;
    use std::str::FromStr;

    fn create_test_node(status: NodeStatus) -> OrchestratorNode {
        OrchestratorNode {
            address: Address::from_str("0x1234567890123456789012345678901234567890").unwrap(),
            ip_address: "127.0.0.1".to_string(),
            port: 8080,
            status,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_webhook_sends_on_status_change() -> Result<()> {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/webhook")
            .with_status(200)
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "event": "node.status_changed",
                "data": {
                    "node_address": "0x1234567890123456789012345678901234567890",
                    "ip_address": "127.0.0.1",
                    "port": 8080,
                    "old_status": "Dead",
                    "new_status": "Healthy"
                },
                "timestamp": "2024-01-01T00:00:00Z"
            })))
            .create();

        let plugin = WebhookPlugin::new(WebhookConfig {
            url: format!("{}/webhook", server.url()),
            bearer_token: None,
        });
        let node = create_test_node(NodeStatus::Healthy);
        let result = plugin.handle_status_change(&node, &NodeStatus::Dead).await;
        assert!(result.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_webhook_sends_on_group_created() -> Result<()> {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/webhook")
            .with_status(200)
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "event": "group.created",
                "data": {
                    "group_id": "1234567890",
                    "configuration_name": "test_configuration",
                    "nodes": ["0x1234567890123456789012345678901234567890"]
                },
                "timestamp": "2024-01-01T00:00:00Z"
            })))
            .create();

        let plugin = WebhookPlugin::new(WebhookConfig {
            url: format!("{}/webhook", server.url()),
            bearer_token: None,
        });
        let group_id = "1234567890";
        let configuration_name = "test_configuration";
        let nodes = vec!["0x1234567890123456789012345678901234567890".to_string()];
        let result = plugin
            .send_group_created(group_id.to_string(), configuration_name.to_string(), nodes)
            .await;
        assert!(result.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_webhook_sends_on_metrics_updated() -> Result<()> {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/webhook")
            .with_status(200)
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "event": "metrics.updated",
                "data": {
                        "pool_id": 1,
                        "test_metric": 1.0,
                        "metric_2": 2.0
                },
                "timestamp": "2024-01-01T00:00:00Z"
            })))
            .create();

        let plugin = WebhookPlugin::new(WebhookConfig {
            url: format!("{}/webhook", server.url()),
            bearer_token: None,
        });
        let mut metrics = std::collections::HashMap::new();
        metrics.insert("test_metric".to_string(), 1.0);
        metrics.insert("metric_2".to_string(), 2.0);
        let result = plugin.send_metrics_updated(1, metrics).await;
        assert!(result.is_ok());

        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_with_bearer_token() -> Result<()> {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/webhook")
            .with_status(200)
            .match_header("Authorization", "Bearer test_token")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "event": "metrics.updated",
                "data": {
                    "pool_id": 1,
                    "metric_2": 2.0,
                    "test_metric": 1.0
                },
                "timestamp": "2024-01-01T00:00:00Z"
            })))
            .create();

        let plugin = WebhookPlugin::new(WebhookConfig {
            url: format!("{}/webhook", server.url()),
            bearer_token: Some("test_token".to_string()),
        });
        let mut metrics = std::collections::HashMap::new();
        metrics.insert("test_metric".to_string(), 1.0);
        metrics.insert("metric_2".to_string(), 2.0);
        let result = plugin.send_metrics_updated(1, metrics).await;
        assert!(result.is_ok());
        mock.assert_async().await;
        Ok(())
    }

    #[tokio::test]
    async fn test_webhook_retry_logic() -> Result<()> {
        let mut server = Server::new_async().await;

        // First two attempts fail, third succeeds
        let _mock1 = server
            .mock("POST", "/webhook")
            .with_status(500)
            .expect(1)
            .create();

        let _mock2 = server
            .mock("POST", "/webhook")
            .with_status(502)
            .expect(1)
            .create();

        let _mock3 = server
            .mock("POST", "/webhook")
            .with_status(200)
            .expect(1)
            .create();

        let plugin = WebhookPlugin::new(WebhookConfig {
            url: format!("{}/webhook", server.url()),
            bearer_token: None,
        });

        let mut metrics = std::collections::HashMap::new();
        metrics.insert("test_metric".to_string(), 1.0);
        let result = plugin.send_metrics_updated(1, metrics).await;
        assert!(result.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn test_webhook_max_retries_exceeded() -> Result<()> {
        let mut server = Server::new_async().await;

        // All attempts fail
        let _mock = server
            .mock("POST", "/webhook")
            .with_status(500)
            .expect(6) // 1 initial + 5 retries
            .create();

        let plugin = WebhookPlugin::new(WebhookConfig {
            url: format!("{}/webhook", server.url()),
            bearer_token: None,
        });

        let mut metrics = std::collections::HashMap::new();
        metrics.insert("test_metric".to_string(), 1.0);
        let result = plugin.send_metrics_updated(1, metrics).await;
        assert!(result.is_err());
        Ok(())
    }
}
