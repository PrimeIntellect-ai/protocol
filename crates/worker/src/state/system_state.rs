use anyhow::Result;
use directories::ProjectDirs;
use log::debug;
use log::error;
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

#[derive(Debug, Clone)]
struct PersistedSystemState {
    endpoint: Option<String>,
    p2p_keypair: p2p::Keypair,
}

impl Serialize for PersistedSystemState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serde_json::to_string(self)
            .map_err(serde::ser::Error::custom)
            .and_then(|s| serializer.serialize_str(&s))
    }
}

impl<'de> Deserialize<'de> for PersistedSystemState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        serde_json::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SystemState {
    last_heartbeat: Arc<RwLock<Option<std::time::Instant>>>,
    is_running: Arc<RwLock<bool>>, // Keep is_running in the normal heartbeat state
    endpoint: Arc<RwLock<Option<String>>>,
    state_dir_overwrite: Option<PathBuf>,
    disable_state_storing: bool,
    compute_pool_id: u32,
    p2p_keypair: p2p::Keypair,
}

impl SystemState {
    pub(crate) fn new(
        state_dir: Option<String>,
        disable_state_storing: bool,
        compute_pool_id: u32,
    ) -> Self {
        let default_state_dir = get_default_state_dir();
        debug!("Default state dir: {default_state_dir:?}");
        let state_path = state_dir
            .map(PathBuf::from)
            .or_else(|| default_state_dir.map(PathBuf::from));
        debug!("State path: {state_path:?}");

        let mut endpoint = None;
        let mut p2p_keypair = None;

        // Try to load state, log info if creating new file
        if !disable_state_storing {
            if let Some(path) = &state_path {
                let state_file = path.join(STATE_FILENAME);
                if !state_file.exists() {
                    debug!(
                        "No state file found at {state_file:?}, will create on first state change"
                    );
                } else if let Ok(Some(loaded_state)) = SystemState::load_state(path) {
                    debug!("Loaded previous state from {state_file:?}");
                    endpoint = loaded_state.endpoint;
                    p2p_keypair = Some(loaded_state.p2p_keypair);
                } else {
                    debug!("Failed to load state from {state_file:?}");
                }
            }
        }

        if p2p_keypair.is_none() {
            p2p_keypair = Some(p2p::Keypair::generate_ed25519());
        }

        Self {
            last_heartbeat: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            endpoint: Arc::new(RwLock::new(endpoint)),
            state_dir_overwrite: state_path.clone(),
            disable_state_storing,
            compute_pool_id,
            p2p_keypair: p2p_keypair.expect("p2p keypair must be Some at this point"),
        }
    }

    fn save_state(&self, heartbeat_endpoint: Option<String>) -> Result<()> {
        if !self.disable_state_storing {
            debug!("Saving state");
            if let Some(state_dir) = &self.state_dir_overwrite {
                let state = PersistedSystemState {
                    endpoint: heartbeat_endpoint,
                    p2p_keypair: self.p2p_keypair.clone(),
                };

                debug!("state: {state:?}");

                fs::create_dir_all(state_dir)?;
                let state_path = state_dir.join(STATE_FILENAME);

                // Use JSON serialization instead of TOML
                match serde_json::to_string_pretty(&state) {
                    Ok(json_string) => {
                        fs::write(&state_path, json_string)?;
                        debug!("Saved state to {state_path:?}");
                    }
                    Err(e) => {
                        error!("Failed to serialize state: {e}");
                        return Err(anyhow::anyhow!("Failed to serialize state: {}", e));
                    }
                }
            }
        }
        Ok(())
    }

    fn load_state(state_dir: &Path) -> Result<Option<PersistedSystemState>> {
        let state_path = state_dir.join(STATE_FILENAME);
        if state_path.exists() {
            let contents = fs::read_to_string(state_path)?;
            match serde_json::from_str(&contents) {
                Ok(state) => return Ok(Some(state)),
                Err(e) => {
                    debug!("Error parsing state file: {e}");
                    return Ok(None);
                }
            }
        }
        Ok(None)
    }

    pub(crate) fn get_compute_pool_id(&self) -> u32 {
        self.compute_pool_id
    }

    pub(crate) fn get_p2p_keypair(&self) -> &p2p::Keypair {
        &self.p2p_keypair
    }

    pub(crate) fn get_p2p_id(&self) -> p2p::PeerId {
        self.p2p_keypair.public().to_peer_id()
    }

    pub(crate) async fn update_last_heartbeat(&self) {
        let mut heartbeat = self.last_heartbeat.write().await;
        *heartbeat = Some(std::time::Instant::now());
    }

    pub(crate) async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    pub(crate) async fn set_running(
        &self,
        running: bool,
        heartbeat_endpoint: Option<String>,
    ) -> Result<()> {
        // Read current values
        let current_running = self.is_running().await;
        let current_endpoint = self.get_heartbeat_endpoint().await;

        // Only update and save if values changed
        if running != current_running || heartbeat_endpoint != current_endpoint {
            let mut is_running = self.is_running.write().await;
            let mut endpoint = self.endpoint.write().await;
            *is_running = running;

            if !running {
                *endpoint = None;
            } else {
                *endpoint = heartbeat_endpoint;
            }

            if endpoint.is_some() {
                if let Err(e) = self.save_state(endpoint.clone()) {
                    // Only save the endpoint
                    error!("Failed to save heartbeat state: {e}");
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn get_heartbeat_endpoint(&self) -> Option<String> {
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
        assert!(default_state_dir.is_some());
    }

    #[tokio::test]
    async fn test_new_state_dir() {
        let temp_dir = setup_test_dir();

        let state = SystemState::new(
            Some(temp_dir.path().to_string_lossy().to_string()),
            false,
            0,
        );
        let _ = state
            .set_running(true, Some("http://localhost:8080/heartbeat".to_string()))
            .await;

        let state_file = temp_dir.path().join(STATE_FILENAME);
        assert!(state_file.exists());

        let contents = fs::read_to_string(state_file).expect("Failed to read state file");
        let state: PersistedSystemState =
            serde_json::from_str(&contents).expect("Failed to parse state file");
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

        let state = SystemState::new(
            Some(temp_dir.path().to_string_lossy().to_string()),
            false,
            0,
        );
        assert!(!(state.is_running().await));
        assert_eq!(state.get_heartbeat_endpoint().await, None);
    }

    #[tokio::test]
    async fn test_load_state() {
        let temp_dir = setup_test_dir();
        let state_file = temp_dir.path().join(STATE_FILENAME);
        fs::write(
            &state_file,
            r#"{"endpoint": "http://localhost:8080/heartbeat"}"#,
        )
        .expect("Failed to write to state file");

        let state = SystemState::new(
            Some(temp_dir.path().to_string_lossy().to_string()),
            false,
            0,
        );
        assert_eq!(
            state.get_heartbeat_endpoint().await,
            Some("http://localhost:8080/heartbeat".to_string())
        );
    }
}
