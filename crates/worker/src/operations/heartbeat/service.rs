use crate::docker::DockerService;
use crate::metrics::store::MetricsStore;
use crate::state::system_state::SystemState;
use crate::TaskHandles;
use log::info;
use reqwest::Client;
use shared::models::api::ApiResponse;
use shared::models::heartbeat::{HeartbeatRequest, HeartbeatResponse};
use shared::security::request_signer::sign_request_with_nonce;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct HeartbeatService {
    state: Arc<SystemState>,
    interval: Duration,
    client: Client,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
    node_wallet: Wallet,
    docker_service: Arc<DockerService>,
    metrics_store: Arc<MetricsStore>,
}
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum HeartbeatError {
    #[error("HTTP request failed")]
    RequestFailed,
    #[error("Service initialization failed")]
    InitFailed,
}

impl HeartbeatService {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        interval: Duration,
        cancellation_token: CancellationToken,
        task_handles: TaskHandles,
        node_wallet: Wallet,
        docker_service: Arc<DockerService>,
        metrics_store: Arc<MetricsStore>,
        state: Arc<SystemState>,
    ) -> Result<Arc<Self>, HeartbeatError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| HeartbeatError::InitFailed)?; // Adjusted to match the expected error type

        Ok(Arc::new(Self {
            state,
            interval,
            client,
            cancellation_token,
            task_handles,
            node_wallet,
            docker_service,
            metrics_store,
        }))
    }

    pub(crate) async fn activate_heartbeat_if_endpoint_exists(&self) {
        if let Some(endpoint) = self.state.get_heartbeat_endpoint().await {
            info!("Starting heartbeat from recovered state");
            self.start(endpoint).await.unwrap();
        }
    }

    pub(crate) async fn start(&self, endpoint: String) -> Result<(), HeartbeatError> {
        if self.state.is_running().await {
            return Ok(());
        }

        if let Err(e) = self.state.set_running(true, Some(endpoint)).await {
            log::error!("Failed to set running to true: {e:?}");
        }
        let state = self.state.clone();
        let client = self.client.clone();
        let interval_duration = self.interval;
        let cancellation_token = self.cancellation_token.clone();
        let docker_service = self.docker_service.clone();
        let wallet_clone = self.node_wallet.clone();
        let metrics_store = self.metrics_store.clone();
        let handle = tokio::spawn(async move {
            let mut interval = interval(interval_duration);
            let mut had_error = false;
            let mut first_start = true;
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !state.is_running().await {
                            break;
                        }
                        match send_heartbeat(&client, state.get_heartbeat_endpoint().await, wallet_clone.clone(), docker_service.clone(), metrics_store.clone(), state.get_p2p_id()).await {
                            Ok(_) => {
                                state.update_last_heartbeat().await;
                                if had_error {
                                    log::info!("Orchestrator sync restored - connection is healthy again");
                                    had_error = false;
                                } else if first_start {
                                    log::info!("Successfully connected to orchestrator");
                                    first_start = false;
                                } else {
                                    log::debug!("Synced with orchestrator");
                                }
                            }
                            Err(e) => {
                                log::error!("{}", &format!("Failed to sync with orchestrator: {e:?}"));
                                had_error = true;
                            }
                        }
                    }
                    _ = cancellation_token.cancelled() => {
                        log::info!("Sync service received cancellation signal"); // Updated log message
                        if let Err(e) = state.set_running(false, None).await {
                            log::error!("Failed to set running to false: {e:?}");
                        }
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
    pub(crate) async fn stop(&self) {
        if let Err(e) = self.state.set_running(false, None).await {
            log::error!("Failed to set running to false: {e:?}");
        }
    }
}

async fn send_heartbeat(
    client: &Client,
    endpoint: Option<String>,
    wallet: Wallet,
    docker_service: Arc<DockerService>,
    metrics_store: Arc<MetricsStore>,
    p2p_id: p2p::PeerId,
) -> Result<HeartbeatResponse, HeartbeatError> {
    if endpoint.is_none() {
        return Err(HeartbeatError::RequestFailed);
    }

    let current_task_state = docker_service.state.get_current_task().await;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let task_details = if let Some(task) = &current_task_state {
        docker_service.get_task_details(task).await
    } else {
        None
    };

    let request = if let Some(task) = current_task_state {
        let metrics_for_task = metrics_store
            .get_metrics_for_task(task.id.to_string())
            .await;
        HeartbeatRequest {
            address: wallet.address().to_string(),
            task_id: Some(task.id.to_string()),
            task_state: Some(task.state.to_string()),
            metrics: Some(metrics_for_task),
            version: Some(
                option_env!("WORKER_VERSION")
                    .unwrap_or(env!("CARGO_PKG_VERSION"))
                    .to_string(),
            ),
            timestamp: Some(ts),
            p2p_id: Some(p2p_id.to_string()), // TODO: this should always be `Some`
            task_details,
        }
    } else {
        HeartbeatRequest {
            address: wallet.address().to_string(),
            version: Some(
                option_env!("WORKER_VERSION")
                    .unwrap_or(env!("CARGO_PKG_VERSION"))
                    .to_string(),
            ),
            timestamp: Some(ts),
            p2p_id: Some(p2p_id.to_string()), // TODO: this should always be `Some`
            ..Default::default()
        }
    };

    let signature = sign_request_with_nonce(
        "/heartbeat",
        &wallet,
        Some(&serde_json::to_value(&request).map_err(|e| {
            log::error!("Failed to serialize request: {e:?}");
            HeartbeatError::RequestFailed
        })?),
    )
    .await
    .map_err(|e| {
        log::error!("Failed to sign request: {e:?}");
        HeartbeatError::RequestFailed
    })?;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-address", wallet.address().to_string().parse().unwrap());
    headers.insert("x-signature", signature.signature.parse().unwrap());

    let response = client
        .post(endpoint.unwrap())
        .json(&signature.data)
        .headers(headers)
        .send()
        .await
        .map_err(|e| {
            log::error!("Request failed: {e:?}");
            HeartbeatError::RequestFailed
        })?;

    let response = if response.status().is_success() {
        response
    } else {
        let status = response.status();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        log::error!("Error response received: status={status}, body={error_text}");
        return Err(HeartbeatError::RequestFailed);
    };

    let response = response
        .json::<ApiResponse<HeartbeatResponse>>()
        .await
        .map_err(|e| {
            log::error!("Failed to parse response: {e:?}");
            HeartbeatError::RequestFailed
        })?;

    let heartbeat_response = response.data.clone();
    log::debug!("Heartbeat response: {heartbeat_response:?}");

    // Get current task before updating
    let old_task = docker_service.state.get_current_task().await;

    let new_task = match heartbeat_response.current_task {
        Some(task) => {
            // Only log if task image changed or there was no previous task
            if old_task
                .as_ref()
                .map(|t| t.image != task.image)
                .unwrap_or(true)
            {
                log::info!("Current task is to run image: {:?}", task.image);
            }
            Some(task)
        }
        None => None,
    };

    // Clear metrics for old task if task ID changed
    if let (Some(old), Some(new)) = (&old_task, &new_task) {
        if old.id != new.id {
            log::info!("Clearing metrics for old task: {}", old.id);
            metrics_store
                .clear_metrics_for_task(&old.id.to_string())
                .await;
        }
    } else if let (Some(old), None) = (&old_task, &new_task) {
        // Clear metrics when transitioning from having a task to no task
        log::info!("Clearing metrics for completed task: {}", old.id);
        metrics_store
            .clear_metrics_for_task(&old.id.to_string())
            .await;
    }

    docker_service.state.set_current_task(new_task).await;
    let is_running = docker_service.state.get_is_running().await;
    if !is_running {
        tokio::spawn(async move {
            if let Err(e) = docker_service.run().await {
                log::error!("❌ Docker service failed: {e}");
            }
        });
    }

    Ok(response.data)
}
