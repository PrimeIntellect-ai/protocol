use crate::metrics::MetricsContext;
use crate::store::redis::RedisStore;
use crate::validators::Validator;
use alloy::primitives::U256;
use anyhow::{Context, Error};
use hex;
use log::{debug, warn};
use log::{error, info};
use redis::Commands;
use serde::{Deserialize, Serialize};
use shared::utils::StorageProvider;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::{
    SyntheticDataWorkValidator, WorkInfo,
};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
pub mod toploc;
use toploc::{Toploc, ToplocConfig};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum ValidationResult {
    Accept,
    Reject,
    Crashed,
    Pending,
    Unknown,
    Invalidated,
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationResult::Accept => write!(f, "accept"),
            ValidationResult::Reject => write!(f, "reject"),
            ValidationResult::Crashed => write!(f, "crashed"),
            ValidationResult::Pending => write!(f, "pending"),
            ValidationResult::Unknown => write!(f, "unknown"),
            ValidationResult::Invalidated => write!(f, "invalidated"),
        }
    }
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
    /// Error when no matching toploc config is found.
    NoMatchingToplocConfig(String),
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
            ProcessWorkKeyError::NoMatchingToplocConfig(msg) => {
                write!(f, "No matching toploc config: {:?}", msg)
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct GroupInformation {
    /// The full filename without the last -i.parquet index
    group_file_name: String,
    /// Everything before the first group ID number
    prefix: String,
    /// The group ID number from the filename
    group_id: String,
    /// The group size from the filename
    group_size: u32,
    /// The file number from the filename
    /// This is not the file upload count! The first file starts with 0
    file_number: u32,
    /// The index number from the filename
    idx: String,
}
impl FromStr for GroupInformation {
    type Err = Error;

    /// Parse a filename into GroupInformation
    /// Expected format: "prefix-groupid-groupsize-filenumber-idx.parquet"
    fn from_str(file_name: &str) -> Result<Self, Self::Err> {
        let re = regex::Regex::new(r".*?-([0-9a-fA-F]+)-(\d+)-(\d+)-(\d+)(\.[^.]+)$")
            .map_err(|e| Error::msg(format!("Failed to compile regex: {}", e)))?;

        let caps = re
            .captures(file_name)
            .ok_or_else(|| Error::msg("File name does not match expected format"))?;

        let groupid_start = caps
            .get(1)
            .ok_or_else(|| Error::msg("Failed to extract group ID"))?
            .start();
        let prefix = file_name[..groupid_start - 1].to_string();

        let groupid = caps
            .get(1)
            .ok_or_else(|| Error::msg("Failed to extract group ID"))?
            .as_str();
        let groupsize = caps
            .get(2)
            .ok_or_else(|| Error::msg("Failed to extract group size"))?
            .as_str()
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse group size: {}", e)))?;
        let filenumber = caps
            .get(3)
            .ok_or_else(|| Error::msg("Failed to extract file number"))?
            .as_str()
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse file number: {}", e)))?;
        let idx = caps
            .get(4)
            .ok_or_else(|| Error::msg("Failed to extract index"))?
            .as_str();
        let extension = caps
            .get(5)
            .ok_or_else(|| Error::msg("Failed to extract extension"))?
            .as_str();

        // Get the group file name by removing just the index part but keeping the extension
        let group_file_name = file_name[..file_name
            .rfind('-')
            .ok_or_else(|| Error::msg("Failed to find last hyphen in filename"))?]
            .to_string()
            + extension;

        Ok(Self {
            group_file_name,
            prefix,
            group_id: groupid.to_string(),
            group_size: groupsize,
            file_number: filenumber,
            idx: idx.to_string(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ToplocGroup {
    // This is the filename without the last -i.parquet index
    pub group_file_name: String,
    pub group_size: u32,
    pub file_number: u32,
    pub prefix: String,
    pub sorted_work_keys: Vec<String>,
    pub group_id: String,
}

impl From<GroupInformation> for ToplocGroup {
    fn from(info: GroupInformation) -> Self {
        Self {
            group_file_name: info.group_file_name,
            group_size: info.group_size,
            file_number: info.file_number,
            prefix: info.prefix,
            sorted_work_keys: Vec::new(),
            group_id: info.group_id,
        }
    }
}

impl GroupInformation {
    fn to_redis(&self) -> Result<String, anyhow::Error> {
        Ok(serde_json::to_string(self)?)
    }

    fn from_redis(s: &str) -> Result<Self, anyhow::Error> {
        Ok(serde_json::from_str(s)?)
    }
}

#[derive(Clone)]
pub struct SyntheticDataValidator {
    pool_id: U256,
    validator: SyntheticDataWorkValidator,
    prime_network: PrimeNetworkContract,
    toploc: Vec<Toploc>,
    penalty: U256,
    storage_provider: Arc<dyn StorageProvider>,
    redis_store: RedisStore,
    cancellation_token: CancellationToken,
    work_validation_interval: u64,
    unknown_status_expiry_seconds: u64,
    // Interval between work validation requests to toploc server
    grace_interval: u64,
    batch_trigger_size: usize,
    // Whether to use node grouping
    with_node_grouping: bool,
    disable_chain_invalidation: bool,
    metrics: Option<MetricsContext>,
}

impl Validator for SyntheticDataValidator {
    type Error = anyhow::Error;

    fn name(&self) -> &str {
        "Synthetic Data Validator"
    }
}

#[derive(Debug)]
pub struct ValidationPlan {
    pub single_trigger_tasks: Vec<(String, WorkInfo)>,
    pub group_trigger_tasks: Vec<ToplocGroup>,
    pub status_check_tasks: Vec<String>,
    pub group_status_check_tasks: Vec<ToplocGroup>,
}

impl SyntheticDataValidator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool_id_str: String,
        validator: SyntheticDataWorkValidator,
        prime_network: PrimeNetworkContract,
        toploc_configs: Vec<ToplocConfig>,
        penalty: U256,
        storage_provider: Arc<dyn StorageProvider>,
        redis_store: RedisStore,
        cancellation_token: CancellationToken,
        work_validation_interval: u64,
        unknown_status_expiry_seconds: u64,
        grace_interval: u64,
        batch_trigger_size: usize,
        with_node_grouping: bool,
        disable_chain_invalidation: bool,
        metrics: Option<MetricsContext>,
    ) -> Self {
        let pool_id = pool_id_str.parse::<U256>().expect("Invalid pool ID");

        let mut toploc = Vec::new();
        for config in toploc_configs {
            toploc.push(Toploc::new(config, metrics.clone()));
        }

        Self {
            pool_id,
            validator,
            prime_network,
            toploc,
            penalty,
            storage_provider,
            redis_store,
            cancellation_token,
            work_validation_interval,
            unknown_status_expiry_seconds,
            grace_interval,
            batch_trigger_size,
            with_node_grouping,
            disable_chain_invalidation,
            metrics,
        }
    }

    pub async fn invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        let data = hex::decode(work_key)
            .map_err(|e| Error::msg(format!("Failed to decode hex work key: {}", e)))?;
        info!("Invalidating work: {}", work_key);

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work invalidation");
            return Ok(());
        }

        if let Some(metrics) = &self.metrics {
            metrics.record_work_key_invalidation();
        }

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

        let original_file_name = self
            .storage_provider
            .resolve_mapping_for_sha(work_key)
            .await
            .map_err(|e| ProcessWorkKeyError::FileNameResolutionError(e.to_string()))?;

        if original_file_name.is_empty() {
            error!(
                "Failed to resolve original file name for work key: {}",
                work_key
            );
            if let Some(metrics) = &self.metrics {
                metrics.record_work_key_error("file_name_resolution_failed");
            }
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

    async fn process_workkey_status(&self, work_key: &str) -> Result<(), ProcessWorkKeyError> {
        let cleaned_file_name = self.get_file_name_for_work_key(work_key).await?;

        let toploc_config = self
            .toploc
            .iter()
            .find(|t| t.matches_file_name(&cleaned_file_name))
            .ok_or(ProcessWorkKeyError::NoMatchingToplocConfig(format!(
                "No matching toploc config found for {}",
                cleaned_file_name
            )))?;

        let result = toploc_config
            .get_single_file_validation_status(&cleaned_file_name)
            .await;
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

    async fn build_group_for_key(&self, work_key: &str) -> Result<String, Error> {
        let file = self
            .get_file_name_for_work_key(work_key)
            .await
            .map_err(|e| {
                error!("Failed to get file name for work key: {}", e);
                Error::msg(format!("Failed to get file name for work key: {}", e))
            })?;
        debug!("File for work key: {:?} | {:?}", work_key, file);
        let group_info = GroupInformation::from_str(&file)?;
        debug!("Group info: {:?}", group_info);
        let group_key: String = format!(
            "group:{}:{}:{}",
            group_info.group_id, group_info.group_size, group_info.file_number
        );
        let mut redis: redis::Connection = self.redis_store.client.get_connection()?;
        redis
            .hset::<_, _, _, ()>(&group_key, work_key, group_info.to_redis()?)
            .unwrap();

        Ok(group_key)
    }

    async fn is_group_ready_for_validation(&self, group_key: &str) -> Result<bool, Error> {
        let mut redis: redis::Connection = self.redis_store.client.get_connection()?;
        let group_size: u32 = redis.hlen::<_, u32>(group_key).unwrap();
        let expected_size = group_key.split(':').nth(2).unwrap().parse::<u32>().unwrap();
        Ok(group_size == expected_size)
    }

    async fn get_group(&self, work_key: &str) -> Result<Option<ToplocGroup>, Error> {
        let group_key = match self.build_group_for_key(work_key).await {
            Ok(key) => key,
            Err(e) => {
                error!("Failed to build group key for work key {}: {}", work_key, e);
                return Ok(None);
            }
        };
        let ready_for_validation = self.is_group_ready_for_validation(&group_key).await?;
        debug!(
            "Group for key {:?} ready for validation: {:?}",
            work_key, ready_for_validation
        );
        if ready_for_validation {
            let mut redis: redis::Connection = self.redis_store.client.get_connection()?;
            let group_entries: HashMap<String, String> = redis.hgetall(&group_key)?;
            // Parse all entries once and sort by idx
            let mut entries: Vec<(String, GroupInformation)> = group_entries
                .into_iter()
                .map(|(key, value)| {
                    let info = GroupInformation::from_redis(&value).unwrap();
                    (key, info)
                })
                .collect();

            entries.sort_by_key(|(_, info)| info.idx.parse::<usize>().unwrap());

            let mut toploc_group: ToplocGroup = entries
                .first()
                .ok_or_else(|| Error::msg("No group info found"))?
                .1
                .clone()
                .into();

            // Extract sorted work keys from the sorted entries
            toploc_group.sorted_work_keys = entries.into_iter().map(|(key, _)| key).collect();

            return Ok(Some(toploc_group));
        }
        Ok(None)
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
        self.process_work_keys(work_keys).await
    }

    pub async fn build_validation_plan(
        &self,
        work_keys: Vec<String>,
    ) -> Result<ValidationPlan, Error> {
        let mut single_trigger_tasks: Vec<(String, WorkInfo)> = Vec::new();
        let mut group_trigger_tasks: Vec<ToplocGroup> = Vec::new();
        let mut status_check_tasks: Vec<String> = Vec::new();
        let mut group_status_check_tasks: Vec<ToplocGroup> = Vec::new();

        let mut keys_to_process = 0;

        for work_key in work_keys {
            // Get work info from cache or fetch
            let work_info = match self.get_work_info_from_redis(&work_key).await? {
                Some(cached_info) => cached_info,
                None => {
                    match self.validator.get_work_info(self.pool_id, &work_key).await {
                        Ok(info) => {
                            // Update cache with fetched work info
                            if let Err(e) = self.update_work_info_in_redis(&work_key, &info).await {
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
            let cache_status = self
                .get_work_validation_status_from_redis(&work_key)
                .await?;

            match cache_status {
                Some(
                    ValidationResult::Accept | ValidationResult::Reject | ValidationResult::Crashed,
                ) => {
                    continue; // Already processed
                }
                Some(ValidationResult::Unknown) => {
                    keys_to_process += 1;
                    if self.with_node_grouping {
                        let check_group = self.get_group(&work_key).await?;
                        debug!("Group for work key: {:?} | {:?}", work_key, check_group);
                        if let Some(group) = check_group {
                            // Only add group if it's not already in the list
                            if !group_status_check_tasks.iter().any(|g| {
                                g.group_id == group.group_id && g.file_number == group.file_number
                            }) {
                                group_status_check_tasks.push(group);
                            }
                        }
                    } else {
                        status_check_tasks.push(work_key); // Check status
                    }
                }
                Some(_) | None => {
                    keys_to_process += 1;
                    // Needs triggering (covers Pending, Invalidated, and None cases)
                    if self.with_node_grouping {
                        debug!("Checking group for work key: {:?}", work_key);
                        let check_group = self.get_group(&work_key).await?;
                        if let Some(group) = check_group {
                            debug!("Group found for work key: {:?}", work_key);
                            group_trigger_tasks.push(group);
                        } else {
                            debug!("Could not build a final group for work key: {:?}", work_key);
                        }
                    } else {
                        single_trigger_tasks.push((work_key.clone(), work_info));
                    }
                }
            }
        }

        info!(
            "keys_to_process: {} (including keys with no status or pending status)",
            keys_to_process
        );
        if let Some(metrics) = &self.metrics {
            metrics.record_work_keys_to_process(keys_to_process as f64);
        }

        Ok(ValidationPlan {
            single_trigger_tasks,
            group_trigger_tasks,
            status_check_tasks,
            group_status_check_tasks,
        })
    }

    fn find_matching_toploc_config(&self, file_name: &str) -> Option<&Toploc> {
        self.toploc.iter().find(|t| t.matches_file_name(file_name))
    }

    pub async fn process_single_task(&self, work_info: (String, WorkInfo)) -> Result<(), Error> {
        let file_name = self
            .get_file_name_for_work_key(&work_info.0)
            .await
            .map_err(|e| {
                error!("Failed to get file name for work key: {}", e);
                Error::msg(format!("Failed to get file name for work key: {}", e))
            })?;
        let toploc_config = self
            .find_matching_toploc_config(&file_name)
            .ok_or(Error::msg("No matching toploc config found"))?;
        toploc_config
            .trigger_single_file_validation(
                &work_info.0,
                &work_info.1.node_id.to_string(),
                &file_name,
            )
            .await?;

        // Update validation status to Unknown after successful trigger
        self.update_work_validation_status(&work_info.0, &ValidationResult::Unknown)
            .await?;

        Ok(())
    }

    pub async fn process_group_task(&self, group: ToplocGroup) -> Result<(), Error> {
        let toploc_config = self
            .find_matching_toploc_config(&group.prefix)
            .ok_or(Error::msg(format!(
                "No matching toploc config found for group {} {}",
                group.group_id, group.prefix
            )))?;

        toploc_config
            .trigger_group_file_validation(
                &group.group_file_name,
                group.sorted_work_keys.clone(),
                &group.group_id,
                group.file_number,
                group.group_size,
            )
            .await?;

        // Update status for all work keys in the group
        for work_key in &group.sorted_work_keys {
            self.update_work_validation_status(work_key, &ValidationResult::Unknown)
                .await?;
        }

        Ok(())
    }
    pub async fn process_group_status_check(&self, group: ToplocGroup) -> Result<(), Error> {
        let toploc_config = self
            .find_matching_toploc_config(&group.prefix)
            .ok_or(Error::msg(format!(
                "No matching toploc config found for group for status check {} {}",
                group.group_id, group.prefix
            )))?;

        let status = toploc_config
            .get_group_file_validation_status(&group.group_file_name)
            .await?;
        let toploc_config_name = toploc_config.name();
        if let Some(metrics) = &self.metrics {
            metrics.record_group_validation_status(
                &group.group_id,
                &toploc_config_name,
                &status.status.to_string(),
            );
        }

        // Calculate total claimed units
        let mut total_claimed_units: U256 = U256::from(0);
        for work_key in &group.sorted_work_keys {
            if let Some(work_info) = self.get_work_info_from_redis(work_key).await? {
                total_claimed_units += work_info.work_units;
            }
        }

        // Log basic info for all cases
        info!(
            "Group {} ({}) - Status: {:?}, Claimed: {}, Toploc: {} flops (input: {} flops)",
            group.group_id,
            group.group_file_name,
            status.status,
            total_claimed_units,
            status.output_flops,
            status.input_flops
        );

        // Handle rejection case
        if status.status == ValidationResult::Reject {
            let indices = status.failing_indices;
            let work_keys_to_invalidate: Vec<String> = indices
                .iter()
                .map(|&idx| group.sorted_work_keys[idx as usize].clone())
                .collect();

            warn!(
                "Group {} rejected - Invalidating keys: {:?}",
                group.group_id, work_keys_to_invalidate
            );

            for work_key in work_keys_to_invalidate {
                self.invalidate_work(&work_key).await?;
            }
        }

        // Handle mismatched units case
        if total_claimed_units != U256::from(status.output_flops as u64) {
            warn!(
                "Group {} units mismatch - Claimed: {}, Toploc: {}, Keys: {:?}",
                group.group_id, total_claimed_units, status.output_flops, group.sorted_work_keys
            );
        }

        // Update validation status for all work keys
        for work_key in &group.sorted_work_keys {
            self.update_work_validation_status(work_key, &status.status)
                .await?;
        }

        Ok(())
    }

    pub async fn process_work_keys(self, work_keys: Vec<String>) -> Result<(), Error> {
        debug!("Processing work keys: {:?}", work_keys);
        let validation_plan = self.build_validation_plan(work_keys).await?;
        debug!(
            "Number of single trigger tasks: {}",
            validation_plan.single_trigger_tasks.len()
        );
        debug!(
            "Number of group trigger tasks: {}",
            validation_plan.group_trigger_tasks.len()
        );
        debug!(
            "Number of status check tasks: {}",
            validation_plan.status_check_tasks.len()
        );
        debug!(
            "Number of group status check tasks: {}",
            validation_plan.group_status_check_tasks.len()
        );

        let self_arc = Arc::new(self);
        let cancellation_token = self_arc.cancellation_token.clone();
        let validator_clone_trigger = self_arc.clone();
        let validator_clone_group_trigger = self_arc.clone();
        let validator_clone_group_status = self_arc.clone();
        let validator_clone_single_status = self_arc.clone();

        let batch_single_trigger_task = validation_plan
            .single_trigger_tasks
            .into_iter()
            .take(self_arc.batch_trigger_size)
            .collect::<Vec<_>>();
        let batch_group_trigger_task = validation_plan
            .group_trigger_tasks
            .into_iter()
            .take(self_arc.batch_trigger_size)
            .collect::<Vec<_>>();

        let trigger_handle = tokio::spawn(async move {
            for work_info in batch_single_trigger_task {
                if let Err(e) = validator_clone_trigger.process_single_task(work_info).await {
                    error!("Failed to process single task: {}", e);
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
            for work_key in validation_plan.status_check_tasks {
                if let Err(e) = validator_clone_single_status
                    .process_workkey_status(&work_key)
                    .await
                {
                    error!("Failed to process work key {}: {}", work_key, e);
                }
            }
        });

        let group_trigger_handle = tokio::spawn(async move {
            for group in batch_group_trigger_task {
                if let Err(e) = validator_clone_group_trigger
                    .process_group_task(group)
                    .await
                {
                    error!("Failed to process group task: {}", e);
                }
                debug!(
                    "waiting before next task: {}",
                    validator_clone_group_trigger.grace_interval
                );
                tokio::time::sleep(tokio::time::Duration::from_secs(
                    validator_clone_group_trigger.grace_interval,
                ))
                .await;
            }
        });

        let group_status_handle = tokio::spawn(async move {
            for group in validation_plan.group_status_check_tasks {
                if let Err(e) = validator_clone_group_status
                    .process_group_status_check(group)
                    .await
                {
                    error!("Failed to process group status check: {}", e);
                }
            }
        });
        let all_tasks_future = async {
            let (trigger_res, group_trigger_res, status_res, group_status_res) = tokio::join!(
                trigger_handle,
                group_trigger_handle,
                status_handle,
                group_status_handle
            );

            if let Err(e) = trigger_res {
                error!("Single trigger task panicked: {:?}", e);
            }
            if let Err(e) = group_trigger_res {
                error!("Group trigger task panicked: {:?}", e);
            }
            if let Err(e) = status_res {
                error!("Single status task panicked: {:?}", e);
            }
            if let Err(e) = group_status_res {
                error!("Group status task panicked: {:?}", e);
            }
        };

        tokio::select! {
            _ = all_tasks_future => {
                info!("All validation sub-tasks completed for this cycle.");
            },

            _ = cancellation_token.cancelled() => {
                warn!("Validation processing was cancelled.");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::metrics::export_metrics;

    use super::*;
    use alloy::primitives::Address;
    use anyhow::Ok;
    use mockito::Server;
    use shared::utils::MockStorageProvider;
    use shared::web3::contracts::core::builder::{ContractBuilder, Contracts};
    use shared::web3::wallet::Wallet;
    use std::str::FromStr;
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

    fn setup_test_env() -> Result<(RedisStore, Contracts), Error> {
        let store = test_store();
        let url = Url::parse("http://localhost:8545").unwrap();

        let demo_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            url,
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

        Ok((store, contracts))
    }

    #[tokio::test]
    async fn test_build_validation_plan() -> Result<(), Error> {
        // Add test to build validation plan
        // Since we do not have blockchain access add task infos to redis first
        let (store, contracts) = setup_test_env()?;
        let metrics_context = MetricsContext::new("0".to_string(), Some("0".to_string()));
        let mock_storage = MockStorageProvider::new();

        // single group
        let single_group_file_name = "Qwen3/dataset/samplingn-9999999-1-9-0.parquet";
        mock_storage.add_file(single_group_file_name, "file1");
        mock_storage.add_mapping_file(
            "9999999999999999999999999999999999999999999999999999999999999999",
            single_group_file_name,
        );

        let single_unknown_file_name = "Qwen3/dataset/samplingn-8888888-1-9-0.parquet";
        mock_storage.add_file(single_unknown_file_name, "file1");
        mock_storage.add_mapping_file(
            "8888888888888888888888888888888888888888888888888888888888888888",
            single_unknown_file_name,
        );

        // multiple group
        mock_storage.add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
            "file1",
        );
        mock_storage.add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
            "file2",
        );
        mock_storage.add_mapping_file(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
        );
        mock_storage.add_mapping_file(
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
        );

        let storage_provider = Arc::new(mock_storage);

        let validator = SyntheticDataValidator::new(
            "0".to_string(),
            contracts.synthetic_data_validator.clone().unwrap(),
            contracts.prime_network.clone(),
            vec![ToplocConfig {
                server_url: "http://localhost:8080".to_string(),
                ..Default::default()
            }],
            U256::from(0),
            storage_provider,
            store,
            CancellationToken::new(),
            10,
            60,
            1,
            10,
            true,
            false,
            Some(metrics_context),
        );

        let work_keys = vec![
            "9999999999999999999999999999999999999999999999999999999999999999".to_string(),
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641".to_string(),
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf".to_string(),
            "8888888888888888888888888888888888888888888888888888888888888888".to_string(),
        ];

        let work_info = WorkInfo {
            node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ..Default::default()
        };
        for work_key in work_keys.clone() {
            validator
                .update_work_info_in_redis(&work_key, &work_info)
                .await?;
        }
        validator
            .update_work_validation_status(&work_keys[3], &ValidationResult::Unknown)
            .await?;

        let validation_plan = validator.build_validation_plan(work_keys.clone()).await?;
        assert_eq!(validation_plan.single_trigger_tasks.len(), 0);
        assert_eq!(validation_plan.group_trigger_tasks.len(), 2);
        assert_eq!(validation_plan.status_check_tasks.len(), 0);
        assert_eq!(validation_plan.group_status_check_tasks.len(), 1);

        let metrics = export_metrics().unwrap();
        println!("metrics: {:?}", metrics);
        assert!(
            metrics.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 4")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_status_update() -> Result<(), Error> {
        let (store, contracts) = setup_test_env()?;
        let mock_storage = MockStorageProvider::new();
        let storage_provider = Arc::new(mock_storage);

        let validator = SyntheticDataValidator::new(
            "0".to_string(),
            contracts.synthetic_data_validator.clone().unwrap(),
            contracts.prime_network.clone(),
            vec![ToplocConfig {
                server_url: "http://localhost:8080".to_string(),
                ..Default::default()
            }],
            U256::from(0),
            storage_provider,
            store,
            CancellationToken::new(),
            10,
            60,
            1,
            10,
            false,
            false,
            None,
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
    async fn test_group_filename_parsing() -> Result<(), Error> {
        // Test case 1: Valid filename with all components
        let name = "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet";
        let group_info = GroupInformation::from_str(name)?;

        assert_eq!(
            group_info.group_file_name,
            "Qwen3/dataset/samplingn-3450756714426841564-2-9.parquet"
        );
        assert_eq!(group_info.prefix, "Qwen3/dataset/samplingn");
        assert_eq!(group_info.group_id, "3450756714426841564");
        assert_eq!(group_info.group_size, 2);
        assert_eq!(group_info.file_number, 9);
        assert_eq!(group_info.idx, "1");

        // Test case 2: Invalid filename format
        let invalid_name = "invalid-filename.parquet";
        assert!(GroupInformation::from_str(invalid_name).is_err());

        // Test case 3: Filename with different numbers
        let name2 = "test/dataset/data-123-5-10-3.parquet";
        let group_info2 = GroupInformation::from_str(name2)?;

        assert_eq!(
            group_info2.group_file_name,
            "test/dataset/data-123-5-10.parquet"
        );
        assert_eq!(group_info2.prefix, "test/dataset/data");
        assert_eq!(group_info2.group_id, "123");
        assert_eq!(group_info2.group_size, 5);
        assert_eq!(group_info2.file_number, 10);
        assert_eq!(group_info2.idx, "3");

        Ok(())
    }

    #[tokio::test]
    async fn test_group_build() -> Result<(), Error> {
        let (store, contracts) = setup_test_env()?;

        let config = ToplocConfig {
            server_url: "http://localhost:8080".to_string(),
            ..Default::default()
        };

        let mock_storage = MockStorageProvider::new();
        mock_storage.add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
            "file1",
        );
        mock_storage.add_mapping_file(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-1.parquet",
        );
        mock_storage.add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
            "file2",
        );
        mock_storage.add_mapping_file(
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf",
            "Qwen3/dataset/samplingn-3450756714426841564-2-9-0.parquet",
        );

        let storage_provider = Arc::new(mock_storage);

        let validator = SyntheticDataValidator::new(
            "0".to_string(),
            contracts.synthetic_data_validator.clone().unwrap(),
            contracts.prime_network.clone(),
            vec![config],
            U256::from(0),
            storage_provider,
            store,
            CancellationToken::new(),
            10,
            60,
            1,
            10,
            false,
            false,
            None,
        );

        let group = validator
            .get_group("c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641")
            .await?;
        assert!(group.is_none());

        let group = validator
            .get_group("88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf")
            .await?;
        assert!(group.is_some());
        let group = group.unwrap();
        assert_eq!(&group.sorted_work_keys.len(), &2);
        assert_eq!(
            &group.sorted_work_keys[0],
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_group_e2e() -> Result<(), Error> {
        let mut server = Server::new_async().await;
        let (store, contracts) = setup_test_env()?;

        let config = ToplocConfig {
            server_url: server.url(),
            file_prefix_filter: Some("Qwen/Qwen0.6".to_string()),
            ..Default::default()
        };

        let file_sha = "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641";
        let group_id = "3450756714426841564";

        let mock_storage = MockStorageProvider::new();
        mock_storage.add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-1-0-0.parquet", group_id),
            "file1",
        );
        mock_storage.add_mapping_file(
            file_sha,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-1-0-0.parquet", group_id),
        );
        server
            .mock(
                "POST",
                "/validategroup/dataset/samplingn-3450756714426841564-1-0.parquet",
            )
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "file_shas": [file_sha],
                "group_id": group_id,
                "file_number": 0,
                "group_size": 1
            })))
            .with_status(200)
            .with_body(r#"ok"#)
            .create();
        server
            .mock(
                "GET",
                "/statusgroup/dataset/samplingn-3450756714426841564-1-0.parquet",
            )
            .with_status(200)
            .with_body(r#"{"status": "accept"}"#)
            .create();

        let storage_provider = Arc::new(mock_storage);
        let metrics_context = MetricsContext::new("0".to_string(), Some("0".to_string()));

        let validator = SyntheticDataValidator::new(
            "0".to_string(),
            contracts.synthetic_data_validator.clone().unwrap(),
            contracts.prime_network.clone(),
            vec![config],
            U256::from(0),
            storage_provider,
            store,
            CancellationToken::new(),
            10,
            60,
            1,
            10,
            true,
            false,
            Some(metrics_context),
        );

        let work_keys: Vec<String> = vec![file_sha.to_string()];
        let work_keys_2: Vec<String> = work_keys.clone();
        let work_keys_3: Vec<String> = work_keys.clone();

        let work_info = WorkInfo {
            node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ..Default::default()
        };
        for work_key in work_keys.clone() {
            validator
                .update_work_info_in_redis(&work_key, &work_info)
                .await?;
        }

        let plan = validator.build_validation_plan(work_keys).await?;
        assert_eq!(plan.group_trigger_tasks.len(), 1);
        assert_eq!(plan.group_trigger_tasks[0].group_id, group_id);
        let metrics_0 = export_metrics().unwrap();
        assert!(metrics_0
            .contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 1"));

        let group = validator.get_group(file_sha).await?;
        assert!(group.is_some());
        let group = group.unwrap();
        assert_eq!(group.group_id, group_id);
        assert_eq!(group.group_size, 1);
        assert_eq!(group.file_number, 0);

        let result = validator.process_group_task(group).await;
        assert!(result.is_ok());

        let cache_status = validator
            .get_work_validation_status_from_redis(file_sha)
            .await?;
        assert_eq!(cache_status, Some(ValidationResult::Unknown));

        let plan_2 = validator.build_validation_plan(work_keys_2).await?;
        assert_eq!(plan_2.group_trigger_tasks.len(), 0);
        assert_eq!(plan_2.group_status_check_tasks.len(), 1);

        let metrics = export_metrics().unwrap();
        assert!(
            metrics.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 1")
        );

        let result = validator
            .process_group_status_check(plan_2.group_status_check_tasks[0].clone())
            .await;
        assert!(result.is_ok());

        let cache_status = validator
            .get_work_validation_status_from_redis(file_sha)
            .await?;
        assert_eq!(cache_status, Some(ValidationResult::Accept));

        let plan_3 = validator.build_validation_plan(work_keys_3).await?;
        assert_eq!(plan_3.group_trigger_tasks.len(), 0);
        assert_eq!(plan_3.group_status_check_tasks.len(), 0);
        let metrics_2 = export_metrics().unwrap();
        assert!(metrics_2
            .contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 0"));
        assert!(metrics_2.contains("toploc_config_name=\"Qwen/Qwen0.6\""));

        Ok(())
    }

    #[tokio::test]
    async fn test_process_group_status_check_reject() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        // Mock the group status check endpoint to return a reject status
        // Toploc server runs model Qwen3
        let _status_mock = server
            .mock(
                "GET",
                "/statusgroup/dataset/samplingn-3450756714426841564-1-9.parquet",
            )
            .with_status(200)
            .with_body(r#"{"status": "reject", "flops": 0.0, "failing_indices": [0]}"#)
            .create();

        let (store, contracts) = setup_test_env()?;

        let config = ToplocConfig {
            server_url: server.url(),
            file_prefix_filter: Some("Qwen3".to_string()),
            ..Default::default()
        };

        let mock_storage = MockStorageProvider::new();
        mock_storage.add_file(
            "Qwen3/dataset/samplingn-3450756714426841564-1-9-0.parquet",
            "file1",
        );
        mock_storage.add_mapping_file(
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            "Qwen3/dataset/samplingn-3450756714426841564-1-9-0.parquet",
        );

        let storage_provider = Arc::new(mock_storage);

        let validator = SyntheticDataValidator::new(
            "0".to_string(),
            contracts.synthetic_data_validator.clone().unwrap(),
            contracts.prime_network.clone(),
            vec![config],
            U256::from(0),
            storage_provider,
            store,
            CancellationToken::new(),
            10,
            60,
            1,
            10,
            true,
            true,
            None,
        );

        let group = validator
            .get_group("c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641")
            .await?;
        let group = group.unwrap();

        // Process the group status check
        let result = validator.process_group_status_check(group).await;
        assert!(result.is_ok());

        // Verify that the work was invalidated
        let work_info = validator
            .get_work_info_from_redis(
                "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            )
            .await?;
        assert!(work_info.is_none(), "Work should be invalidated");

        Ok(())
    }
}
