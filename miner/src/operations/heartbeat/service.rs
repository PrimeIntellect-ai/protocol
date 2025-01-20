use super::state::HeartbeatState;
use crate::TaskHandles;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Serialize)]
struct HeartbeatRequest {}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct HeartbeatResponse {
    success: bool,
}

#[derive(Clone)]
pub struct HeartbeatService {
    state: HeartbeatState,
    interval: Duration,
    client: Client,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum HeartbeatError {
    #[error("HTTP request failed")]
    RequestFailed,
    #[error("Service initialization failed")]
    InitFailed,
}
impl HeartbeatService {
    pub fn new(
        interval: Duration,
        state_dir: Option<String>,
        cancellation_token: CancellationToken,
        task_handles: TaskHandles,
    ) -> Result<Arc<Self>, HeartbeatError> {
        let state: HeartbeatState = if state_dir.is_some() {
            HeartbeatState::new(state_dir)
        } else {
            HeartbeatState::new(None)
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(5)) // 5 second timeout
            .build()
            .map_err(|_| HeartbeatError::InitFailed)?; // Adjusted to match the expected error type

        Ok(Arc::new(Self {
            state,
            interval,
            client,
            cancellation_token,
            task_handles,
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
        let cancellation_token = self.cancellation_token.clone();
        let handle = tokio::spawn(async move {
            let mut interval = interval(interval_duration);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !state.is_running().await {
                            break;
                        }
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
                    _ = cancellation_token.cancelled() => {
                        log::info!("Heartbeat service received cancellation signal");
                        state.set_running(false, None).await;
                        break;
                    }
                }
            }
            log::info!("Heartbeat service stopped");
        });

        let mut task_handles = self.task_handles.lock().await;
        task_handles.push(handle);

        Ok(())
    }

    #[allow(dead_code)]
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
