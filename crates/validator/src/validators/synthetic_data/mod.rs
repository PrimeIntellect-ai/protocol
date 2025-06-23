use crate::metrics::MetricsContext;
use crate::store::redis::RedisStore;
use crate::validators::synthetic_data::types::{InvalidationType, RejectionInfo};
use alloy::primitives::U256;
use anyhow::{Context as _, Error, Result};
use chrono;
use futures::future;
use log::{debug, warn};
use log::{error, info};
use redis::AsyncCommands;
use serde_json;
use shared::utils::StorageProvider;
use shared::web3::contracts::implementations::prime_network_contract::PrimeNetworkContract;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::{
    SyntheticDataWorkValidator, WorkInfo,
};
use shared::web3::wallet::WalletProvider;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
pub mod chain_operations;
#[cfg(test)]
mod tests;
pub mod toploc;
pub mod types;

use toploc::{GroupValidationResult, Toploc, ToplocConfig};
use types::{
    GroupInformation, ProcessWorkKeyError, ToplocGroup, ValidationPlan, ValidationResult,
    WorkValidationInfo,
};
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

    /// Invalidation type for toploc validation failures
    toploc_invalidation_type: InvalidationType,

    /// Invalidation type for work unit mismatch failures
    work_unit_invalidation_type: InvalidationType,

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
        toploc_invalidation_type: InvalidationType,
        work_unit_invalidation_type: InvalidationType,
        metrics: Option<MetricsContext>,
    ) -> Self {
        let pool_id = pool_id_str.parse::<U256>().expect("Invalid pool ID");

        let mut toploc = Vec::new();
        for config in toploc_configs {
            toploc.push(Toploc::new(config, metrics.clone()));
        }

        info!("Toploc invalidation type: {:?}", toploc_invalidation_type);
        info!(
            "Work unit invalidation type: {:?}",
            work_unit_invalidation_type
        );

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
            toploc_invalidation_type,
            work_unit_invalidation_type,
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
        self.update_work_validation_info(
            work_key,
            &WorkValidationInfo {
                status: status.clone(),
                reason: None,
            },
        )
        .await
    }

    async fn update_work_validation_info(
        &self,
        work_key: &str,
        validation_info: &WorkValidationInfo,
    ) -> Result<(), Error> {
        let expiry = match validation_info.status {
            // Must switch to pending within 60 seconds otherwise we resubmit it
            ValidationResult::Unknown => self.unknown_status_expiry_seconds,
            _ => 0,
        };
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let key = self.get_key_for_work_key(work_key);
        let validation_data = serde_json::to_string(&validation_info)?;

        if expiry > 0 {
            let _: () = con
                .set_options(
                    &key,
                    &validation_data,
                    redis::SetOptions::default().with_expiration(redis::SetExpiry::EX(expiry)),
                )
                .await
                .map_err(|e| Error::msg(format!("Failed to set work validation status: {}", e)))?;
        } else {
            let _: () = con
                .set(&key, &validation_data)
                .await
                .map_err(|e| Error::msg(format!("Failed to set work validation status: {}", e)))?;
        }

        // Manage rejection tracking for efficient querying
        self.update_rejection_tracking(&mut con, work_key, validation_info)
            .await?;

        Ok(())
    }

    async fn update_rejection_tracking(
        &self,
        con: &mut redis::aio::MultiplexedConnection,
        work_key: &str,
        validation_info: &WorkValidationInfo,
    ) -> Result<(), Error> {
        let rejection_set_key = "work_rejections";
        let rejection_data_key = format!("work_rejection_data:{}", work_key);
        let is_rejected = validation_info.status == ValidationResult::Reject;

        if is_rejected {
            // Add to rejections set with current timestamp
            let timestamp = chrono::Utc::now().timestamp();
            let _: () = con
                .zadd(rejection_set_key, work_key, timestamp)
                .await
                .map_err(|e| Error::msg(format!("Failed to add to rejections set: {}", e)))?;

            // Store rejection details if reason exists
            if let Some(reason) = &validation_info.reason {
                let rejection_detail = serde_json::json!({
                    "reason": reason,
                    "timestamp": timestamp
                });
                let _: () = con
                    .set(&rejection_data_key, rejection_detail.to_string())
                    .await
                    .map_err(|e| Error::msg(format!("Failed to set rejection data: {}", e)))?;
            }
        }

        Ok(())
    }

    #[cfg(test)]
    async fn get_work_validation_status_from_redis(
        &self,
        work_key: &str,
    ) -> Result<Option<ValidationResult>, Error> {
        let validation_info = self.get_work_validation_info_from_redis(work_key).await?;
        Ok(validation_info.map(|info| info.status))
    }
    #[cfg(test)]
    async fn get_work_validation_info_from_redis(
        &self,
        work_key: &str,
    ) -> Result<Option<WorkValidationInfo>, Error> {
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;
        let key = self.get_key_for_work_key(work_key);
        let data: Option<String> = con
            .get(key)
            .await
            .map_err(|e| Error::msg(format!("Failed to get work validation status: {}", e)))?;

        match data {
            Some(data) => {
                // Try to parse as WorkValidationInfo first (new format)
                if let Ok(validation_info) = serde_json::from_str::<WorkValidationInfo>(&data) {
                    Ok(Some(validation_info))
                } else {
                    // Fall back to old format (just ValidationResult)
                    match serde_json::from_str::<ValidationResult>(&data) {
                        Ok(status) => Ok(Some(WorkValidationInfo {
                            status,
                            reason: None,
                        })),
                        Err(e) => Err(Error::msg(format!(
                            "Failed to parse work validation data: {}",
                            e
                        ))),
                    }
                }
            }
            None => Ok(None),
        }
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
                    | ValidationResult::IncompleteGroup
                    | ValidationResult::FileNameResolutionFailed,
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
                if let Ok(validation_info) = serde_json::from_str::<WorkValidationInfo>(status_str)
                {
                    status_map.insert(work_key.clone(), validation_info.status);
                } else {
                    // Fall back to old format (just ValidationResult)
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

    pub async fn get_all_rejections(&self) -> Result<Vec<RejectionInfo>, Error> {
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        let rejection_set_key = "work_rejections";

        let work_keys_with_scores: Vec<(String, f64)> = con
            .zrange_withscores(rejection_set_key, 0, -1)
            .await
            .map_err(|e| Error::msg(format!("Failed to get rejections from set: {}", e)))?;

        if work_keys_with_scores.is_empty() {
            return Ok(Vec::new());
        }

        let work_keys: Vec<&String> = work_keys_with_scores.iter().map(|(key, _)| key).collect();

        // Batch fetch rejection details using MGET for efficiency
        let rejection_data_keys: Vec<String> = work_keys
            .iter()
            .map(|key| format!("work_rejection_data:{}", key))
            .collect();

        let rejection_details: Vec<Option<String>> = con
            .mget(&rejection_data_keys)
            .await
            .map_err(|e| Error::msg(format!("Failed to batch get rejection details: {}", e)))?;

        let mut rejections = Vec::new();

        for (i, (work_key, score_timestamp)) in work_keys_with_scores.into_iter().enumerate() {
            let mut reason = None;
            let mut timestamp = Some(score_timestamp as i64);

            if let Some(Some(detail_json)) = rejection_details.get(i) {
                if let Ok(detail) = serde_json::from_str::<serde_json::Value>(detail_json) {
                    reason = detail
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string());
                    if let Some(stored_timestamp) = detail.get("timestamp").and_then(|t| t.as_i64())
                    {
                        timestamp = Some(stored_timestamp);
                    }
                }
            }

            rejections.push(RejectionInfo {
                work_key,
                reason,
                timestamp,
            });
        }

        Ok(rejections)
    }

    pub async fn get_recent_rejections(&self, limit: isize) -> Result<Vec<RejectionInfo>, Error> {
        let mut con = self
            .redis_store
            .client
            .get_multiplexed_async_connection()
            .await?;

        let rejection_set_key = "work_rejections";

        // Get most recent rejections (sorted by timestamp in descending order)
        let work_keys: Vec<String> = con
            .zrevrange(rejection_set_key, 0, limit - 1)
            .await
            .map_err(|e| Error::msg(format!("Failed to get recent rejections: {}", e)))?;

        if work_keys.is_empty() {
            return Ok(Vec::new());
        }

        // Batch fetch rejection details
        let rejection_data_keys: Vec<String> = work_keys
            .iter()
            .map(|key| format!("work_rejection_data:{}", key))
            .collect();

        let rejection_details: Vec<Option<String>> = con
            .mget(&rejection_data_keys)
            .await
            .map_err(|e| Error::msg(format!("Failed to batch get rejection details: {}", e)))?;

        let mut rejections = Vec::new();

        for (i, work_key) in work_keys.into_iter().enumerate() {
            let mut reason = None;
            let mut timestamp = None;

            if let Some(Some(detail_json)) = rejection_details.get(i) {
                if let Ok(detail) = serde_json::from_str::<serde_json::Value>(detail_json) {
                    reason = detail
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string());
                    timestamp = detail.get("timestamp").and_then(|t| t.as_i64());
                }
            }

            rejections.push(RejectionInfo {
                work_key,
                reason,
                timestamp,
            });
        }

        Ok(rejections)
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

        let mut toploc_nodes_to_invalidate = Vec::new();
        let mut nodes_with_wrong_work_unit_claims = Vec::new();

        if status.status == ValidationResult::Reject {
            let rejected_nodes = self.handle_group_toploc_rejection(&group, &status).await?;
            toploc_nodes_to_invalidate.extend(rejected_nodes);
        } else if status.status == ValidationResult::Accept {
            let wrong_claim_nodes = self
                .handle_group_toploc_acceptance(
                    &group,
                    &status,
                    total_claimed_units,
                    &node_work_units,
                    &toploc_config_name,
                )
                .await?;
            nodes_with_wrong_work_unit_claims.extend(wrong_claim_nodes);
        }

        for work_key in &group.sorted_work_keys {
            if let Some(work_info) = self.get_work_info_from_redis(work_key).await? {
                let node_id_str = work_info.node_id.to_string();

                if toploc_nodes_to_invalidate.contains(&node_id_str) {
                    self.invalidate_according_to_invalidation_type(
                        work_key,
                        self.toploc_invalidation_type,
                    )
                    .await?;

                    self.update_work_validation_info(
                        work_key,
                        &WorkValidationInfo {
                            status: ValidationResult::Reject,
                            reason: status.reason.clone(),
                        },
                    )
                    .await?;
                } else if nodes_with_wrong_work_unit_claims.contains(&node_id_str) {
                    self.invalidate_according_to_invalidation_type(
                        work_key,
                        self.work_unit_invalidation_type,
                    )
                    .await?;

                    self.update_work_validation_info(
                        work_key,
                        &WorkValidationInfo {
                            status: ValidationResult::Reject,
                            reason: Some("Incorrect work unit claims".to_string()),
                        },
                    )
                    .await?;
                } else {
                    self.update_work_validation_info(
                        work_key,
                        &WorkValidationInfo {
                            status: status.status.clone(),
                            reason: status.reason.clone(),
                        },
                    )
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
                    .process_single_workkey_status(&work_key)
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

    async fn process_single_workkey_status(
        &self,
        work_key: &str,
    ) -> Result<(), ProcessWorkKeyError> {
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
                if let Err(e) = self
                    .invalidate_according_to_invalidation_type(
                        work_key,
                        self.toploc_invalidation_type,
                    )
                    .await
                {
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
