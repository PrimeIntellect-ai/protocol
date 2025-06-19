use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::rpc::types::Log;
use async_trait::async_trait;
use log::{error, info, warn};
use std::sync::Arc;

use crate::blockchain::event_handlers::EventHandler;
use crate::error::OrchestratorError;
use crate::plugins::node_groups::NodeGroupsPlugin;
use shared::utils::StorageProvider;

pub struct SoftInvalidationHandler {
    name: String,
    event_signature: String,
    topic_hash: B256,
    group_manager: Arc<NodeGroupsPlugin>,
    storage_provider: Arc<dyn StorageProvider>,
}

impl SoftInvalidationHandler {
    pub fn new(
        group_manager: Arc<NodeGroupsPlugin>,
        storage_provider: Arc<dyn StorageProvider>,
    ) -> Self {
        let event_signature =
            "WorkInvalidated(uint256,address,address,bytes32,uint256)".to_string();
        let topic_hash = keccak256(event_signature.as_bytes());

        Self {
            name: "SoftInvalidationHandler".to_string(),
            event_signature,
            topic_hash,
            group_manager,
            storage_provider,
        }
    }

    fn decode_work_invalidated_event(
        &self,
        log: &Log,
    ) -> Result<WorkInvalidatedData, OrchestratorError> {
        // Verify this is the correct event
        if log.topics().first() != Some(&self.topic_hash) {
            return Err(OrchestratorError::Custom(
                "Event topic doesn't match WorkInvalidated".to_string(),
            ));
        }

        // All fields are non-indexed according to ABI, so they're all in data
        let data = log.data();
        if data.data.len() < 160 {
            // 5 * 32 bytes = 160 bytes minimum
            return Err(OrchestratorError::Custom(
                "Event data too short for WorkInvalidated".to_string(),
            ));
        }

        // Parse the data fields (each is 32 bytes)
        let pool_id = U256::from_be_slice(&data.data[0..32]);
        let provider_bytes = &data.data[32..64];
        let provider = Address::from_slice(&provider_bytes[12..32]); // Address is 20 bytes, padded to 32
        let node_id_bytes = &data.data[64..96];
        let node_id = Address::from_slice(&node_id_bytes[12..32]);
        let work_key = B256::from_slice(&data.data[96..128]);
        let work_units = U256::from_be_slice(&data.data[128..160]);

        Ok(WorkInvalidatedData {
            pool_id,
            provider,
            node_id,
            work_key,
            work_units,
        })
    }

    async fn handle_work_invalidated(
        &self,
        data: WorkInvalidatedData,
    ) -> Result<(), OrchestratorError> {
        info!(
            "Processing soft invalidation - Pool: {}, Work Key: {:?}, Node: {:?}",
            data.pool_id, data.work_key, data.node_id
        );

        // Use work_key as SHA256 for storage resolution
        let work_key_hex = format!("{:x}", data.work_key);

        // Try to resolve work key to filename
        let filename = match self
            .storage_provider
            .resolve_mapping_for_sha(&work_key_hex)
            .await
        {
            Ok(fname) => fname,
            Err(e) => {
                error!(
                    "Failed to resolve filename for work key {}: {}",
                    work_key_hex, e
                );
                return Err(OrchestratorError::Custom(format!(
                    "Failed to resolve work key {}: {}",
                    work_key_hex, e
                )));
            }
        };

        // Extract group ID from filename if it's a group file
        use crate::utils::group_parser::GroupInformation;
        let group_id = if GroupInformation::is_group_file(&filename) {
            match GroupInformation::extract_group_id_quick(&filename) {
                Some(gid) => gid,
                None => {
                    info!(
                        "Work key {} filename {} does not contain group ID, skipping group dissolution",
                        work_key_hex, filename
                    );
                    return Ok(());
                }
            }
        } else {
            info!(
                "Work key {} does not belong to a group file, skipping group dissolution",
                work_key_hex
            );
            return Ok(());
        };

        info!(
            "Found group ID {} for invalidated work, checking if group exists",
            group_id
        );

        // Check if the group exists and dissolve it
        match self.group_manager.get_group_by_id(&group_id).await {
            Ok(Some(_group)) => {
                info!("Dissolving group {} due to soft invalidation", group_id);
                if let Err(e) = self.group_manager.dissolve_group(&group_id).await {
                    error!("Failed to dissolve group {}: {}", group_id, e);
                    return Err(OrchestratorError::Custom(format!(
                        "Failed to dissolve group {}: {}",
                        group_id, e
                    )));
                }
                info!(
                    "Successfully dissolved group {} due to producing invalid work",
                    group_id
                );
            }
            Ok(None) => {
                warn!(
                    "Group {} not found, may have already been dissolved",
                    group_id
                );
            }
            Err(e) => {
                error!("Error checking group existence for {}: {}", group_id, e);
                return Err(OrchestratorError::Custom(format!(
                    "Error checking group {}: {}",
                    group_id, e
                )));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl EventHandler for SoftInvalidationHandler {
    async fn handle_event(&self, log: Log) -> Result<(), OrchestratorError> {
        match self.decode_work_invalidated_event(&log) {
            Ok(data) => {
                if let Err(e) = self.handle_work_invalidated(data).await {
                    error!("Failed to handle soft invalidation: {}", e);
                    return Err(e);
                }
            }
            Err(e) => {
                warn!("Failed to decode WorkInvalidated event: {}", e);
                return Err(e);
            }
        }

        Ok(())
    }

    fn event_signature(&self) -> &str {
        &self.event_signature
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug)]
struct WorkInvalidatedData {
    pool_id: U256,
    #[allow(dead_code)]
    provider: Address,
    node_id: Address,
    work_key: B256,
    #[allow(dead_code)]
    work_units: U256,
}
