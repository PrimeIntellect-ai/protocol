use alloy::primitives::U256;
use anyhow::{Context, Error};
use directories::ProjectDirs;
use hex;
use log::debug;
use log::{error, info};
use serde::{Deserialize, Serialize};
use shared::utils::google_cloud::resolve_mapping_for_sha;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
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

enum ValidationResult {
    Accept,
    Reject,
    Crashed,
    Pending,
    Unknown,
}

#[derive(Debug)]
pub enum ProcessWorkKeyError {
    /// Error when resolving the original file name for the work key.
    FileNameResolutionError(String),
    /// Error when triggering remote toploc validation.
    ValidationTriggerError(String),
    /// Error when polling for remote toploc validation.
    ValidationPollingError(String),
    /// Error when invalidating work.
    InvalidatingWorkError(String),
    /// Error when processing work key.
    MaxAttemptsReached(String),
}

pub struct SyntheticDataValidator {
    pool_id: U256,
    validator: SyntheticDataWorkValidator,
    prime_network: PrimeNetworkContract,
    last_validation_timestamp: U256,
    state_dir: Option<PathBuf>,
    leviticus_url: String,
    leviticus_token: Option<String>,
    penalty: U256,
    s3_credentials: Option<String>,
    bucket_name: Option<String>,
}

impl Validator for SyntheticDataValidator {
    type Error = anyhow::Error;

    fn name(&self) -> &str {
        "Synthetic Data Validator"
    }
}

