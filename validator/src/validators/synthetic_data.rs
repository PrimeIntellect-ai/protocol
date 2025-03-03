use alloy::primitives::U256;
use anyhow::{Context, Result};
use directories::ProjectDirs;
use log::debug;
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use toml;

use crate::validators::Validator;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::SyntheticDataWorkValidator;

fn get_default_state_dir() -> Option<String> {
    ProjectDirs::from("com", "prime", "validator")
        .map(|proj_dirs| proj_dirs.data_local_dir().to_string_lossy().into_owned())
}

fn state_filename(pool_id: &str) -> String {
    format!("work_state_{}.toml", pool_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedWorkState {
    pool_id: U256,
    last_validation_timestamp: U256,
}

pub struct SyntheticDataValidator {
    pool_id: U256,
    validator: SyntheticDataWorkValidator,
    last_validation_timestamp: U256,
    state_dir: Option<PathBuf>,
}

impl Validator for SyntheticDataValidator {
    type Error = anyhow::Error;

    fn name(&self) -> &str {
        "Synthetic Data Validator"
    }
}

impl SyntheticDataValidator {
    pub fn new(
        state_dir: Option<String>,
        pool_id_str: String,
        validator: SyntheticDataWorkValidator,
    ) -> Self {
        let pool_id = pool_id_str.parse::<U256>().expect("Invalid pool ID");
        let default_state_dir = get_default_state_dir();
        debug!("Default state dir: {:?}", default_state_dir);
        let state_path = state_dir
            .map(PathBuf::from)
            .or_else(|| default_state_dir.map(PathBuf::from));
        debug!("State path: {:?}", state_path);
        let mut last_validation_timestamp: Option<_> = None;

        // Try to load state, log info if creating new file
        if let Some(path) = &state_path {
            let state_file = path.join(state_filename(&pool_id.to_string()));
            if !state_file.exists() {
                debug!(
                    "No state file found at {:?}, will create on first state change",
                    state_file
                );
            } else if let Ok(Some(loaded_state)) =
                SyntheticDataValidator::load_state(path, &pool_id.to_string())
            {
                debug!("Loaded previous state from {:?}", state_file);
                last_validation_timestamp = Some(loaded_state.last_validation_timestamp);
            } else {
                debug!("Failed to load state from {:?}", state_file);
            }
        }

        // if no last time, set it to 24 hours ago, as nothing before that can be invalidated
        if last_validation_timestamp.is_none() {
            last_validation_timestamp = Some(U256::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("Failed to get current timestamp")
                    .as_secs()
                    .saturating_sub(24 * 60 * 60),
            ));
        }

        Self {
            pool_id,
            validator,
            last_validation_timestamp: last_validation_timestamp.unwrap(),
            state_dir: state_path.clone(),
        }
    }

    fn save_state(&self) -> Result<()> {
        // Get values without block_on
        let state = PersistedWorkState {
            pool_id: self.pool_id,
            last_validation_timestamp: self.last_validation_timestamp,
        };

        if let Some(ref state_dir) = self.state_dir {
            fs::create_dir_all(state_dir)?;
            let state_path = state_dir.join(state_filename(&self.pool_id.to_string()));
            let toml = toml::to_string_pretty(&state)?;
            fs::write(&state_path, toml)?;
            debug!("Saved state to {:?}", state_path);
        }
        Ok(())
    }

    fn load_state(state_dir: &Path, pool_id: &str) -> Result<Option<PersistedWorkState>> {
        let state_path = state_dir.join(state_filename(pool_id));
        if state_path.exists() {
            let contents = fs::read_to_string(state_path)?;
            let state: PersistedWorkState = toml::from_str(&contents)?;
            return Ok(Some(state));
        }
        Ok(None)
    }

    pub async fn validate_work(&mut self) -> Result<()> {
        info!("Validating work for pool ID: {:?}", self.pool_id);

        // Get all work keys for the pool
        let work_keys = self
            .validator
            .get_work_since(self.pool_id, self.last_validation_timestamp)
            .await
            .context("Failed to get work keys")?;

        info!("Found {} work keys to validate", work_keys.len());

        // Process each work key
        for work_key in work_keys {
            info!("Processing work key: {}", work_key);

            match self.validator.get_work_info(self.pool_id, &work_key).await {
                Ok(work_info) => {
                    info!(
                        "Validated work info - Provider: {:?}, Node: {:?}, Timestamp: {}",
                        work_info.provider, work_info.node_id, work_info.timestamp
                    );
                    // TODO: Add actual validation logic based on work_info
                }
                Err(e) => {
                    error!("Failed to get work info for key {}: {}", work_key, e);
                    continue;
                }
            }
        }

        // Update last validation timestamp to current time
        // TODO: We should only set this once we are sure that we have validated all keys
        self.last_validation_timestamp = U256::from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("Failed to get current timestamp")?
                .as_secs(),
        );

        self.save_state()?;

        Ok(())
    }
}
