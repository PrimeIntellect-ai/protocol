use crate::metrics::MetricsContext;
use crate::store::redis::RedisStore;
use alloy::primitives::U256;
use anyhow::{Context as _, Error, Result};
use chrono;
use futures::future;
use log::{debug, warn};
use log::{error, info};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use shared::utils::StorageProvider;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::{
    SyntheticDataWorkValidator, WorkInfo,
};
use shared::web3::wallet::WalletProvider;
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
pub mod toploc;
use toploc::{GroupValidationResult, Toploc, ToplocConfig};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum ValidationResult {
    Accept,
    Reject,
    Crashed,
    Pending,
    Unknown,
    Invalidated,
    IncompleteGroup,
    FileNameResolutionFailed,
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
            ValidationResult::IncompleteGroup => write!(f, "incomplete_group"),
            ValidationResult::FileNameResolutionFailed => write!(f, "filename_resolution_failed"),
        }
    }
}

#[derive(Debug)]
pub enum ProcessWorkKeyError {
    FileNameResolutionError(String),
    ValidationTriggerError(String),
    ValidationPollingError(String),
    InvalidatingWorkError(String),
    MaxAttemptsReached(String),
    GenericError(anyhow::Error),
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
    group_file_name: String,
    prefix: String,
    group_id: String,
    group_size: u32,
    file_number: u32,
    idx: String,
}
impl FromStr for GroupInformation {
    type Err = Error;

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

#[derive(Debug)]
pub struct ValidationPlan {
    pub single_trigger_tasks: Vec<(String, WorkInfo)>,
    pub group_trigger_tasks: Vec<ToplocGroup>,
    pub status_check_tasks: Vec<String>,
    pub group_status_check_tasks: Vec<ToplocGroup>,
}

#[derive(Clone)]
pub struct SyntheticDataValidator<P: alloy::providers::Provider + Clone> {
    pool_id: U256,
    validator: SyntheticDataWorkValidator<P>,
    prime_network: PrimeNetworkContract<P>,
    toploc: Vec<Toploc>,
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

    /// **Storage**: Uses Redis sorted set "incomplete_groups" with deadline as score
    incomplete_group_grace_period_minutes: u64,
    metrics: Option<MetricsContext>,
}

impl<P: alloy::providers::Provider + Clone + 'static> SyntheticDataValidator<P> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool_id_str: String,
        validator: SyntheticDataWorkValidator<P>,
        prime_network: PrimeNetworkContract<P>,
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
        incomplete_group_grace_period_minutes: u64,
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
            incomplete_group_grace_period_minutes,
            metrics,
        }
    }

    fn get_key_for_work_key(&self, work_key: &str) -> String {
        format!("work_validation_status:{}", work_key)
    }

    /// Starts tracking an incomplete group with a grace period for recovery.
    ///
    /// **Purpose**: When a group doesn't have all expected submissions yet, we give it
    /// a configurable grace period (e.g., 5 minutes) to allow remaining nodes to submit
    /// their work before we soft invalidate the incomplete submissions.
    ///
    /// **How it works**:
    /// - Adds the group to a Redis sorted set called "incomplete_groups"
    /// - Uses the "grace period deadline" as the sort score (not Redis TTL!)
    /// - Only tracks if the group exists and isn't already being tracked
    /// - The deadline is set ONLY ONCE when first tracking begins
    ///
    async fn track_incomplete_group(&self, group_key: &str) -> Result<(), Error> {
        if self.incomplete_group_grace_period_minutes == 0 {
            return Ok(()); // Feature disabled
        }

        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        // Only track groups that actually exist in Redis (have at least one submission)
        let group_exists: bool = con
            .exists(group_key)
            .await
            .map_err(|e| Error::msg(format!("Failed to check group existence: {}", e)))?;

        if !group_exists {
            // No point tracking a group that doesn't exist yet
            return Ok(());
        }

        // Calculate when the grace period ends - this will only be used if not already tracked
        let grace_period_deadline = chrono::Utc::now().timestamp()
            + (self.incomplete_group_grace_period_minutes as i64 * 60);

        // Add to sorted set ONLY if not already present
        // NX flag ensures the deadline is set exactly once when tracking begins
        let mut cmd = redis::cmd("ZADD");
        cmd.arg("incomplete_groups")
            .arg("NX")
            .arg(grace_period_deadline)
            .arg(group_key);

        let added: i32 = cmd
            .query_async(&mut con)
            .await
            .map_err(|e| Error::msg(format!("Failed to track incomplete group: {}", e)))?;

        if added > 0 {
            debug!(
                "Started tracking incomplete group: {} with {}min grace period (deadline: {})",
                group_key, self.incomplete_group_grace_period_minutes, grace_period_deadline
            );
        } else {
            debug!(
                "Group {} already being tracked, preserving original deadline",
                group_key
            );
        }

        Ok(())
    }
}