impl SyntheticDataValidator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_dir: Option<String>,
        pool_id_str: String,
        validator: SyntheticDataWorkValidator,
        prime_network: PrimeNetworkContract,
        leviticus_url: String,
        leviticus_token: Option<String>,
        penalty: U256,
        s3_credentials: Option<String>,
        bucket_name: Option<String>,
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
            prime_network,
            last_validation_timestamp: last_validation_timestamp.unwrap(),
            state_dir: state_path.clone(),
            leviticus_url,
            leviticus_token,
            penalty,
            s3_credentials,
            bucket_name,
        }
    }

    fn save_state(&self) -> Result<(), Error> {
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

    fn load_state(state_dir: &Path, pool_id: &str) -> Result<Option<PersistedWorkState>, Error> {
        let state_path = state_dir.join(state_filename(pool_id));
        if state_path.exists() {
            let contents = fs::read_to_string(state_path)?;
            let state: PersistedWorkState = toml::from_str(&contents)?;
            return Ok(Some(state));
        }
        Ok(None)
    }

    pub async fn invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        let data = hex::decode(work_key)
            .map_err(|e| Error::msg(format!("Failed to decode hex work key: {}", e)))?;
        println!("Invalidating work: {}", work_key);
        match self
            .prime_network
            .invalidate_work(self.pool_id, self.penalty, data)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to invalidate work {}: {}", work_key, e);
                Err(Error::msg(format!("Failed to invalidate work: {}", e)))
            }
        }
    }

    async fn trigger_remote_toploc_validation(
        &self,
        file_name: &str,
        sha: &str,
    ) -> Result<(), Error> {
        let validate_url = format!("{}/validate/{}", self.leviticus_url, file_name);
        info!(
            "Triggering remote toploc validation for {} {}",
            file_name, validate_url
        );

        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                if let Some(token) = &self.leviticus_token {
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                            .expect("Invalid token"),
                    );
                }
                headers
            })
            .build()
            .expect("Failed to build HTTP client");

        let body = serde_json::json!({
            "file_sha": sha
        });

        match client.post(&validate_url).json(&body).send().await {
            Ok(_) => Ok(()),
            Err(e) => {
                error!(
                    "Failed to trigger remote toploc validation for {}: {}",
                    file_name, e
                );
                Err(Error::msg(format!(
                    "Failed to trigger remote toploc validation: {}",
                    e
                )))
            }
        }
    }
    async fn poll_remote_toploc_validation(
        &self,
        file_name: &str,
    ) -> Result<ValidationResult, Error> {
        let url = format!("{}/status/{}", self.leviticus_url, file_name);
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                if let Some(token) = &self.leviticus_token {
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token))
                            .expect("Invalid token"),
                    );
                }
                headers
            })
            .build()
            .expect("Failed to build HTTP client");

        match client.get(&url).send().await {
            Ok(response) => {
                if response.status() != reqwest::StatusCode::OK {
                    error!(
                        "Unexpected status code {} for {}",
                        response.status(),
                        file_name
                    );
                    return Err(Error::msg(format!(
                        "Unexpected status code: {}",
                        response.status()
                    )));
                }
                let status_json: serde_json::Value = response.json().await.map_err(|e| {
                    error!("Failed to parse JSON response for {}: {}", file_name, e);
                    Error::msg(format!("Failed to parse JSON response: {}", e))
                })?;

                if status_json.get("status").is_none() {
                    error!("No status found for {}", file_name);
                    Err(Error::msg("No status found"))
                } else {
                    match status_json.get("status").and_then(|s| s.as_str()) {
                        Some(status) => {
                            info!("Validation status for {}: {}", file_name, status);

                            let validation_result = match status {
                                "accept" => ValidationResult::Accept,
                                "reject" => {
                                    if let Err(e) = self.invalidate_work(file_name).await {
                                        error!("Failed to invalidate work {}: {}", file_name, e);
                                    } else {
                                        info!("Successfully invalidated work {}", file_name);
                                    }
                                    ValidationResult::Reject
                                }
                                "crashed" => ValidationResult::Crashed,
                                "pending" => ValidationResult::Pending,
                                _ => ValidationResult::Unknown,
                            };
                            Ok(validation_result)
                        }
                        None => {
                            error!("No status found for {}", file_name);
                            Err(Error::msg("No status found"))
                        }
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to poll remote toploc validation for {}: {}",
                    file_name, e
                );
                Err(Error::msg(format!(
                    "Failed to poll remote toploc validation: {}",
                    e
                )))
            }
        }
    }

    async fn process_workkey(&mut self, work_key: &str) -> Result<(), ProcessWorkKeyError> {
        let original_file_name = resolve_mapping_for_sha(
            self.bucket_name.clone().unwrap().as_str(),
            &self.s3_credentials.clone().unwrap(),
            work_key,
        )
        .await
        .map_err(|e| ProcessWorkKeyError::FileNameResolutionError(e.to_string()))?;

        if original_file_name.is_empty() {
            error!(
                "Failed to resolve original file name for work key: {}",
                work_key
            );
            return Err(ProcessWorkKeyError::FileNameResolutionError(format!(
                "Failed to resolve original file name for work key: {}",
                work_key
            )));
        }

        let cleaned_file_name = original_file_name
            .strip_prefix('/')
            .unwrap_or(original_file_name.as_str());

        // Trigger remote toploc validation
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 3;
        const BACKOFF_FACTOR: u64 = 2;

        while attempts < MAX_ATTEMPTS {
            if let Err(e) = self
                .trigger_remote_toploc_validation(cleaned_file_name, work_key)
                .await
            {
                attempts += 1;
                let backoff_time = BACKOFF_FACTOR.pow(attempts) * 100; // Exponential backoff in milliseconds
                error!("Failed to trigger remote toploc validation for {}: {}. Attempt {}/{}. Retrying in {} ms...", cleaned_file_name, e, attempts, MAX_ATTEMPTS, backoff_time);
                tokio::time::sleep(std::time::Duration::from_millis(backoff_time)).await;
                continue;
            }
            break; // Success, exit the loop
        }

        if attempts == MAX_ATTEMPTS {
            return Err(ProcessWorkKeyError::ValidationTriggerError(format!(
                "Failed to trigger remote toploc validation for {}",
                cleaned_file_name
            )));
        }

        let max_attempts = 5;
        let mut attempts = 0;
        while attempts < max_attempts {
            if attempts > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            let result = self.poll_remote_toploc_validation(cleaned_file_name).await;
            if let Err(e) = result {
                error!(
                    "Failed to poll remote toploc validation for {}: {}",
                    cleaned_file_name, e
                );
                if attempts == max_attempts {
                    return Err(ProcessWorkKeyError::ValidationPollingError(format!(
                        "Failed to poll remote toploc validation for {}",
                        cleaned_file_name
                    )));
                }
                attempts += 1;
                continue;
            }

            let validation_result = result.unwrap();
            match validation_result {
                ValidationResult::Accept => {
                    info!("Validation accepted for {}", cleaned_file_name);
                    return Ok(());
                }
                ValidationResult::Reject => {
                    info!("Validation rejected for {}", cleaned_file_name);
                    if let Err(e) = self.invalidate_work(work_key).await {
                        error!("Failed to invalidate work {}: {}", work_key, e);
                        attempts += 1;
                        if attempts == max_attempts {
                            return Err(ProcessWorkKeyError::InvalidatingWorkError(format!(
                                "Failed to invalidate work {}",
                                work_key
                            )));
                        }
                        continue;
                    }
                    return Ok(());
                }
                _ => {
                    attempts += 1;
                    continue;
                }
            }
        }
        Err(ProcessWorkKeyError::MaxAttemptsReached(format!(
            "Failed to poll remote toploc validation for {}",
            cleaned_file_name
        )))
    }

    pub async fn validate_work(&mut self) -> Result<(), Error> {
        debug!("Validating work for pool ID: {:?}", self.pool_id);

        // Get all work keys for the pool
        let work_keys = self
            .validator
            .get_work_since(self.pool_id, self.last_validation_timestamp)
            .await
            .context("Failed to get work keys")?;

        debug!("Found {} work keys to validate", work_keys.len());

        let mut completed_all_validations = true;

        // Process each work key
        for work_key in work_keys {
            info!("Processing work key: {}", work_key);
            if let Err(e) = self.process_workkey(&work_key).await {
                // TODO: We have an error now - should we actually skip this validation then?
                error!("Work Validation error: {:?}", e);
                completed_all_validations = false;
            }
        }

        if completed_all_validations {
            self.last_validation_timestamp = U256::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .context("Failed to get current timestamp")?
                    .as_secs(),
            );
        }

        self.save_state()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    #[tokio::test]
    async fn test_process_workkey() {
        // TODO: This requires a leviticus server or a mock server
    }
}
