use super::state::HeartbeatState;
use crate::docker::DockerService;
use crate::TaskHandles;
use reqwest::Client;
use shared::models::api::ApiResponse;
use shared::models::heartbeat::{HeartbeatRequest, HeartbeatResponse};
use shared::security::request_signer::sign_request;
use shared::web3::wallet::Wallet;
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct HeartbeatService {
    state: HeartbeatState,
    interval: Duration,
    client: Client,
    cancellation_token: CancellationToken,
    task_handles: TaskHandles,
    node_wallet: Arc<Wallet>,
    docker_service: Arc<DockerService>,
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
        state_dir_overwrite: Option<String>,
        disable_state_storing: bool,
        cancellation_token: CancellationToken,
        task_handles: TaskHandles,
        node_wallet: Arc<Wallet>,
        docker_service: Arc<DockerService>,
    ) -> Result<Arc<Self>, HeartbeatError> {
        let state = HeartbeatState::new(state_dir_overwrite.or(None), disable_state_storing);

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
            node_wallet,
            docker_service,
        }))
    }

    pub async fn activate_heartbeat_if_endpoint_exists(&self) {
        println!("Activating heartbeat if endpoint exists");
        println!("Endpoint: {:?}", self.state.get_endpoint().await);
        if let Some(endpoint) = self.state.get_endpoint().await {
            println!("Starting heartbeat with endpoint: {:?}", endpoint);
            self.start(endpoint).await.unwrap();
        }
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
        let docker_service = self.docker_service.clone();
        let wallet_clone = self.node_wallet.clone();
        let handle = tokio::spawn(async move {
            let mut interval = interval(interval_duration);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if !state.is_running().await {
                            break;
                        }
                        match Self::send_heartbeat(&client, state.get_endpoint().await, wallet_clone.clone(), docker_service.clone()).await {
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
        wallet: Arc<Wallet>,
        docker_service: Arc<DockerService>,
    ) -> Result<HeartbeatResponse, HeartbeatError> {
        if endpoint.is_none() {
            return Err(HeartbeatError::RequestFailed);
        }

        let current_task_state = docker_service.state.get_current_task().await;
        let request = if let Some(task) = current_task_state {
            HeartbeatRequest {
                address: wallet.address().to_string(),
                task_id: Some(task.id.to_string()),
                task_state: Some(task.state.to_string()),
            }
        } else {
            HeartbeatRequest {
                address: wallet.address().to_string(),
                task_id: None,
                task_state: None,
            }
        };

        let signature = sign_request(
            "/heartbeat",
            &wallet,
            Some(&serde_json::to_value(&request).unwrap()),
        )
        .await
        .unwrap();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-address", wallet.address().to_string().parse().unwrap());
        headers.insert("x-signature", signature.parse().unwrap());

        let response = client
            .post(endpoint.unwrap())
            .json(&request)
            .headers(headers)
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
            .json::<ApiResponse<HeartbeatResponse>>()
            .await
            .map_err(|e| {
                log::error!("Failed to parse response: {:?}", e);
                HeartbeatError::RequestFailed
            })?;

        let heartbeat_response = response.data.clone();
        log::info!("Heartbeat response: {:?}", heartbeat_response);
        let response_clone = response.clone();
        println!("Current response: {:?}", response_clone);
        let task = match heartbeat_response.current_task {
            Some(task) => {
                println!("Current task is to run image: {:?}", task.image);
                Some(task)
            }
            None => {
                log::info!("No current task found in heartbeat response");
                None
            }
        };

        docker_service.state.set_current_task(task).await;

        Ok(response.data)
    }
}
