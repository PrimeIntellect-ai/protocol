use anyhow::Result;
use directories::ProjectDirs;
use log::debug;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

const STATE_FILENAME: &str = "heartbeat_state.toml";

fn get_default_state_dir() -> Option<String> {
    ProjectDirs::from("com", "prime", "worker")
        .map(|proj_dirs| proj_dirs.data_local_dir().to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedHeartbeatState {
    endpoint: Option<String>, // Only store the endpoint
}

#[derive(Debug, Clone)]
pub struct HeartbeatState {
    last_heartbeat: Arc<RwLock<Option<std::time::Instant>>>,
    is_running: Arc<RwLock<bool>>, // Keep is_running in the normal heartbeat state
    endpoint: Arc<RwLock<Option<String>>>,
    state_dir_overwrite: Option<PathBuf>,
    disable_state_storing: bool,
}

impl HeartbeatState {
    pub fn new(state_dir: Option<String>, disable_state_storing: bool) -> Self {
        let default_state_dir = get_default_state_dir();
        debug!("Default state dir: {:?}", default_state_dir);
        let state_path = state_dir
            .map(PathBuf::from)
            .or_else(|| default_state_dir.map(PathBuf::from));
        debug!("State path: {:?}", state_path);
        let mut endpoint = None;

        // Try to load state, log info if creating new file
        if let Some(path) = &state_path {
            let state_file = path.join(STATE_FILENAME);
            if !state_file.exists() {
                debug!(
                    "No state file found at {:?}, will create on first state change",
                    state_file
                );
            } else if let Ok(Some(loaded_state)) = HeartbeatState::load_state(path) {
                debug!("Loaded previous state from {:?}", state_file);
                endpoint = loaded_state.endpoint;
            } else {
                debug!("Failed to load state from {:?}", state_file);
            }
        }

        Self {
            last_heartbeat: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            endpoint: Arc::new(RwLock::new(endpoint)),
            state_dir_overwrite: state_path.clone(),
            disable_state_storing,
        }
    }

    fn save_state(&self, heartbeat_endpoint: Option<String>) -> Result<()> {
        if !self.disable_state_storing {
            if let Some(state_dir) = &self.state_dir_overwrite {
                // Get values without block_on
                let state = PersistedHeartbeatState {
                    endpoint: heartbeat_endpoint,
                };

                fs::create_dir_all(state_dir)?;
                let state_path = state_dir.join(STATE_FILENAME);
                let toml = toml::to_string_pretty(&state)?;
                fs::write(&state_path, toml)?;
                debug!("Saved state to {:?}", state_path);
            }
        }
        Ok(())
    }

    fn load_state(state_dir: &Path) -> Result<Option<PersistedHeartbeatState>> {
        let state_path = state_dir.join(STATE_FILENAME);
        if state_path.exists() {
            let contents = fs::read_to_string(state_path)?;
            let state: PersistedHeartbeatState = toml::from_str(&contents)?;
            return Ok(Some(state));
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

            if endpoint.is_some() {
                if let Err(e) = self.save_state(endpoint.clone()) {
                    // Only save the endpoint
                    eprintln!("Failed to save heartbeat state: {}", e);
                }
            }
        }
    }

    pub async fn get_endpoint(&self) -> Option<String> {
        let endpoint = self.endpoint.read().await;
        endpoint.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        tempfile::tempdir().expect("Failed to create temp directory")
    }

    #[tokio::test]
    async fn test_state_dir_overwrite() {
        let default_state_dir = get_default_state_dir();
        println!("Default state dir: {:?}", default_state_dir);
        assert!(default_state_dir.is_some());
    }

    #[tokio::test]
    async fn test_new_state_dir() {
        let temp_dir = setup_test_dir();
        println!("Temp dir: {:?}", temp_dir.path());

        let state = HeartbeatState::new(Some(temp_dir.path().to_string_lossy().to_string()), false);
        state
            .set_running(true, Some("http://localhost:8080/heartbeat".to_string()))
            .await;

        let state_file = temp_dir.path().join(STATE_FILENAME);
        assert!(state_file.exists());

        let contents = fs::read_to_string(state_file).expect("Failed to read state file");
        let state: PersistedHeartbeatState =
            toml::from_str(&contents).expect("Failed to parse state file");
        assert_eq!(
            state.endpoint,
            Some("http://localhost:8080/heartbeat".to_string())
        );
    }

    #[tokio::test]
    async fn test_corrupt_state_file() {
        let temp_dir = setup_test_dir();
        let state_file = temp_dir.path().join(STATE_FILENAME);
        fs::write(&state_file, "invalid_toml_content").expect("Failed to write to state file");

        let state = HeartbeatState::new(Some(temp_dir.path().to_string_lossy().to_string()), false);
        assert!(!(state.is_running().await));
        assert_eq!(state.get_endpoint().await, None);
    }

    #[tokio::test]
    async fn test_load_state() {
        let temp_dir = setup_test_dir();
        let state_file = temp_dir.path().join(STATE_FILENAME);
        fs::write(
            &state_file,
            "endpoint = \"http://localhost:8080/heartbeat\"",
        )
        .expect("Failed to write to state file");

        let state = HeartbeatState::new(Some(temp_dir.path().to_string_lossy().to_string()), false);
        assert_eq!(
            state.get_endpoint().await,
            Some("http://localhost:8080/heartbeat".to_string())
        );
    }
}
