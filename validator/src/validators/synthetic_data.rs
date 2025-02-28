use alloy::primitives::U256;
use anyhow::{Context, Result};
use log::{error, info};

use crate::validators::Validator;
use shared::web3::contracts::implementations::work_validators::synthetic_data_validator::SyntheticDataWorkValidator;

pub struct SyntheticDataValidator {
    pool_id: U256,
    validator: SyntheticDataWorkValidator,
    last_validation_timestamp: U256,
}

impl Validator for SyntheticDataValidator {
    type Error = anyhow::Error;

    fn name(&self) -> &str {
        "Synthetic Data Validator"
    }
}

impl SyntheticDataValidator {
    pub fn new(pool_id_str: String, validator: SyntheticDataWorkValidator) -> Self {
        let pool_id = pool_id_str.parse::<U256>().expect("Invalid pool ID");

        Self {
            pool_id,
            validator,
            last_validation_timestamp: U256::from(0),
        }
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

        Ok(())
    }
}
