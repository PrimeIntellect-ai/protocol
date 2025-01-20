use super::state::HeartbeatState;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::{interval, Duration};
type HeartbeatResult<T> = Result<T, String>;

#[derive(Debug, Serialize)]
struct HeartbeatRequest {}

#[derive(Debug, Deserialize)]
struct HeartbeatResponse {
    success: bool,
}

#[derive(Clone)]
pub struct HeartbeatService {
    state: HeartbeatState,
    interval: Duration,
    client: Client,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum HeartbeatError {
    #[error("HTTP request failed")]
    RequestFailed,
    #[error("Service initialization failed")]
    InitFailed,
}
impl HeartbeatService {
    pub fn new(interval: Duration, state_dir: Option<String>) -> Result<Arc<Self>, HeartbeatError> {
        let state: HeartbeatState;
        if state_dir.is_some() {
            state = HeartbeatState::new(state_dir);
        }
        else {
            state = HeartbeatState::new(None);
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(5)) // 5 second timeout
            .build()
            .map_err(|_| HeartbeatError::InitFailed)?; // Adjusted to match the expected error type

        Ok(Arc::new(Self {
            state,
            interval,
            client,
        }))
    }

    pub async fn start(&self, endpoint: String) -> Result<(), HeartbeatError> {
        if self.state.is_running().await {
            return Ok(());
        }

        self.state.set_running(true, Some(endpoint)).await;
        let state = self.state.clone();
        let client = self.client.clone();
        let interval_duration = self.interval;

        tokio::spawn(async move {
            let mut interval = interval(interval_duration);

            while state.is_running().await {
                interval.tick().await;

                match Self::send_heartbeat(&client, state.get_endpoint().await).await {
                    Ok(_) => {
                        state.update_last_heartbeat().await;
                        log::debug!("Heartbeat sent successfully");
                    }
                    Err(e) => {
                        log::error!("Heartbeat failed: {:?}", e);
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn stop(&self) {
        self.state.set_running(false, None).await;
    }

    async fn send_heartbeat(
        client: &Client,
        endpoint: Option<String>,
    ) -> Result<HeartbeatResponse, HeartbeatError> {
        if endpoint.is_none() {
            return Err(HeartbeatError::RequestFailed);
        }

        let request = HeartbeatRequest {};
        let response = client
            .post(endpoint.unwrap())
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                log::error!("Request failed: {:?}", e);
                HeartbeatError::RequestFailed
            })?
            .error_for_status()
            .map_err(|e| {
                log::error!("Error response received: {:?}", e);
                HeartbeatError::RequestFailed
            })?
            .json::<HeartbeatResponse>()
            .await
            .map_err(|e| {
                log::error!("Failed to parse response: {:?}", e);
                HeartbeatError::RequestFailed
            })?;

        Ok(response)
    }
}
