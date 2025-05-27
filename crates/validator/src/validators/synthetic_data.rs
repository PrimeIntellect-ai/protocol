use crate::store::redis::RedisStore;
use crate::validators::Validator;
use alloy::primitives::U256;
use anyhow::{Context, Error};
use hex;
use log::{debug, warn};
use log::{error, info};
use redis::Commands;
use serde::{Deserialize, Serialize};
use shared::utils::google_cloud::resolve_mapping_for_sha;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::{
    SyntheticDataWorkValidator, WorkInfo,
};
use std::fmt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
enum ValidationResult {
    Accept,
    Reject,
    Crashed,
    Pending,
    Unknown,
    Invalidated,
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
    /// Error when no matching toploc config is found for the file name.
    NoMatchingToplocConfig,
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
            ProcessWorkKeyError::NoMatchingToplocConfig => {
                write!(f, "No matching toploc config found")
            }
        }
    }
}

#[derive(Clone, Default, Debug, PartialEq)]
pub struct ToplocConfig {
    pub server_url: String,
    pub auth_token: Option<String>,
    pub file_prefix_filter: Option<String>,
}

impl ToplocConfig {
    pub fn matches_file_name(&self, file_name: &str) -> bool {
        match &self.file_prefix_filter {
            Some(prefix) => file_name.starts_with(prefix),
            None => true,
        }
    }
}

#[derive(Clone)]
struct ToplocConfigWithClient {
    config: ToplocConfig,
    client: reqwest::Client,
}

