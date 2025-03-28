use crate::store::redis::RedisStore;
use crate::validators::Validator;
use alloy::primitives::U256;
use anyhow::{Context, Error};
use directories::ProjectDirs;
use hex;
use log::debug;
use log::{error, info};
use redis::Commands;
use serde::{Deserialize, Serialize};
use shared::utils::google_cloud::resolve_mapping_for_sha;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::SyntheticDataWorkValidator;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use toml;

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

#[derive(Debug, Serialize, Deserialize)]
enum ValidationResult {
    Accept,
    Reject,
    Crashed,
    Pending,
    Unknown,
}

use std::fmt;

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
    /// Generic error to encapsulate unexpected errors.
    GenericError(anyhow::Error),
}

impl From<anyhow::Error> for ProcessWorkKeyError {
    fn from(err: anyhow::Error) -> Self {
        ProcessWorkKeyError::GenericError(err)
    }
}

impl fmt::Display for ProcessWorkKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessWorkKeyError::FileNameResolutionError(msg) => {
                write!(f, "File name resolution error: {}", msg)
            }
            ProcessWorkKeyError::ValidationTriggerError(msg) => {
                write!(f, "Validation trigger error: {}", msg)
            }
            ProcessWorkKeyError::ValidationPollingError(msg) => {
                write!(f, "Validation polling error: {}", msg)
            }
            ProcessWorkKeyError::InvalidatingWorkError(msg) => {
                write!(f, "Invalidating work error: {}", msg)
            }
            ProcessWorkKeyError::MaxAttemptsReached(msg) => {
                write!(f, "Max attempts reached: {}", msg)
            }
            ProcessWorkKeyError::GenericError(err) => {
                write!(f, "Generic error: {}", err)
            }
        }
    }
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
    redis_store: RedisStore,
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
        redis_store: RedisStore,
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
            redis_store,
        }
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
        info!("Invalidating work: {}", work_key);
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

    fn get_key_for_work_key(&self, work_key: &str) -> String {
        format!("work_validation_status:{}", work_key)
    }

    async fn update_work_validation_status(
        &self,
        work_key: &str,
        status: &ValidationResult,
    ) -> Result<(), Error> {
        let expiry = match status {
            // Must switch to pending within 60 seconds otherwise we resubmit it
            ValidationResult::Unknown => 60,
            _ => 0,
        };
        let mut con = self.redis_store.client.get_connection().unwrap();
        let key = self.get_key_for_work_key(work_key);
        let status = serde_json::to_string(&status)?;
        let _: () = con
            .set_options(
                &key,
                status,
                redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(expiry)),
            )
            .unwrap();
        Ok(())
    }

    async fn get_work_validation_status_from_redis(
        &self,
        work_key: &str,
    ) -> Option<ValidationResult> {
        let mut con = self.redis_store.client.get_connection().unwrap();
        let key = self.get_key_for_work_key(work_key);
        let status: Option<String> = con.get(key).unwrap();
        match status {
            Some(status) => Some(serde_json::from_str(&status).unwrap()),
            None => None,
        }
    }

    async fn get_file_name_for_work_key(
        &self,
        work_key: &str,
    ) -> Result<String, ProcessWorkKeyError> {
        // TODO: Can we cache this?

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
        Ok(cleaned_file_name.to_string())
    }

    async fn trigger_work_validation(&mut self, work_key: &str) -> Result<(), ProcessWorkKeyError> {
        let cleaned_file_name = self.get_file_name_for_work_key(work_key).await?;

        if let Err(e) = self
            .trigger_remote_toploc_validation(&cleaned_file_name, work_key)
            .await
        {
            error!(
                "Failed to trigger remote toploc validation for {}: {}",
                cleaned_file_name, e
            );
            return Err(ProcessWorkKeyError::ValidationTriggerError(e.to_string()));
        }

        Ok(())
    }

    async fn process_workkey_status(&mut self, work_key: &str) -> Result<(), ProcessWorkKeyError> {
        let cleaned_file_name = self.get_file_name_for_work_key(work_key).await?;

        let result = self.poll_remote_toploc_validation(&cleaned_file_name).await;
        let validation_result = result?;

        match validation_result {
            ValidationResult::Accept => {
                info!("Validation accepted for {}", cleaned_file_name);
            }
            ValidationResult::Reject => {
                info!("Validation rejected for {}", cleaned_file_name);
                if let Err(e) = self.invalidate_work(work_key).await {
                    error!("Failed to invalidate work {}: {}", work_key, e);
                    return Err(ProcessWorkKeyError::InvalidatingWorkError(e.to_string()));
                }
            }
            _ => (),
        }

        if let Err(e) = self
            .update_work_validation_status(work_key, &validation_result)
            .await
        {
            error!(
                "Failed to update work validation status for {}: {}",
                work_key, e
            );
            return Err(ProcessWorkKeyError::ValidationPollingError(e.to_string()));
        }

        Ok(())
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

        // Process each work key
        for work_key in &work_keys {
            let cache_status = self.get_work_validation_status_from_redis(&work_key).await;
            debug!("Cache status for {}: {:?}", work_key, cache_status);
            match cache_status {
                Some(status) => match status {
                    ValidationResult::Accept
                    | ValidationResult::Reject
                    | ValidationResult::Crashed => {
                        debug!(
                            "Work key {} already processed with status: {:?}",
                            work_key, status
                        );
                        continue;
                    }
                    _ => {
                        if let Err(e) = self.process_workkey_status(&work_key).await {
                            error!("Failed to process work key {}: {}", work_key, e);
                        }
                    }
                },
                None => {
                    if let Err(e) = self.trigger_work_validation(&work_key).await {
                        error!("Failed to trigger work key {}: {}", work_key, e);
                    }
                }
            }
        }
        Ok(())
    }
}
