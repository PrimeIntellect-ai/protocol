use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const STATE_FILENAME: &str = "heartbeat_state.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedHeartbeatState {
    is_running: bool,
    endpoint: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HeartbeatState {
    last_heartbeat: Arc<RwLock<Option<std::time::Instant>>>,
    is_running: Arc<RwLock<bool>>,
    endpoint: Arc<RwLock<Option<String>>>,
    state_dir: Option<PathBuf>,
}

impl HeartbeatState {
    pub fn new(state_dir: Option<String>) -> Self {
        let state_path = state_dir.map(PathBuf::from);
        let state = Self {
            last_heartbeat: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            endpoint: Arc::new(RwLock::new(None)),
            state_dir: state_path.clone(),
        };

        // Try to load state, log info if creating new file
        if let Some(path) = &state_path {
            let state_file = path.join(STATE_FILENAME);
            if !state_file.exists() {
                println!(
                    "No state file found at {:?}, will create on first state change",
                    state_file
                );
            } else if let Ok(Some(loaded_state)) = state.load_state() {
                println!("Loaded previous state from {:?}", state_file);
                let is_running = loaded_state.is_running;
                let endpoint = loaded_state.endpoint;
                // Initialize state directly without block_on
                *state.is_running.blocking_write() = is_running;
                *state.endpoint.blocking_write() = endpoint;
            }
        }

        state
    }

    fn save_state(&self) -> Result<()> {
        if let Some(state_dir) = &self.state_dir {
            // Get values without block_on
            let is_running = *self.is_running.blocking_read();
            let endpoint = self.endpoint.blocking_read().clone();

            let state = PersistedHeartbeatState {
                is_running,
                endpoint,
            };

            fs::create_dir_all(state_dir)?;
            let state_path = state_dir.join(STATE_FILENAME);
            let toml = toml::to_string_pretty(&state)?;
            fs::write(&state_path, toml)?;
            println!("Saved state to {:?}", state_path);
        }
        Ok(())
    }

    fn load_state(&self) -> Result<Option<PersistedHeartbeatState>> {
        if let Some(state_dir) = &self.state_dir {
            let state_path = state_dir.join(STATE_FILENAME);
            if state_path.exists() {
                let contents = fs::read_to_string(state_path)?;
                let state: PersistedHeartbeatState = toml::from_str(&contents)?;
                return Ok(Some(state));
            }
        }
        Ok(None)
    }

    pub async fn update_last_heartbeat(&self) {
        let mut heartbeat = self.last_heartbeat.write().await;
        *heartbeat = Some(std::time::Instant::now());
    }

    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    pub async fn set_running(&self, running: bool, heartbeat_endpoint: Option<String>) {
        // Read current values
        let current_running = self.is_running().await;
        let current_endpoint = self.get_endpoint().await;

        // Only update and save if values changed
        if running != current_running || heartbeat_endpoint != current_endpoint {
            let mut is_running = self.is_running.write().await;
            let mut endpoint = self.endpoint.write().await;
            *is_running = running;
            *endpoint = heartbeat_endpoint;

            if let Err(e) = self.save_state() {
                eprintln!("Failed to save heartbeat state: {}", e);
            }
        }
    }

    pub async fn get_endpoint(&self) -> Option<String> {
        let endpoint = self.endpoint.read().await;
        endpoint.clone()
    }
}