#[derive(Clone)]
pub struct SyntheticDataValidator {
    pool_id: U256,
    validator: SyntheticDataWorkValidator,
    prime_network: PrimeNetworkContract,
    toploc_configs: Vec<ToplocConfigWithClient>,
    penalty: U256,
    s3_credentials: Option<String>,
    bucket_name: Option<String>,
    redis_store: RedisStore,
    cancellation_token: CancellationToken,
    work_validation_interval: u64,
    unknown_status_expiry_seconds: u64,
    // Interval between work validation requests to toploc server
    grace_interval: u64,
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
        toploc_configs: Vec<ToplocConfig>,
        penalty: U256,
        s3_credentials: Option<String>,
        bucket_name: Option<String>,
        redis_store: RedisStore,
        cancellation_token: CancellationToken,
        work_validation_interval: u64,
        unknown_status_expiry_seconds: u64,
        grace_interval: u64,
    ) -> Self {
        let pool_id = pool_id_str.parse::<U256>().expect("Invalid pool ID");

        if s3_credentials.is_none() && bucket_name.is_none() {
            error!("S3 credentials and bucket name are not provided");
            std::process::exit(1);
        }

        let mut toploc_configs_with_client = Vec::new();
        for config in toploc_configs {
            let client = reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    if let Some(token) = &config.auth_token {
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
            toploc_configs_with_client.push(ToplocConfigWithClient { config, client });
        }

        Self {
            pool_id,
            validator,
            prime_network,
            toploc_configs: toploc_configs_with_client,
            penalty,
            s3_credentials,
            bucket_name,
            redis_store,
            cancellation_token,
            work_validation_interval,
            unknown_status_expiry_seconds,
            grace_interval,
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

    async fn get_file_name_for_work_key(
        &self,
        work_key: &str,
    ) -> Result<String, ProcessWorkKeyError> {
        let redis_key = format!("file_name:{}", work_key);
        let mut con = self
            .redis_store
            .client
            .get_connection()
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;

        // Try to get the file name from Redis cache
        let file_name: Option<String> = con
            .get(&redis_key)
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;
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

    fn get_toploc_config_for_file_name(
        &self,
        file_name: &str,
    ) -> Result<&ToplocConfigWithClient, Error> {
        // Find the first config that matches the file name prefix filter
        // If no filter is set, the config matches by default
        let config = self
            .toploc_configs
            .iter()
            .find(|config| config.config.matches_file_name(file_name))
            .ok_or(Error::msg("No matching toploc config found"))?;

        Ok(config)
    }

    async fn trigger_remote_toploc_validation(
        &self,
        work_key: &str,
        key_address: &str,
    ) -> Result<(), ProcessWorkKeyError> {
        let file_name = self.get_file_name_for_work_key(work_key).await?;

        let toploc_config = self.get_toploc_config_for_file_name(&file_name)?;

        // Returns a file name like /model/dataset/groupid-groupsize-filenumber-idx

        // We need to wait for all submissions before triggering the validation

        // We actually need group information here since toploc wants all files at once
        // http://localhost:8000/validategroup/outputs/step_2/meow - will be adding -$i  to the end of the file name
        // this is important for the template of the task
        /*
        {
            "file_shas": [
              "c94d6199fd9fca27613f4def6bf039110435cc9ac645d50db9f756f70fb1dec2", "8d0f0079c99da0bb7573d913cfece7677f00f6c836cc09979944f0e2765248c7"
            ],
            "group_id": "string",
            "file_number": 0,
            "group_size": 2
          }
        */

        let validate_url = format!("{}/validate/{}", toploc_config.config.server_url, file_name);
        info!(
            "Triggering remote toploc validation for {} {}",
            file_name, validate_url
        );

        let body = serde_json::json!({
            "file_sha": work_key,
            "address": key_address
        });

        let start_time = std::time::Instant::now();
        match toploc_config
            .client
            .post(&validate_url)
            .json(&body)
            .send()
            .await
        {
            Ok(_) => {
                let trigger_duration = start_time.elapsed();
                info!(
                    "Remote toploc validation triggered for {} in {:?}",
                    file_name, trigger_duration
                );

                let redis_start = std::time::Instant::now();
                self.update_work_validation_status(work_key, &ValidationResult::Unknown)
                    .await?;
                let redis_duration = redis_start.elapsed();
                info!(
                    "Redis status updated for {} in {:?}",
                    work_key, redis_duration
                );

                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to trigger remote toploc validation for {}: {}",
                    file_name, e
                );
                Err(ProcessWorkKeyError::ValidationTriggerError(e.to_string()))
            }
        }
    }

    async fn poll_remote_toploc_validation(
        &self,
        file_name: &str,
    ) -> Result<ValidationResult, Error> {
        let toploc_config = self.get_toploc_config_for_file_name(file_name)?;

        let url = format!("{}/status/{}", toploc_config.config.server_url, file_name);

        match toploc_config.client.get(&url).send().await {
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
                            debug!("Validation status for {}: {}", file_name, status);

                            let validation_result = match status {
                                "accept" => ValidationResult::Accept,
                                "reject" => ValidationResult::Reject,
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
            ValidationResult::Unknown => self.unknown_status_expiry_seconds,
            _ => 0,
        };
        let mut con = self.redis_store.client.get_connection()?;
        let key = self.get_key_for_work_key(work_key);
        let status = serde_json::to_string(&status)?;
        if expiry > 0 {
            let _: () = con
                .set_options(
                    &key,
                    status,
                    redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(expiry)),
                )
                .map_err(|e| Error::msg(format!("Failed to set work validation status: {}", e)))?;
        } else {
            let _: () = con
                .set(&key, status)
                .map_err(|e| Error::msg(format!("Failed to set work validation status: {}", e)))?;
        }
        Ok(())
    }

    async fn get_work_validation_status_from_redis(
        &self,
        work_key: &str,
    ) -> Result<Option<ValidationResult>, Error> {
        let mut con = self.redis_store.client.get_connection()?;
        let key = self.get_key_for_work_key(work_key);
        let status: Option<String> = con
            .get(key)
            .map_err(|e| Error::msg(format!("Failed to get work validation status: {}", e)))?;
        status
            .map(|status| {
                serde_json::from_str(&status).map_err(|e| {
                    Error::msg(format!("Failed to parse work validation status: {}", e))
                })
            })
            .transpose()
    }

    async fn update_work_info_in_redis(
        &self,
        work_key: &str,
        work_info: &WorkInfo,
    ) -> Result<(), Error> {
        let mut con = self.redis_store.client.get_connection()?;
        let key = format!("work_info:{}", work_key);
        let work_info = serde_json::to_string(&work_info)?;
        let _: () = con
            .set(&key, work_info)
            .map_err(|e| Error::msg(format!("Failed to set work info: {}", e)))?;
        Ok(())
    }

    async fn get_work_info_from_redis(&self, work_key: &str) -> Result<Option<WorkInfo>, Error> {
        let mut con = self.redis_store.client.get_connection()?;
        let key = format!("work_info:{}", work_key);
        let work_info: Option<String> = con
            .get(&key)
            .map_err(|e| Error::msg(format!("Failed to get work info: {}", e)))?;
        work_info
            .map(|work_info| {
                serde_json::from_str(&work_info)
                    .map_err(|e| Error::msg(format!("Failed to parse work info: {}", e)))
            })
            .transpose()
    }

    async fn process_workkey_status(&self, work_key: &str) -> Result<(), ProcessWorkKeyError> {
        let cleaned_file_name = self.get_file_name_for_work_key(work_key).await?;

        let result = self.poll_remote_toploc_validation(&cleaned_file_name).await;
        let validation_result = result?;
        info!(
            "Validation result for {}: {:?}",
            work_key, validation_result
        );

        match validation_result {
            ValidationResult::Accept => {
                info!("Validation accepted for {}", cleaned_file_name);
            }
            ValidationResult::Reject => {
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

    pub async fn validate_work(self) -> Result<(), Error> {
        debug!("Validating work for pool ID: {:?}", self.pool_id);

        // Get all work keys for the pool from the last 24 hours
        let max_age_in_seconds = 60 * self.work_validation_interval;
        let current_timestamp = U256::from(chrono::Utc::now().timestamp());
        let max_age_ago = current_timestamp - U256::from(max_age_in_seconds);

        let work_keys = self
            .validator
            .get_work_since(self.pool_id, max_age_ago)
            .await
            .context("Failed to get work keys from the last 24 hours")?;

        if !work_keys.is_empty() {
            info!(
                "Found {} work keys to validate in the last {} seconds creation time",
                work_keys.len(),
                max_age_in_seconds
            );
        } else {
            debug!(
                "No work keys to validate in the last {} seconds creation time",
                max_age_in_seconds
            );
        }

        let self_arc = Arc::new(self);
        let cancellation_token = self_arc.cancellation_token.clone();
        let validator_clone_trigger = self_arc.clone();
        let validator_clone_status = self_arc.clone();

        let mut trigger_tasks: Vec<(String, WorkInfo)> = Vec::new();
        let mut status_tasks: Vec<String> = Vec::new();

        for work_key in &work_keys {
            // Get work info from cache or fetch from validator
            let work_info = match self_arc.get_work_info_from_redis(work_key).await? {
                Some(cached_info) => cached_info,
                None => {
                    match self_arc
                        .validator
                        .get_work_info(self_arc.pool_id, work_key)
                        .await
                    {
                        Ok(info) => {
                            // Update cache with fetched work info
                            if let Err(e) =
                                self_arc.update_work_info_in_redis(work_key, &info).await
                            {
                                error!("Failed to cache work info for {}: {}", work_key, e);
                            }
                            info
                        }
                        Err(e) => {
                            error!("Failed to get work info for {}: {}", work_key, e);
                            continue;
                        }
                    }
                }
            };
            debug!("Key {} has {} work units", work_key, work_info.work_units);

            // Invalidate work if work units exceed threshold
            // TODO: This has to be adjusted for synthetic-II !
            if work_info.work_units > U256::from(1) {
                if let Err(e) = self_arc.invalidate_work(work_key).await {
                    error!("Failed to invalidate work {}: {}", work_key, e);
                    continue;
                }
                self_arc
                    .update_work_validation_status(work_key, &ValidationResult::Invalidated)
                    .await?;
                continue;
            }

            let cache_status = self_arc
                .get_work_validation_status_from_redis(work_key)
                .await?;
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
                        status_tasks.push(work_key.clone());
                    }
                },
                None => {
                    trigger_tasks.push((work_key.clone(), work_info));
                }
            }
        }

        let trigger_handle = tokio::spawn(async move {
            for work_info in trigger_tasks {
                if let Err(e) = validator_clone_trigger
                    .trigger_remote_toploc_validation(
                        &work_info.0,
                        &work_info.1.node_id.to_string(),
                    )
                    .await
                {
                    error!("Failed to trigger work key {}: {}", work_info.0, e);
                }
                info!(
                    "waiting before next task: {}",
                    validator_clone_trigger.grace_interval
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(
                    validator_clone_trigger.grace_interval,
                ))
                .await;
            }
        });

        let status_handle = tokio::spawn(async move {
            for work_key in status_tasks {
                if let Err(e) = validator_clone_status
                    .process_workkey_status(&work_key)
                    .await
                {
                    error!("Failed to process work key {}: {}", work_key, e);
                }
            }
        });

        tokio::select! {
            _ = trigger_handle => (),
            _ = status_handle => (),
            _ = cancellation_token.cancelled() => {
                warn!("Validation cancelled");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::Address;
    use anyhow::Ok;
    use shared::web3::contracts::core::builder::ContractBuilder;
    use shared::web3::wallet::Wallet;
    use url::Url;
    fn test_store() -> RedisStore {
        let store = RedisStore::new_test();
        let mut con = store
            .client
            .get_connection()
            .expect("Should connect to test Redis instance");

        redis::cmd("PING")
            .query::<String>(&mut con)
            .expect("Redis should be responsive");
        redis::cmd("FLUSHALL")
            .query::<String>(&mut con)
            .expect("Redis should be flushed");
        store
    }

    #[tokio::test]
    async fn test_status_update() -> Result<(), Error> {
        let store = test_store();
        let demo_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            Url::parse("http://localhost:8545").unwrap(),
        )
        .map_err(|e| Error::msg(format!("Failed to create demo wallet: {}", e)))?;
        let contracts = ContractBuilder::new(&demo_wallet)
            .with_compute_registry()
            .with_ai_token()
            .with_prime_network()
            .with_compute_pool()
            .with_domain_registry()
            .with_stake_manager()
            .with_synthetic_data_validator(Some(Address::ZERO))
            .build()
            .map_err(|e| Error::msg(format!("Failed to build contracts: {}", e)))?;

        // Get S3 credentials from environment variables if they exist
        let s3_credentials = std::env::var("S3_CREDENTIALS").ok();
        let bucket_name = std::env::var("S3_BUCKET_NAME").ok();

        // If either credential is missing, we'll proceed with None values
        if s3_credentials.is_none() || bucket_name.is_none() {
            println!("S3 credentials or bucket name not found in environment, proceeding with test using None values");
            return Ok(());
        }

        let validator = SyntheticDataValidator::new(
            "0".to_string(),
            contracts.synthetic_data_validator.clone().unwrap(),
            contracts.prime_network.clone(),
            vec![ToplocConfig {
                server_url: "http://localhost:8080".to_string(),
                ..Default::default()
            }],
            U256::from(0),
            s3_credentials,
            bucket_name,
            store,
            CancellationToken::new(),
            10,
            60,
            1,
        );
        validator
            .update_work_validation_status(
                "0x0000000000000000000000000000000000000000",
                &ValidationResult::Accept,
            )
            .await
            .map_err(|e| {
                error!("Failed to update work validation status: {}", e);
                Error::msg(format!("Failed to update work validation status: {}", e))
            })?;

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        let status = validator
            .get_work_validation_status_from_redis("0x0000000000000000000000000000000000000000")
            .await
            .map_err(|e| {
                error!("Failed to get work validation status: {}", e);
                Error::msg(format!("Failed to get work validation status: {}", e))
            })?;
        assert_eq!(status, Some(ValidationResult::Accept));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_toploc_config_for_file_name() -> Result<(), Error> {
        let configs = [ToplocConfig {
                server_url: "http://localhost:8080".to_string(),
                file_prefix_filter: Some("model1".to_string()),
                ..Default::default()
            },
            ToplocConfig {
                server_url: "http://localhost:8081".to_string(),
                file_prefix_filter: Some("model2".to_string()),
                ..Default::default()
            }];

        let file_name = "model1/dataset/groupid-groupsize-filenumber-idx";
        let config = configs
            .iter()
            .find(|config| config.matches_file_name(file_name));
        assert_eq!(config, Some(&configs[0]));
        Ok(())
    }
}