impl SyntheticDataValidator<WalletProvider> {
    pub async fn soft_invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        info!("Soft invalidating work: {}", work_key);

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work soft invalidation");
            return Ok(());
        }

        // Special case for tests - skip actual blockchain interaction
        #[cfg(test)]
        {
            info!("Test mode: skipping actual work soft invalidation");
            let _ = &self.prime_network;
            Ok(())
        }

        #[cfg(not(test))]
        {
            let work_info = self
                .get_work_info_from_redis(work_key)
                .await?
                .ok_or_else(|| Error::msg("Work info not found for soft invalidation"))?;
            let work_key_bytes = hex::decode(work_key)
                .map_err(|e| Error::msg(format!("Failed to decode hex work key: {}", e)))?;

            // Create 64-byte payload: work_key (32 bytes) + work_units (32 bytes)
            let mut data = Vec::with_capacity(64);
            data.extend_from_slice(&work_key_bytes);

            // Convert work_units to 32-byte representation
            let work_units_bytes = work_info.work_units.to_be_bytes::<32>();
            data.extend_from_slice(&work_units_bytes);

            match self
                .prime_network
                .soft_invalidate_work(self.pool_id, data)
                .await
            {
                Ok(_) => Ok(()),
                Err(e) => {
                    error!("Failed to soft invalidate work {}: {}", work_key, e);
                    Err(Error::msg(format!("Failed to soft invalidate work: {}", e)))
                }
            }
        }
    }
    /// Finds groups whose grace period has ended and cleans them up.
    async fn get_groups_past_grace_period(&self) -> Result<Vec<String>, Error> {
        if self.incomplete_group_grace_period_minutes == 0 {
            return Ok(Vec::new()); // Feature disabled
        }

        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        let current_timestamp = chrono::Utc::now().timestamp();

        // Get all groups whose grace period deadline has passed
        // ZRANGEBYSCORE returns members with score between -infinity and current_timestamp
        let groups_past_deadline: Vec<String> = con
            .zrangebyscore("incomplete_groups", "-inf", current_timestamp)
            .await
            .map_err(|e| Error::msg(format!("Failed to get groups past grace period: {}", e)))?;

        // Remove these groups from tracking since their grace period is over
        if !groups_past_deadline.is_empty() {
            let _: () = con
                .zrem("incomplete_groups", &groups_past_deadline)
                .await
                .map_err(|e| Error::msg(format!("Failed to remove groups from tracking: {}", e)))?;

            debug!(
                "Found {} groups past their grace period",
                groups_past_deadline.len()
            );
        }

        Ok(groups_past_deadline)
    }

    /// Checks if a group is currently being tracked for incomplete recovery.
    ///Used in tests and debugging to verify that incomplete groups
    #[cfg(test)]
    async fn is_group_being_tracked_as_incomplete(&self, group_key: &str) -> Result<bool, Error> {
        if self.incomplete_group_grace_period_minutes == 0 {
            return Ok(false); // Feature disabled
        }

        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        // Check if group exists in the incomplete groups sorted set
        // ZSCORE returns the score (deadline) if present, or None if not tracked
        let score: Option<f64> = con
            .zscore("incomplete_groups", group_key)
            .await
            .map_err(|e| Error::msg(format!("Failed to check incomplete tracking: {}", e)))?;
        Ok(score.is_some())
    }

    /// Updates the grace period deadline for a tracked incomplete group.
    /// Used in tests to simulate time passing by setting deadline relative to current time.
    ///
    /// # Arguments
    /// * `group_key` - The group to update
    /// * `minutes_from_now` - Minutes from current timestamp (negative for past, positive for future)
    #[cfg(test)]
    async fn update_incomplete_group_deadline_relative(
        &self,
        group_key: &str,
        minutes_from_now: i64,
    ) -> Result<(), Error> {
        if self.incomplete_group_grace_period_minutes == 0 {
            return Ok(()); // Feature disabled
        }

        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        let current_timestamp = chrono::Utc::now().timestamp();
        let new_deadline = current_timestamp + (minutes_from_now * 60);

        // Update the score (deadline) for the group in the sorted set
        let _: () = con
            .zadd("incomplete_groups", group_key, new_deadline)
            .await
            .map_err(|e| {
                Error::msg(format!("Failed to update incomplete group deadline: {}", e))
            })?;

        debug!(
            "Updated deadline for incomplete group {} to {} ({} minutes from now)",
            group_key, new_deadline, minutes_from_now
        );

        Ok(())
    }

    /// Stops tracking a group (called when group becomes complete or is invalidated).
    async fn remove_incomplete_group_tracking(&self, group_key: &str) -> Result<(), Error> {
        if self.incomplete_group_grace_period_minutes == 0 {
            return Ok(()); // Feature disabled
        }

        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        // Remove the group from the incomplete groups sorted set
        let removed: i32 = con
            .zrem("incomplete_groups", group_key)
            .await
            .map_err(|e| {
                Error::msg(format!("Failed to remove incomplete group tracking: {}", e))
            })?;

        if removed > 0 {
            debug!("Stopped tracking incomplete group: {}", group_key);
        }

        Ok(())
    }

    async fn update_work_validation_status(
        &self,
        work_key: &str,
        status: &ValidationResult,
    ) -> Result<(), Error> {
        let expiry = match status {
            ValidationResult::Unknown => self.unknown_status_expiry_seconds,
            _ => 0,
        };
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let key = self.get_key_for_work_key(work_key);
        let status = serde_json::to_string(&status)?;
        if expiry > 0 {
            let _: () = con
                .set_options(
                    &key,
                    status,
                    redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(expiry)),
                )
                .await
                .map_err(|e| Error::msg(format!("Failed to set work validation status: {}", e)))?;
        } else {
            let _: () = con
                .set(&key, status)
                .await
                .map_err(|e| Error::msg(format!("Failed to set work validation status: {}", e)))?;
        }
        Ok(())
    }

    #[cfg(test)]
    async fn get_work_validation_status_from_redis(
        &self,
        work_key: &str,
    ) -> Result<Option<ValidationResult>, Error> {
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let key = self.get_key_for_work_key(work_key);
        let status: Option<String> = con
            .get(key)
            .await
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
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let key = format!("work_info:{}", work_key);
        let work_info = serde_json::to_string(&work_info)?;
        let _: () = con
            .set(&key, work_info)
            .await
            .map_err(|e| Error::msg(format!("Failed to set work info: {}", e)))?;
        Ok(())
    }

    async fn get_work_info_from_redis(&self, work_key: &str) -> Result<Option<WorkInfo>, Error> {
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let key = format!("work_info:{}", work_key);
        let work_info: Option<String> = con
            .get(&key)
            .await
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
        let attempts_key = format!("file_name_attempts:{}", work_key);
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;

        let file_name: Option<String> = con
            .get(&redis_key)
            .await
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;
        if let Some(cached_file_name) = file_name {
            return Ok(cached_file_name);
        }

        // Increment attempts counter
        let attempts: i64 = con
            .incr(&attempts_key, 1)
            .await
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;

        // Set expiry on attempts counter (24 hours)
        let _: () = con
            .expire(&attempts_key, 24 * 60 * 60)
            .await
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;

        const MAX_ATTEMPTS: i64 = 60;
        if attempts >= MAX_ATTEMPTS {
            // If we've tried too many times, soft invalidate the work and update its status
            if let Err(e) = self.soft_invalidate_work(work_key).await {
                error!(
                    "Failed to soft invalidate work after max filename resolution attempts: {}",
                    e
                );
            }
            // Set the validation status to FileNameResolutionFailed to prevent future processing
            if let Err(e) = self
                .update_work_validation_status(
                    work_key,
                    &ValidationResult::FileNameResolutionFailed,
                )
                .await
            {
                error!(
                    "Failed to update validation status after max attempts: {}",
                    e
                );
            }
            return Err(ProcessWorkKeyError::MaxAttemptsReached(format!(
                "Failed to resolve filename after {} attempts for work key: {}",
                MAX_ATTEMPTS, work_key
            )));
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

        let _: () = con
            .set(&redis_key, cleaned_file_name)
            .await
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;

        // Reset attempts counter on success
        let _: () = con
            .del(&attempts_key)
            .await
            .map_err(|e| ProcessWorkKeyError::GenericError(e.into()))?;

        Ok(cleaned_file_name.to_string())
    }

    async fn build_group_for_key(&self, work_key: &str) -> Result<String, Error> {
        let file = match self.get_file_name_for_work_key(work_key).await {
            Ok(name) => name,
            Err(ProcessWorkKeyError::MaxAttemptsReached(_)) => {
                // Status is already set in get_file_name_for_work_key
                return Err(Error::msg(format!(
                    "Failed to resolve filename after max attempts for work key: {}",
                    work_key
                )));
            }
            Err(e) => {
                error!("Failed to get file name for work key: {}", e);
                return Err(Error::msg(format!(
                    "Failed to get file name for work key: {}",
                    e
                )));
            }
        };

        let group_info = GroupInformation::from_str(&file)?;
        let group_key: String = format!(
            "group:{}:{}:{}",
            group_info.group_id, group_info.group_size, group_info.file_number
        );
        let mut redis = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        redis
            .hset::<_, _, _, ()>(&group_key, work_key, group_info.to_redis()?)
            .await
            .map_err(|e| Error::msg(format!("Failed to set group info in Redis: {}", e)))?;

        Ok(group_key)
    }

    async fn is_group_ready_for_validation(&self, group_key: &str) -> Result<bool, Error> {
        let mut redis = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let group_size: u32 = redis
            .hlen::<_, u32>(group_key)
            .await
            .map_err(|e| Error::msg(format!("Failed to get group size from Redis: {}", e)))?;

        let expected_size = group_key
            .split(':')
            .nth(2)
            .ok_or_else(|| Error::msg("Failed to get group size from group key"))?
            .parse::<u32>()
            .map_err(|e| Error::msg(format!("Failed to parse group size: {}", e)))?;

        Ok(group_size == expected_size)
    }

    async fn get_group(&self, work_key: &str) -> Result<Option<ToplocGroup>> {
        let group_key = match self.build_group_for_key(work_key).await {
            Ok(key) => key,
            Err(e) => {
                error!("Failed to build group key for work key {}: {}", work_key, e);
                return Ok(None);
            }
        };
        let ready_for_validation = self.is_group_ready_for_validation(&group_key).await?;
        if ready_for_validation {
            // Remove from incomplete group tracking since it's now complete
            if let Err(e) = self.remove_incomplete_group_tracking(&group_key).await {
                error!("Failed to remove incomplete group tracking: {}", e);
            }
            let mut redis = self
                .redis_store
                .client
                .get_multiplexed_async_connection()
                .await?;
            let group_entries: HashMap<String, String> = redis.hgetall(&group_key).await?;
            let mut entries: Vec<(String, GroupInformation)> = Vec::new();
            for (key, value) in group_entries {
                let info = GroupInformation::from_redis(&value)
                    .map_err(|e| Error::msg(format!("Failed to parse group info: {}", e)))?;
                entries.push((key, info));
            }

            entries.sort_by_key(|(_, info)| info.idx.parse::<usize>().unwrap_or(0));

            let mut toploc_group: ToplocGroup = entries
                .first()
                .ok_or_else(|| Error::msg("No group info found"))?
                .1
                .clone()
                .into();

            toploc_group.sorted_work_keys = entries.into_iter().map(|(key, _)| key).collect();

            return Ok(Some(toploc_group));
        } else {
            // Track incomplete group for potential soft invalidation
            if let Err(e) = self.track_incomplete_group(&group_key).await {
                error!("Failed to track incomplete group: {}", e);
            }
        }
        Ok(None)
    }

    pub async fn build_validation_plan(
        &self,
        work_keys: Vec<String>,
    ) -> Result<ValidationPlan, Error> {
        self.build_validation_plan_batched(work_keys).await
    }

    pub async fn build_validation_plan_batched(
        &self,
        work_keys: Vec<String>,
    ) -> Result<ValidationPlan, Error> {
        let mut single_trigger_tasks: Vec<(String, WorkInfo)> = Vec::new();
        let mut group_trigger_tasks: Vec<ToplocGroup> = Vec::new();
        let mut status_check_tasks: Vec<String> = Vec::new();
        let mut group_status_check_tasks: Vec<ToplocGroup> = Vec::new();

        let mut keys_to_process = 0;

        // Step 1: Batch fetch work info and validation status from Redis
        let (work_info_map, status_map) = self.batch_fetch_redis_data(&work_keys).await?;

        // Step 2: Collect work keys that need blockchain lookup
        let mut missing_work_keys = Vec::new();
        for work_key in &work_keys {
            if !work_info_map.contains_key(work_key) {
                missing_work_keys.push(work_key.clone());
            }
        }

        // Step 3: Batch fetch missing work info from blockchain
        let blockchain_work_info = self
            .batch_fetch_blockchain_work_info(missing_work_keys)
            .await?;

        // Step 4: Process all work keys with cached + fetched data
        for work_key in work_keys {
            // Get work info - either from cache or blockchain fetch
            let work_info = if let Some(info) = work_info_map.get(&work_key) {
                info
            } else if let Some(info) = blockchain_work_info.get(&work_key) {
                // Cache the fetched work info asynchronously
                let self_clone = self.clone();
                let work_key_clone = work_key.clone();
                let info_copy = *info;
                tokio::spawn(async move {
                    if let Err(e) = self_clone
                        .update_work_info_in_redis(&work_key_clone, &info_copy)
                        .await
                    {
                        error!("Failed to cache work info for {}: {}", work_key_clone, e);
                    }
                });
                info
            } else {
                error!("Failed to get work info for {}", work_key);
                continue;
            };

            let cache_status = status_map.get(&work_key);

            match cache_status {
                Some(
                    ValidationResult::Accept
                    | ValidationResult::Reject
                    | ValidationResult::Crashed
                    | ValidationResult::IncompleteGroup,
                ) => {
                    continue; // Already processed
                }
                Some(ValidationResult::Unknown) => {
                    keys_to_process += 1;
                    if self.with_node_grouping {
                        let check_group = self.get_group(&work_key).await?;
                        if let Some(group) = check_group {
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
                        let check_group = self.get_group(&work_key).await?;
                        if let Some(group) = check_group {
                            group_trigger_tasks.push(group);
                        }
                    } else {
                        single_trigger_tasks.push((work_key.clone(), *work_info));
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

    /// Batch fetch work info and validation status from Redis using pipelining
    async fn batch_fetch_redis_data(
        &self,
        work_keys: &[String],
    ) -> Result<(HashMap<String, WorkInfo>, HashMap<String, ValidationResult>), Error> {
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        let mut pipe = redis::pipe();

        for work_key in work_keys {
            let work_info_key = format!("work_info:{}", work_key);
            pipe.get(&work_info_key);
        }

        for work_key in work_keys {
            let status_key = self.get_key_for_work_key(work_key);
            pipe.get(status_key);
        }

        let results: Vec<Option<String>> = pipe
            .query_async(&mut con)
            .await
            .map_err(|e| Error::msg(format!("Failed to execute Redis pipeline: {}", e)))?;

        let mut work_info_map = HashMap::new();
        let mut status_map = HashMap::new();

        let work_keys_len = work_keys.len();

        for (i, work_key) in work_keys.iter().enumerate() {
            if let Some(Some(work_info_str)) = results.get(i) {
                match serde_json::from_str::<WorkInfo>(work_info_str) {
                    Ok(work_info) => {
                        work_info_map.insert(work_key.clone(), work_info);
                    }
                    Err(e) => {
                        debug!("Failed to parse work info for {}: {}", work_key, e);
                    }
                }
            }
        }

        for (i, work_key) in work_keys.iter().enumerate() {
            if let Some(Some(status_str)) = results.get(i + work_keys_len) {
                match serde_json::from_str::<ValidationResult>(status_str) {
                    Ok(status) => {
                        status_map.insert(work_key.clone(), status);
                    }
                    Err(e) => {
                        debug!("Failed to parse validation status for {}: {}", work_key, e);
                    }
                }
            }
        }

        Ok((work_info_map, status_map))
    }

    /// Batch fetch work info from blockchain for multiple work keys
    async fn batch_fetch_blockchain_work_info(
        &self,
        work_keys: Vec<String>,
    ) -> Result<HashMap<String, WorkInfo>, Error> {
        if work_keys.is_empty() {
            return Ok(HashMap::new());
        }

        let futures: Vec<_> = work_keys
            .into_iter()
            .map(|work_key| {
                let validator = self.validator.clone();
                let pool_id = self.pool_id;
                async move {
                    match validator.get_work_info(pool_id, &work_key).await {
                        Ok(work_info) => Some((work_key, work_info)),
                        Err(e) => {
                            error!("Failed to get work info for {}: {}", work_key, e);
                            None
                        }
                    }
                }
            })
            .collect();

        let results = future::join_all(futures).await;

        Ok(results.into_iter().flatten().collect())
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

        for work_key in &group.sorted_work_keys {
            self.update_work_validation_status(work_key, &ValidationResult::Unknown)
                .await?;
        }

        Ok(())
    }

    async fn calculate_group_work_units(
        &self,
        group: &ToplocGroup,
    ) -> Result<(U256, std::collections::HashMap<String, U256>), Error> {
        let mut total_claimed_units: U256 = U256::from(0);
        let mut node_work_units: std::collections::HashMap<String, U256> =
            std::collections::HashMap::new();

        for work_key in &group.sorted_work_keys {
            if let Some(work_info) = self.get_work_info_from_redis(work_key).await? {
                total_claimed_units += work_info.work_units;
                node_work_units.insert(work_info.node_id.to_string(), work_info.work_units);
            }
        }

        Ok((total_claimed_units, node_work_units))
    }

    async fn handle_group_toploc_rejection(
        &self,
        group: &ToplocGroup,
        status: &GroupValidationResult,
    ) -> Result<Vec<String>, Error> {
        let indices = &status.failing_indices;
        let work_keys_to_invalidate: Vec<String> = indices
            .iter()
            .map(|&idx| group.sorted_work_keys[idx as usize].clone())
            .collect();

        warn!(
            "Group {} rejected - Invalidating keys: {:?}",
            group.group_id, work_keys_to_invalidate
        );

        let mut rejected_nodes = Vec::new();
        for work_key in work_keys_to_invalidate {
            if let Some(work_info) = self.get_work_info_from_redis(&work_key).await? {
                rejected_nodes.push(work_info.node_id.to_string());
            }
        }

        Ok(rejected_nodes)
    }

    async fn handle_group_toploc_acceptance(
        &self,
        group: &ToplocGroup,
        status: &GroupValidationResult,
        total_claimed_units: U256,
        node_work_units: &std::collections::HashMap<String, U256>,
        toploc_config_name: &str,
    ) -> Result<Vec<String>, Error> {
        let output_flops_u256 = U256::from(status.output_flops as u64);
        let diff = if total_claimed_units > output_flops_u256 {
            total_claimed_units - output_flops_u256
        } else {
            output_flops_u256 - total_claimed_units
        };
        let work_units_match = diff <= U256::from(1);

        if let Some(metrics) = &self.metrics {
            metrics.record_group_work_units_check_result(
                &group.group_id,
                toploc_config_name,
                if work_units_match {
                    "match"
                } else {
                    "mismatch"
                },
            );
        }

        if !work_units_match {
            let nodes_with_wrong_work_unit_claims = self
                .handle_work_units_mismatch(
                    group,
                    status,
                    total_claimed_units,
                    node_work_units,
                    output_flops_u256,
                )
                .await?;
            Ok(nodes_with_wrong_work_unit_claims)
        } else {
            Ok(Vec::new())
        }
    }

    async fn handle_work_units_mismatch(
        &self,
        group: &ToplocGroup,
        status: &GroupValidationResult,
        total_claimed_units: U256,
        node_work_units: &std::collections::HashMap<String, U256>,
        output_flops_u256: U256,
    ) -> Result<Vec<String>, Error> {
        let num_nodes = node_work_units.len() as u64;
        let expected_work_units_per_node = if num_nodes > 0 {
            output_flops_u256 / U256::from(num_nodes)
        } else {
            U256::from(0)
        };

        let mut nodes_with_wrong_work_unit_claims = Vec::new();
        for (node_address, work_units) in node_work_units {
            let node_diff = if *work_units > expected_work_units_per_node {
                *work_units - expected_work_units_per_node
            } else {
                expected_work_units_per_node - *work_units
            };
            if node_diff > U256::from(1) {
                nodes_with_wrong_work_unit_claims.push(node_address.clone());
            }
        }

        warn!(
            "Group {} units mismatch - Claimed: {}, Toploc: {}, Expected per node: {}, Keys: {:?}, Nodes with wrong work unit claims: {:?}",
            group.group_id,
            total_claimed_units,
            status.output_flops,
            expected_work_units_per_node,
            group.sorted_work_keys,
            nodes_with_wrong_work_unit_claims
        );

        Ok(nodes_with_wrong_work_unit_claims)
    }
}

impl SyntheticDataValidator<WalletProvider> {
    pub async fn validate_work(self) -> Result<(), Error> {
        debug!("Validating work for pool ID: {:?}", self.pool_id);

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
        // Process incomplete groups past their grace period before handling new work
        if let Err(e) = self.process_groups_past_grace_period().await {
            error!("Failed to process groups past grace period: {}", e);
        }

        self.process_work_keys(work_keys).await
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

        let (total_claimed_units, node_work_units) =
            self.calculate_group_work_units(&group).await?;

        info!(
            "Group {} ({}) - Status: {:?}, Claimed: {}, Toploc: {} flops (input: {} flops)",
            group.group_id,
            group.group_file_name,
            status.status,
            total_claimed_units,
            status.output_flops,
            status.input_flops
        );

        let mut nodes_to_invalidate = Vec::new();

        if status.status == ValidationResult::Reject {
            let rejected_nodes = self.handle_group_toploc_rejection(&group, &status).await?;
            nodes_to_invalidate.extend(rejected_nodes);
        } else if status.status == ValidationResult::Accept {
            let nodes_with_wrong_work_unit_claims = self
                .handle_group_toploc_acceptance(
                    &group,
                    &status,
                    total_claimed_units,
                    &node_work_units,
                    &toploc_config_name,
                )
                .await?;
            nodes_to_invalidate.extend(nodes_with_wrong_work_unit_claims);
        }

        if !nodes_to_invalidate.is_empty() {
            for work_key in &group.sorted_work_keys {
                if let Some(work_info) = self.get_work_info_from_redis(work_key).await? {
                    if nodes_to_invalidate.contains(&work_info.node_id.to_string()) {
                        self.invalidate_work(work_key).await?;
                    }
                }
            }
        }

        for work_key in &group.sorted_work_keys {
            let work_info = self.get_work_info_from_redis(work_key).await?;
            if let Some(work_info) = work_info {
                if nodes_to_invalidate.contains(&work_info.node_id.to_string()) {
                    self.update_work_validation_status(work_key, &ValidationResult::Reject)
                        .await?;
                } else {
                    self.update_work_validation_status(work_key, &status.status)
                        .await?;
                }
            }
        }

        Ok(())
    }

    pub async fn process_work_keys(self, work_keys: Vec<String>) -> Result<(), Error> {
        let validation_plan = self.build_validation_plan(work_keys).await?;

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

    pub async fn invalidate_work(&self, work_key: &str) -> Result<(), Error> {
        info!("Invalidating work: {}", work_key);

        if let Some(metrics) = &self.metrics {
            metrics.record_work_key_invalidation();
        }

        if self.disable_chain_invalidation {
            info!("Chain invalidation is disabled, skipping work invalidation");
            return Ok(());
        }

        #[cfg(test)]
        {
            info!("Test mode: skipping actual work invalidation");
            let _ = &self.prime_network;
            let _ = &self.penalty;
            Ok(())
        }

        #[cfg(not(test))]
        {
            let data = hex::decode(work_key)
                .map_err(|e| Error::msg(format!("Failed to decode hex work key: {}", e)))?;
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
    }

    /// Main entry point for processing incomplete groups that have exceeded their grace period.
    ///
    /// **Purpose**: This is the "cleanup job" that runs periodically to handle incomplete
    /// groups whose grace period has ended. It soft invalidates work from groups that
    /// never received all expected submissions.
    ///
    /// **Flow**:
    /// 1. Find groups whose grace period deadline has passed
    /// 2. Double-check they're still incomplete (race condition protection)
    /// 3. Soft invalidate all work keys in those groups
    /// 4. Update work status to "IncompleteGroup" (specific reason for soft invalidation)
    /// 5. Stop tracking the groups
    pub async fn process_groups_past_grace_period(&self) -> Result<(), Error> {
        if self.incomplete_group_grace_period_minutes == 0 {
            return Ok(()); // Feature disabled
        }

        debug!("Checking for incomplete groups past their grace period");

        // Get all group keys that have passed their grace period deadline
        let expired_group_keys = self.get_groups_past_grace_period().await?;

        if expired_group_keys.is_empty() {
            return Ok(());
        }

        info!(
            "Found {} incomplete groups past their grace period - will soft invalidate",
            expired_group_keys.len()
        );

        for group_key in expired_group_keys {
            // Double-check the group is still incomplete before invalidating
            // (it might have completed just before the deadline - race condition protection)
            let still_incomplete = !self.is_group_ready_for_validation(&group_key).await?;

            if still_incomplete {
                // Get all work keys in this incomplete group and soft invalidate them

                if let Some(metrics) = &self.metrics {
                    metrics.record_group_soft_invalidation(&group_key);
                }

                match self.get_group_work_keys_from_redis(&group_key).await {
                    Ok(work_keys) => {
                        warn!("Soft invalidating {} work keys from incomplete group past grace period: {}", 
                              work_keys.len(), group_key);

                        for work_key in work_keys {
                            // Soft invalidate (less penalty than hard invalidation)
                            if let Err(e) = self.soft_invalidate_work(&work_key).await {
                                error!("Failed to soft invalidate work key {}: {}", work_key, e);
                            } else {
                                // Mark work as soft invalidated due to incomplete group
                                if let Err(e) = self
                                    .update_work_validation_status(
                                        &work_key,
                                        &ValidationResult::IncompleteGroup,
                                    )
                                    .await
                                {
                                    error!(
                                        "Failed to update work validation status for {}: {}",
                                        work_key, e
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to get work keys for incomplete group {}: {}",
                            group_key, e
                        );
                    }
                }
            } else {
                info!(
                    "Group {} completed just before grace period deadline - not invalidating",
                    group_key
                );
            }

            // Always stop tracking the group (whether we invalidated it or it completed)
            if let Err(e) = self.remove_incomplete_group_tracking(&group_key).await {
                error!("Failed to stop tracking group {}: {}", group_key, e);
            }
        }

        Ok(())
    }

    async fn get_group_work_keys_from_redis(&self, group_key: &str) -> Result<Vec<String>, Error> {
        let mut redis = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        let work_keys: Vec<String> = redis
            .hkeys(group_key)
            .await
            .map_err(|e| Error::msg(format!("Failed to get work keys from group: {}", e)))?;

        Ok(work_keys)
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

    fn setup_test_env() -> Result<(RedisStore, Contracts<WalletProvider>), Error> {
        let store = test_store();
        let url = Url::parse("http://localhost:8545").unwrap();

        let demo_wallet = Wallet::new(
            "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
            url,
        )
        .map_err(|e| Error::msg(format!("Failed to create demo wallet: {}", e)))?;

        let contracts = ContractBuilder::new(demo_wallet.provider())
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
        let (store, contracts) = setup_test_env()?;
        let metrics_context = MetricsContext::new("0".to_string(), Some("0".to_string()));
        let mock_storage = MockStorageProvider::new();

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
            0, // incomplete_group_grace_period_minutes (disabled)
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
            0, // incomplete_group_grace_period_minutes (disabled)
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
            0, // incomplete_group_grace_period_minutes (disabled)
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

        const FILE_SHA: &str = "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641";
        const GROUP_ID: &str = "3450756714426841564";
        const NODE_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

        let mock_storage = MockStorageProvider::new();
        mock_storage.add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-1-0-0.parquet", GROUP_ID),
            "file1",
        );
        mock_storage.add_mapping_file(
            FILE_SHA,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-1-0-0.parquet", GROUP_ID),
        );
        server
            .mock(
                "POST",
                format!("/validategroup/dataset/samplingn-{}-1-0.parquet", GROUP_ID).as_str(),
            )
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "file_shas": [FILE_SHA],
                "group_id": GROUP_ID,
                "file_number": 0,
                "group_size": 1
            })))
            .with_status(200)
            .with_body(r#"ok"#)
            .create();
        server
            .mock(
                "GET",
                format!("/statusgroup/dataset/samplingn-{}-1-0.parquet", GROUP_ID).as_str(),
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
            0, // incomplete_group_grace_period_minutes (disabled)
            Some(metrics_context),
        );

        let work_keys: Vec<String> = vec![FILE_SHA.to_string()];

        let work_info = WorkInfo {
            node_id: Address::from_str(NODE_ADDRESS).unwrap(),
            ..Default::default()
        };
        for work_key in work_keys.clone() {
            validator
                .update_work_info_in_redis(&work_key, &work_info)
                .await?;
        }

        let plan = validator.build_validation_plan(work_keys.clone()).await?;
        assert_eq!(plan.group_trigger_tasks.len(), 1);
        assert_eq!(plan.group_trigger_tasks[0].group_id, GROUP_ID);
        let metrics_0 = export_metrics().unwrap();
        assert!(metrics_0
            .contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 1"));

        let group = validator.get_group(FILE_SHA).await?;
        assert!(group.is_some());
        let group = group.unwrap();
        assert_eq!(group.group_id, GROUP_ID);
        assert_eq!(group.group_size, 1);
        assert_eq!(group.file_number, 0);

        let result = validator.process_group_task(group).await;
        assert!(result.is_ok());

        let cache_status = validator
            .get_work_validation_status_from_redis(FILE_SHA)
            .await?;
        assert_eq!(cache_status, Some(ValidationResult::Unknown));

        let plan_2 = validator.build_validation_plan(work_keys.clone()).await?;
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
            .get_work_validation_status_from_redis(FILE_SHA)
            .await?;
        assert_eq!(cache_status, Some(ValidationResult::Accept));

        let plan_3 = validator.build_validation_plan(work_keys.clone()).await?;
        assert_eq!(plan_3.group_trigger_tasks.len(), 0);
        assert_eq!(plan_3.group_status_check_tasks.len(), 0);
        let metrics_2 = export_metrics().unwrap();
        assert!(metrics_2
            .contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 0"));
        assert!(metrics_2.contains("toploc_config_name=\"Qwen/Qwen0.6\""));
        assert!(metrics_2.contains(&format!("validator_group_work_units_check_total{{group_id=\"{}\",pool_id=\"0\",result=\"match\",toploc_config_name=\"Qwen/Qwen0.6\",validator_id=\"0\"}} 1", GROUP_ID)));

        Ok(())
    }

    #[tokio::test]
    async fn test_group_e2e_work_unit_mismatch() -> Result<(), Error> {
        let mut server = Server::new_async().await;
        let (store, contracts) = setup_test_env()?;

        let config = ToplocConfig {
            server_url: server.url(),
            file_prefix_filter: Some("Qwen/Qwen0.6".to_string()),
            ..Default::default()
        };

        const HONEST_NODE_ADDRESS: &str = "0x0000000000000000000000000000000000000001";
        const HONEST_FILE_SHA: &str =
            "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641";
        const EXCESSIVE_FILE_SHA: &str =
            "88e4672c19e5a10bff2e23d223f8bfc38ae1425feaa18db9480e631a4fd98edf";
        const EXCESSIVE_NODE_ADDRESS: &str = "0x0000000000000000000000000000000000000002";
        const GROUP_ID: &str = "3456714426841564";

        let mock_storage = MockStorageProvider::new();
        mock_storage.add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-0.parquet", GROUP_ID),
            "file1",
        );
        mock_storage.add_file(
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-1.parquet", GROUP_ID),
            "file2",
        );
        mock_storage.add_mapping_file(
            HONEST_FILE_SHA,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-0.parquet", GROUP_ID),
        );
        mock_storage.add_mapping_file(
            EXCESSIVE_FILE_SHA,
            &format!("Qwen/Qwen0.6/dataset/samplingn-{}-2-0-1.parquet", GROUP_ID),
        );
        server
            .mock(
                "POST",
                format!("/validategroup/dataset/samplingn-{}-2-0.parquet", GROUP_ID).as_str(),
            )
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "file_shas": [HONEST_FILE_SHA, EXCESSIVE_FILE_SHA],
                "group_id": GROUP_ID,
                "file_number": 0,
                "group_size": 2
            })))
            .with_status(200)
            .with_body(r#"ok"#)
            .create();
        server
            .mock(
                "GET",
                format!("/statusgroup/dataset/samplingn-{}-2-0.parquet", GROUP_ID).as_str(),
            )
            .with_status(200)
            .with_body(r#"{"status": "accept", "input_flops": 1, "output_flops": 2000}"#)
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
            0, // incomplete_group_grace_period_minutes (disabled)
            Some(metrics_context),
        );

        let work_keys: Vec<String> =
            vec![HONEST_FILE_SHA.to_string(), EXCESSIVE_FILE_SHA.to_string()];

        const EXPECTED_WORK_UNITS: u64 = 1000;
        const EXCESSIVE_WORK_UNITS: u64 = 1500;
        let work_info_1 = WorkInfo {
            node_id: Address::from_str(HONEST_NODE_ADDRESS).unwrap(),
            work_units: U256::from(EXPECTED_WORK_UNITS),
            ..Default::default()
        };
        let work_info_2 = WorkInfo {
            node_id: Address::from_str(EXCESSIVE_NODE_ADDRESS).unwrap(),
            work_units: U256::from(EXCESSIVE_WORK_UNITS),
            ..Default::default()
        };

        validator
            .update_work_info_in_redis(HONEST_FILE_SHA, &work_info_1)
            .await?;
        validator
            .update_work_info_in_redis(EXCESSIVE_FILE_SHA, &work_info_2)
            .await?;

        let plan = validator.build_validation_plan(work_keys.clone()).await?;
        assert_eq!(plan.group_trigger_tasks.len(), 1);
        assert_eq!(plan.group_trigger_tasks[0].group_id, GROUP_ID);
        let metrics_0 = export_metrics().unwrap();
        assert!(metrics_0
            .contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 2"));

        let group = validator.get_group(HONEST_FILE_SHA).await?;
        assert!(group.is_some());
        let group = group.unwrap();
        assert_eq!(group.group_id, GROUP_ID);
        assert_eq!(group.group_size, 2);
        assert_eq!(group.file_number, 0);

        let result = validator.process_group_task(group).await;
        assert!(result.is_ok());

        let cache_status_1 = validator
            .get_work_validation_status_from_redis(HONEST_FILE_SHA)
            .await?;
        assert_eq!(cache_status_1, Some(ValidationResult::Unknown));
        let cache_status_2 = validator
            .get_work_validation_status_from_redis(EXCESSIVE_FILE_SHA)
            .await?;
        assert_eq!(cache_status_2, Some(ValidationResult::Unknown));

        let plan_2 = validator.build_validation_plan(work_keys.clone()).await?;
        assert_eq!(plan_2.group_trigger_tasks.len(), 0);
        assert_eq!(plan_2.group_status_check_tasks.len(), 1);

        let metrics = export_metrics().unwrap();
        assert!(
            metrics.contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 2")
        );

        let result = validator
            .process_group_status_check(plan_2.group_status_check_tasks[0].clone())
            .await;
        assert!(result.is_ok());

        let cache_status_1 = validator
            .get_work_validation_status_from_redis(HONEST_FILE_SHA)
            .await?;
        assert_eq!(cache_status_1, Some(ValidationResult::Accept));

        let cache_status_2 = validator
            .get_work_validation_status_from_redis(EXCESSIVE_FILE_SHA)
            .await?;
        assert_eq!(cache_status_2, Some(ValidationResult::Reject));

        let plan_3 = validator.build_validation_plan(work_keys.clone()).await?;
        assert_eq!(plan_3.group_trigger_tasks.len(), 0);
        assert_eq!(plan_3.group_status_check_tasks.len(), 0);
        let metrics_2 = export_metrics().unwrap();
        assert!(metrics_2.contains(&format!("validator_group_validations_total{{group_id=\"{}\",pool_id=\"0\",result=\"accept\",toploc_config_name=\"Qwen/Qwen0.6\",validator_id=\"0\"}} 1", GROUP_ID)));
        assert!(metrics_2
            .contains("validator_work_keys_to_process{pool_id=\"0\",validator_id=\"0\"} 0"));
        assert!(metrics_2.contains("toploc_config_name=\"Qwen/Qwen0.6\""));
        assert!(metrics_2.contains(&format!("validator_group_work_units_check_total{{group_id=\"{}\",pool_id=\"0\",result=\"mismatch\",toploc_config_name=\"Qwen/Qwen0.6\",validator_id=\"0\"}} 1", GROUP_ID)));

        Ok(())
    }

    #[tokio::test]
    async fn test_process_group_status_check_reject() -> Result<(), Error> {
        let mut server = Server::new_async().await;

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
            0, // incomplete_group_grace_period_minutes (disabled)
            None,
        );

        let group = validator
            .get_group("c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641")
            .await?;
        let group = group.unwrap();

        let result = validator.process_group_status_check(group).await;
        assert!(result.is_ok());

        let work_info = validator
            .get_work_info_from_redis(
                "c257e3d3fe866a00df1285f8bbbe601fed6b85229d983bbbb75e19a068346641",
            )
            .await?;
        assert!(work_info.is_none(), "Work should be invalidated");

        Ok(())
    }

    #[tokio::test]
    async fn test_incomplete_group_recovery() -> Result<(), Error> {
        let (store, contracts) = setup_test_env()?;
        let mock_storage = MockStorageProvider::new();

        // Create an incomplete group with only 1 of 2 expected files
        const GROUP_ID: &str = "1234567890123456";
        const FILE_SHA_1: &str = "a1b2c3d4e5f6789012345678901234567890123456789012345678901234abcd";
        const FILE_SHA_2: &str = "b2c3d4e5f6789012345678901234567890123456789012345678901234bcde";

        mock_storage.add_file(
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
            "file1",
        );
        mock_storage.add_file(
            &format!("TestModel/dataset/test-{}-2-0-1.parquet", GROUP_ID),
            "file2",
        );
        mock_storage.add_mapping_file(
            FILE_SHA_1,
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
        );
        mock_storage.add_mapping_file(
            FILE_SHA_2,
            &format!("TestModel/dataset/test-{}-2-0-1.parquet", GROUP_ID),
        );

        let storage_provider = Arc::new(mock_storage);

        // Create validator with 1 minute grace period
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
            true, // disable_chain_invalidation for testing
            1,    // 1 minute grace period
            None,
        );

        // Add work info for only the first file (making the group incomplete)
        let work_info = WorkInfo {
            node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ..Default::default()
        };
        validator
            .update_work_info_in_redis(FILE_SHA_1, &work_info)
            .await?;

        // Try to get the group - should be None (incomplete) and should start tracking
        let group = validator.get_group(FILE_SHA_1).await?;
        assert!(group.is_none(), "Group should be incomplete");

        // Check that the incomplete group is being tracked
        let group_key = format!("group:{}:2:0", GROUP_ID);
        let is_tracked = validator
            .is_group_being_tracked_as_incomplete(&group_key)
            .await?;
        assert!(is_tracked, "Group should be tracked as incomplete");

        // Simulate the group becoming complete by adding the second file
        validator
            .update_work_info_in_redis(FILE_SHA_2, &work_info)
            .await?;

        // Now the group should be complete (check from either file)
        let group = validator.get_group(FILE_SHA_2).await?;
        assert!(group.is_some(), "Group should now be complete");
        let group = group.unwrap();
        assert_eq!(group.sorted_work_keys.len(), 2);

        // Should also work when checking from the first file
        let group_from_first = validator.get_group(FILE_SHA_1).await?;
        assert!(
            group_from_first.is_some(),
            "Group should also be complete when checked from first file"
        );

        // The incomplete group tracking should be removed
        let is_still_tracked = validator
            .is_group_being_tracked_as_incomplete(&group_key)
            .await?;
        assert!(
            !is_still_tracked,
            "Group should no longer be tracked as incomplete"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_expired_incomplete_group_soft_invalidation() -> Result<(), Error> {
        let (store, contracts) = setup_test_env()?;
        let mock_storage = MockStorageProvider::new();

        // Create an incomplete group
        const GROUP_ID: &str = "9876543210987654";
        const FILE_SHA_1: &str = "c1d2e3f4567890123456789012345678901234567890123456789012345cdef";

        mock_storage.add_file(
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
            "file1",
        );
        mock_storage.add_mapping_file(
            FILE_SHA_1,
            &format!("TestModel/dataset/test-{}-2-0-0.parquet", GROUP_ID),
        );

        let storage_provider = Arc::new(mock_storage);

        // Create validator with very short grace period for testing
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
            true, // disable_chain_invalidation for testing
            1,    // 1 minute grace period (but we'll simulate expiry)
            Some(MetricsContext::new("0".to_string(), Some("0".to_string()))),
        );

        // Add work info for only the first file (making the group incomplete)
        let work_info = WorkInfo {
            node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ..Default::default()
        };
        validator
            .update_work_info_in_redis(FILE_SHA_1, &work_info)
            .await?;

        // Try to get the group - should be None (incomplete) and should start tracking
        let group = validator.get_group(FILE_SHA_1).await?;
        assert!(group.is_none(), "Group should be incomplete");

        // Manually expire the incomplete group tracking by removing it and simulating expiry
        // In a real test, you would wait for the actual expiry, but for testing we simulate it
        let group_key = format!("group:{}:2:0", GROUP_ID);
        validator.track_incomplete_group(&group_key).await?;

        // Process groups past grace period (this would normally find groups past deadline)
        // Since we can't easily simulate time passage in tests, we'll test the method exists

        let tracking = validator
            .is_group_being_tracked_as_incomplete(&group_key)
            .await?;
        assert!(tracking, "Group should still be tracked as incomplete");

        // Update the deadline to simulate time passage
        validator
            .update_incomplete_group_deadline_relative(&group_key, -2)
            .await?;

        let result = validator.process_groups_past_grace_period().await;
        assert!(
            result.is_ok(),
            "Should process groups past grace period without error"
        );
        let new_tracking = validator
            .is_group_being_tracked_as_incomplete(&group_key)
            .await?;
        assert!(
            !new_tracking,
            "Group should no longer be tracked as incomplete"
        );
        let key_status = validator
            .get_work_validation_status_from_redis(FILE_SHA_1)
            .await?;
        assert_eq!(key_status, Some(ValidationResult::IncompleteGroup));

        let metrics = export_metrics().unwrap();
        assert!(metrics.contains(&format!("validator_work_keys_soft_invalidated_total{{group_key=\"group:{}:2:0\",pool_id=\"0\",validator_id=\"0\"}} 1", GROUP_ID)));

        Ok(())
    }

    #[tokio::test]
    async fn test_incomplete_group_status_tracking() -> Result<(), Error> {
        let (store, contracts) = setup_test_env()?;
        let mock_storage = MockStorageProvider::new();

        // Create an incomplete group scenario
        const GROUP_ID: &str = "1111111111111111";
        const FILE_SHA_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";

        mock_storage.add_file(
            &format!("TestModel/dataset/test-{}-3-0-0.parquet", GROUP_ID),
            "file1",
        );
        mock_storage.add_mapping_file(
            FILE_SHA_1,
            &format!("TestModel/dataset/test-{}-3-0-0.parquet", GROUP_ID),
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
            true, // disable_chain_invalidation for testing
            1,    // 1 minute grace period
            None,
        );

        // Add work info for only 1 of 3 expected files
        let work_info = WorkInfo {
            node_id: Address::from_str("0x0000000000000000000000000000000000000000").unwrap(),
            ..Default::default()
        };
        validator
            .update_work_info_in_redis(FILE_SHA_1, &work_info)
            .await?;

        // Trigger incomplete group tracking
        let group = validator.get_group(FILE_SHA_1).await?;
        assert!(group.is_none(), "Group should be incomplete");

        // Manually process groups past grace period to simulate what would happen
        // after the grace period expires (we simulate this since we can't wait in tests)
        let group_key = format!("group:{}:3:0", GROUP_ID);

        // Manually add the group to tracking and then process it
        validator.track_incomplete_group(&group_key).await?;

        // Get work keys and manually soft invalidate to simulate expired processing
        let work_keys = validator.get_group_work_keys_from_redis(&group_key).await?;
        assert_eq!(work_keys.len(), 1);

        // Soft invalidate and set status to IncompleteGroup
        validator.soft_invalidate_work(FILE_SHA_1).await?;
        validator
            .update_work_validation_status(FILE_SHA_1, &ValidationResult::IncompleteGroup)
            .await?;

        // Verify the status is set correctly
        let status = validator
            .get_work_validation_status_from_redis(FILE_SHA_1)
            .await?;
        assert_eq!(
            status,
            Some(ValidationResult::IncompleteGroup),
            "Work should be marked as IncompleteGroup"
        );

        // Verify that IncompleteGroup status is treated as "already processed"
        // by checking it's not included in validation plans
        let validation_plan = validator
            .build_validation_plan(vec![FILE_SHA_1.to_string()])
            .await?;
        assert_eq!(validation_plan.single_trigger_tasks.len(), 0);
        assert_eq!(validation_plan.group_trigger_tasks.len(), 0);
        assert_eq!(validation_plan.status_check_tasks.len(), 0);
        assert_eq!(validation_plan.group_status_check_tasks.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_filename_resolution_retries_and_soft_invalidation() -> Result<(), Error> {
        let (store, contracts) = setup_test_env()?;
        let mock_storage = MockStorageProvider::new();
        let storage_provider = Arc::new(mock_storage);

        const WORK_KEY: &str = "1234567890123456789012345678901234567890123456789012345678901234";

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
            true, // disable_chain_invalidation for testing
            1,    // 1 minute grace period
            None,
        );

        // Try 59 times to reach just before max attempts (60)
        for _ in 0..59 {
            let result = validator.get_file_name_for_work_key(WORK_KEY).await;
            assert!(result.is_err());
            assert!(matches!(
                result.unwrap_err(),
                ProcessWorkKeyError::FileNameResolutionError(_)
            ));
        }

        // 60th attempt should trigger soft invalidation and return MaxAttemptsReached
        let result = validator.get_file_name_for_work_key(WORK_KEY).await;
        assert!(matches!(
            result.unwrap_err(),
            ProcessWorkKeyError::MaxAttemptsReached(_)
        ));

        // Verify work was marked as invalidated
        let status = validator
            .get_work_validation_status_from_redis(WORK_KEY)
            .await?;
        assert!(status.is_some());

        Ok(())
    }
}
