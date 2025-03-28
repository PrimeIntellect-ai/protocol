use crate::store::redis::RedisStore;
use crate::validators::Validator;
use alloy::primitives::U256;
use anyhow::{Context, Error};
use hex;
use log::debug;
use log::{error, info};
use redis::Commands;
use serde::{Deserialize, Serialize};
use shared::utils::google_cloud::resolve_mapping_for_sha;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::SyntheticDataWorkValidator;
use std::fmt;

#[derive(Debug, Serialize, Deserialize)]
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

        Self {
            pool_id,
            validator,
            prime_network,
            leviticus_url,
            leviticus_token,
            penalty,
            s3_credentials,
            bucket_name,
            redis_store,
        }
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
        status.map(|status| serde_json::from_str(&status).unwrap())
    }

    async fn get_file_name_for_work_key(
        &self,
        work_key: &str,
    ) -> Result<String, ProcessWorkKeyError> {
        let redis_key = format!("file_name:{}", work_key);
        let mut con = self.redis_store.client.get_connection().unwrap();

        // Try to get the file name from Redis cache
        let file_name: Option<String> = con.get(&redis_key).unwrap();
        if let Some(cached_file_name) = file_name {
            return Ok(cached_file_name);
        }

        // Resolve the file name if not found in cache
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
            .unwrap_or(&original_file_name);

        // Cache the resolved and cleaned file name in Redis
        let _: () = con.set(&redis_key, cleaned_file_name).unwrap();

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

        // Get all work keys for the pool from the last 24 hours
        let twenty_four_hours_in_seconds = 86400;
        let current_timestamp = U256::from(chrono::Utc::now().timestamp());
        let twenty_four_hours_ago = current_timestamp - U256::from(twenty_four_hours_in_seconds);

        let work_keys = self
            .validator
            .get_work_since(self.pool_id, twenty_four_hours_ago)
            .await
            .context("Failed to get work keys from the last 24 hours")?;

        debug!("Found {} work keys to validate", work_keys.len());

        // Process each work key
        for work_key in &work_keys {
            let cache_status = self.get_work_validation_status_from_redis(work_key).await;
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
                        if let Err(e) = self.process_workkey_status(work_key).await {
                            error!("Failed to process work key {}: {}", work_key, e);
                        }
                    }
                },
                None => {
                    if let Err(e) = self.trigger_work_validation(work_key).await {
                        error!("Failed to trigger work key {}: {}", work_key, e);
                    }
                }
            }
        }
        Ok(())
    }
}
