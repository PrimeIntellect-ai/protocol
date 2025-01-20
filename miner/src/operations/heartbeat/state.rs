use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct HeartbeatState {
    last_heartbeat: Arc<RwLock<Option<std::time::Instant>>>,
    is_running: Arc<RwLock<bool>>,
    endpoint: Arc<RwLock<Option<String>>>,
}

impl HeartbeatState {
    pub fn new() -> Self {
        Self {
            last_heartbeat: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            endpoint: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn update_last_heartbeat(&self) {
        let mut heartbeat = self.last_heartbeat.write().await;
        *heartbeat = Some(std::time::Instant::now());
    }

    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    pub async fn set_running(&self, running: bool, heartbeat_endpoint: Option<String>) {
        let mut is_running = self.is_running.write().await;
        let mut endpoint = self.endpoint.write().await;
        *is_running = running;
        *endpoint = heartbeat_endpoint;
    }

    pub async fn get_endpoint(&self) -> Option<String> {
        let endpoint = self.endpoint.read().await;
        endpoint.clone()
    }
}
